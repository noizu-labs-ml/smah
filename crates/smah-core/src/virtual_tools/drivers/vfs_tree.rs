//! VFS tree: fabricated file contents, real files under the smah cache
//! (ticket #25).
//!
//! The model fabricates a structured JSON spec — `{"files":[{"path", "content"}]}`
//! — never shell, never mounts. This lane writes the real files under
//! `~/.cache/smah/virtual-tools/<tool>/vfs/root/` itself (regenerable cache;
//! `vfs.sha256` keys the cache to the tool-file digest). No FUSE, no kernel
//! mounts: the files are ordinary files a human can open, and the driver
//! serves them over JSON-RPC fs tools.
//!
//! Containment mirrors `fs_surface`: lexical `..` reject → resolve through
//! existing-ancestor canonicalization → component-prefix check against the
//! root → final-component symlink refusal on write (T-026). A fabrication
//! spec that tries to escape is a **DirectiveViolation** (exit 3), not a
//! retry; a client fs_write that tries to escape is refused per-request and
//! audited while the server keeps serving.

use std::path::{Path, PathBuf};

use serde_json::Value as Json;

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::drivers::ChatBackend;
use crate::virtual_tools::drivers::ChatMessage;
use crate::virtual_tools::loader::ToolDef;

use super::support;

/// Fabrication caps (T-026/T-030 hygiene).
pub(crate) const MAX_FILES: usize = 200;
pub(crate) const MAX_FILE_BYTES: usize = 64 * 1024;
pub(crate) const MAX_TOTAL_BYTES: usize = 1024 * 1024;
/// Client fs_write shares the per-file cap.
pub(crate) const WRITE_BYTES_MAX: usize = MAX_FILE_BYTES;

/// One fabricated file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct FileSpec {
    pub path: String,
    pub content: String,
}

/// Spec rejection classes: shape errors retry once; escape attempts abort.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum SpecError {
    /// Retryable: wrong shape, caps, duplicate paths.
    Invalid(String),
    /// Non-retryable containment violation (`..`, absolute, symlink escape).
    Escape(String),
}

/// Validate one fabricated relative path. Returns its components.
fn validate_rel_path(path: &str) -> Result<Vec<String>, SpecError> {
    if path.starts_with('/') {
        return Err(SpecError::Escape(format!(
            "path '{path}' is absolute — fabricated paths are relative to the tool root"
        )));
    }
    if path.contains('\\') {
        return Err(SpecError::Invalid(format!(
            "path '{path}' contains a backslash — use POSIX '/' separators"
        )));
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        if component.is_empty() {
            return Err(SpecError::Invalid(format!(
                "path '{path}' has an empty component"
            )));
        }
        if component == "." || component == ".." {
            return Err(SpecError::Escape(format!(
                "path '{path}' contains a '{component}' component — fabricated paths may not escape the tool root"
            )));
        }
        let ok_len = component.len() <= 128;
        let ok_chars = component
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !ok_len || !ok_chars {
            return Err(SpecError::Invalid(format!(
                "path component '{component}' is invalid (letters/digits/'.'/'_'/'-', max 128)"
            )));
        }
        components.push(component.to_string());
    }
    Ok(components)
}

/// Parse + validate a fabricated spec. `Err(SpecError::Escape)` aborts;
/// `Err(SpecError::Invalid)` feeds back for the single retry.
pub(crate) fn parse_spec(root: &Json) -> Result<Vec<FileSpec>, SpecError> {
    let files = root.get("files").and_then(Json::as_array).ok_or_else(|| {
        SpecError::Invalid("spec must be a JSON object with a 'files' array".into())
    })?;
    if files.len() > MAX_FILES {
        return Err(SpecError::Invalid(format!(
            "spec declares {} files (max {MAX_FILES})",
            files.len()
        )));
    }
    let mut out = Vec::with_capacity(files.len());
    let mut total = 0usize;
    for file in files {
        let path = file
            .get("path")
            .and_then(Json::as_str)
            .ok_or_else(|| SpecError::Invalid("each file needs a string 'path'".into()))?;
        let content = file
            .get("content")
            .and_then(Json::as_str)
            .ok_or_else(|| SpecError::Invalid(format!("file '{path}' needs a string 'content'")))?;
        validate_rel_path(path)?;
        if content.len() > MAX_FILE_BYTES {
            return Err(SpecError::Invalid(format!(
                "file '{path}' content is {} bytes (max {MAX_FILE_BYTES})",
                content.len()
            )));
        }
        if out.iter().any(|f: &FileSpec| f.path == path) {
            return Err(SpecError::Invalid(format!(
                "duplicate fabricated path '{path}'"
            )));
        }
        total += content.len();
        if total > MAX_TOTAL_BYTES {
            return Err(SpecError::Invalid(format!(
                "spec totals {total} bytes (max {MAX_TOTAL_BYTES})"
            )));
        }
        out.push(FileSpec {
            path: path.to_string(),
            content: content.to_string(),
        });
    }
    Ok(out)
}

/// Lexical client-path validation for the fs tools: relative, no `..`.
fn validate_client_path(path: &str) -> Result<Option<Vec<String>>, String> {
    if path.is_empty() || path == "." {
        return Ok(None); // the root itself
    }
    if path.starts_with('/') {
        return Err(format!(
            "path '{path}' is absolute — fs paths are relative to the tool root"
        ));
    }
    let mut components = Vec::new();
    for component in path.split('/') {
        if component.is_empty() || component == "." {
            continue;
        }
        if component == ".." {
            return Err(format!(
                "path '{path}' escapes the tool root — '..' is not allowed"
            ));
        }
        components.push(component.to_string());
    }
    Ok(Some(components))
}

/// Canonical ancestor resolution (fs_surface model): canonicalize the nearest
/// existing ancestor, re-join the not-yet-existing tail.
fn resolve_candidate(root_canonical: &Path, candidate: &Path) -> PathBuf {
    let mut probe = candidate.to_path_buf();
    let mut rejoin: Vec<std::ffi::OsString> = Vec::new();
    loop {
        match std::fs::canonicalize(&probe) {
            Ok(existing) => {
                let mut out = existing;
                for part in rejoin.iter().rev() {
                    out = out.join(part);
                }
                return out;
            }
            Err(_) => {
                match probe.file_name() {
                    Some(name) => rejoin.push(name.to_os_string()),
                    None => return root_canonical.to_path_buf(),
                }
                if !probe.pop() {
                    return root_canonical.to_path_buf();
                }
            }
        }
    }
}

/// Component-prefix containment check (both paths canonicalized).
fn is_within(root: &Path, candidate: &Path) -> bool {
    candidate.strip_prefix(root).is_ok()
}

/// A resolved, contained client path.
pub(crate) struct ContainedPath {
    /// Lexical path under the root (may not exist yet).
    pub candidate: PathBuf,
    /// Canonical ancestor-resolved path used for the containment check.
    pub resolved: PathBuf,
}

/// Resolve + contain a client path against the tree root.
pub(crate) fn contain(root: &Path, client_path: &str) -> Result<ContainedPath, String> {
    let root_canonical = std::fs::canonicalize(root).map_err(|err| {
        format!(
            "tool root unreadable: {}",
            support::one_line(&err.to_string())
        )
    })?;
    let components = validate_client_path(client_path)?;
    let candidate = match &components {
        None => root_canonical.clone(),
        Some(components) => {
            let mut path = root_canonical.clone();
            for component in components {
                path = path.join(component);
            }
            path
        }
    };
    let resolved = resolve_candidate(&root_canonical, &candidate);
    if !is_within(&root_canonical, &resolved) {
        return Err(format!(
            "path '{client_path}' resolves outside the tool root — refused"
        ));
    }
    Ok(ContainedPath {
        candidate,
        resolved,
    })
}

/// Write a client-supplied file inside the tree (contained, capped,
/// final-component symlink refusal). Returns the byte count written.
pub(crate) fn write_client_file(root: &Path, path: &str, content: &str) -> Result<usize, String> {
    if content.len() > WRITE_BYTES_MAX {
        return Err(format!(
            "fs_write content is {} bytes (max {WRITE_BYTES_MAX})",
            content.len()
        ));
    }
    let contained = contain(root, path)?;
    if let Ok(meta) = std::fs::symlink_metadata(&contained.candidate) {
        if meta.file_type().is_symlink() {
            return Err(format!(
                "path '{path}' is a symlink — symlink writes are refused"
            ));
        }
    }
    let parent = contained
        .candidate
        .parent()
        .ok_or_else(|| format!("path '{path}' has no parent"))?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("mkdir: {}", support::one_line(&err.to_string())))?;
    std::fs::write(&contained.candidate, content)
        .map_err(|err| format!("write: {}", support::one_line(&err.to_string())))?;
    Ok(content.len())
}

/// `fs_list`: sorted relative paths under the requested directory.
pub(crate) fn list(root: &Path, path: &str) -> Result<String, String> {
    let contained = contain(root, path)?;
    let mut entries: Vec<String> = Vec::new();
    let prefix_len = std::fs::canonicalize(root)
        .map_err(|err| {
            format!(
                "tool root unreadable: {}",
                support::one_line(&err.to_string())
            )
        })?
        .as_os_str()
        .len();
    walk(&contained.resolved, prefix_len, &mut entries)?;
    entries.sort();
    Ok(serde_json::to_string(&entries).unwrap_or_else(|_| "[]".to_string()))
}

fn walk(dir: &Path, prefix_len: usize, out: &mut Vec<String>) -> Result<(), String> {
    let entries = std::fs::read_dir(dir)
        .map_err(|err| format!("list: {}", support::one_line(&err.to_string())))?;
    for entry in entries {
        let entry =
            entry.map_err(|err| format!("list: {}", support::one_line(&err.to_string())))?;
        let path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|err| format!("list: {}", support::one_line(&err.to_string())))?;
        let rel = path
            .to_string_lossy()
            .get(prefix_len..)
            .unwrap_or("")
            .trim_start_matches('/')
            .to_string();
        if file_type.is_dir() {
            out.push(format!("{rel}/"));
            walk(&path, prefix_len, out)?;
        } else {
            out.push(rel);
        }
    }
    Ok(())
}

/// `fs_read`: file contents (must be valid UTF-8; fabricated files are).
pub(crate) fn read(root: &Path, path: &str) -> Result<String, String> {
    let contained = contain(root, path)?;
    let meta = std::fs::symlink_metadata(&contained.candidate)
        .map_err(|_| format!("no such file: {path}"))?;
    if meta.file_type().is_symlink() {
        return Err(format!(
            "path '{path}' is a symlink — symlink reads are refused"
        ));
    }
    std::fs::read_to_string(&contained.candidate)
        .map_err(|err| format!("read: {}", support::one_line(&err.to_string())))
}

/// `fs_stat`: a small JSON descriptor.
pub(crate) fn stat(root: &Path, path: &str) -> Result<String, String> {
    let contained = contain(root, path)?;
    let value = match std::fs::symlink_metadata(&contained.candidate) {
        Ok(meta) if meta.file_type().is_symlink() => serde_json::json!({
            "path": path, "kind": "symlink", "bytes": null,
        }),
        Ok(meta) if meta.is_dir() => serde_json::json!({
            "path": path, "kind": "dir", "bytes": null,
        }),
        Ok(meta) => serde_json::json!({
            "path": path, "kind": "file", "bytes": meta.len(),
        }),
        Err(_) => serde_json::json!({
            "path": path, "kind": "missing", "bytes": null,
        }),
    };
    Ok(value.to_string())
}

/// Write fabricated files into the tree. `wipe` clears the root first
/// (full regeneration); `false` appends to an existing tree (fs_generate).
pub(crate) fn apply_files(root: &Path, files: &[FileSpec], wipe: bool) -> Result<usize, String> {
    if wipe {
        let _ = std::fs::remove_dir_all(root);
    }
    std::fs::create_dir_all(root)
        .map_err(|err| format!("tool root: {}", support::one_line(&err.to_string())))?;
    for file in files {
        write_client_file(root, &file.path, &file.content)
            .map_err(|err| format!("fabricated file '{}': {err}", file.path))?;
    }
    Ok(files.len())
}

/// Manifest receipt path: `<vfs leaf>/manifest.json`.
pub(crate) fn manifest_path(dir: &Path) -> PathBuf {
    dir.join("manifest.json")
}

/// Write the fabrication receipt (relative paths + byte counts only).
fn write_manifest(dir: &Path, files: &[FileSpec]) -> Result<(), String> {
    let entries: Vec<Json> = files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "bytes": f.content.len()}))
        .collect();
    let text = serde_json::to_string_pretty(&serde_json::json!({"files": entries}))
        .map_err(|err| format!("manifest serialize: {err}"))?;
    std::fs::write(manifest_path(dir), text)
        .map_err(|err| format!("manifest write: {}", support::one_line(&err.to_string())))
}

pub(crate) const FABRICATE_SYSTEM: &str = concat!(
    "You fabricate the file tree of a VIRTUAL filesystem for a smah virtual tool.\n",
    "Output ONLY one JSON object, no markdown fences, no prose:\n",
    "{\"files\":[{\"path\":\"relative/dir/file.txt\",\"content\":\"file contents\"}]}\n",
    "Paths are POSIX-style and RELATIVE to the tool root; no absolute paths, no '..' components.\n",
    "Max 200 files, 64 KiB per file, 1 MiB total. Make the contents coherent with the persona\n",
    "(realistic records, configs, notes — plain text). Do not output anything else."
);

fn fabricate_user_prompt(tool: &ToolDef) -> String {
    let mut out = String::new();
    out.push_str("VIRTUAL FILESYSTEM PERSONA:\n");
    out.push_str(&tool.body);
    if !tool.constraints.is_empty() {
        out.push_str("\n\nHard constraints:\n");
        for constraint in &tool.constraints {
            out.push_str(&format!("- {constraint}\n"));
        }
    }
    out.push_str("\nContract artifact (interface):\n");
    match &tool.interface {
        Some(interface) if interface.len() <= MAX_TOTAL_BYTES => out.push_str(interface),
        Some(_) => out.push_str(
            "(interface artifact over the size cap — fabricate a coherent small tree fitting the persona)",
        ),
        None => out.push_str(
            "(none declared — fabricate a coherent small file tree fitting the persona)",
        ),
    }
    out.push_str("\n\nFabricate the file tree now. Output only the JSON object.");
    out
}

/// Extract the JSON object from a model reply (tolerates fences and prose).
pub(crate) fn extract_json_object(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')? + 1;
    if end <= start || start > MAX_TOTAL_BYTES {
        return None;
    }
    Some(&trimmed[start..end])
}

/// One fabrication attempt: extract → parse → apply. `Err` is the retry
/// reason; an escape attempt is escalated by the caller (DirectiveViolation).
fn attempt(reply: &str, root: &Path, dir: &Path) -> Result<usize, (String, bool)> {
    let text =
        extract_json_object(reply).ok_or(("reply contained no JSON object".to_string(), false))?;
    let parsed: Json = serde_json::from_str(text).map_err(|err| {
        (
            format!(
                "spec JSON did not parse: {}",
                support::one_line(&err.to_string())
            ),
            false,
        )
    })?;
    let files = parse_spec(&parsed).map_err(|err| match err {
        SpecError::Escape(detail) => (detail, true),
        SpecError::Invalid(detail) => (detail, false),
    })?;
    let count = apply_files(root, &files, true).map_err(|e| (e, false))?;
    write_manifest(dir, &files).map_err(|e| (e, false))?;
    Ok(count)
}

/// Ensure the fabricated tree exists and matches the current tool file.
/// Returns the tree root and the model turns consumed. Two invalid specs
/// abort with [`SmahError::DirectiveLoopOverflow`] (exit 5); an escape
/// attempt aborts immediately with [`SmahError::DirectiveViolation`] (exit 3).
pub(crate) fn ensure_tree(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    model: &str,
    audit: &Audit,
    run_id: &str,
    dir: &Path,
    force: bool,
) -> Result<(PathBuf, usize), SmahError> {
    let (digest, tool_len) =
        support::sha256_file(&tool.path).map_err(|err| SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!("unreadable while keying the cache: {err}"),
        })?;
    let root = dir.join("root");
    let marker = dir.join(support::MARKER_VFS);
    if !force && root.is_dir() {
        if let Ok(stored) = std::fs::read_to_string(&marker) {
            if stored.trim() == digest {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "vfs.tree",
                    "reuse",
                    Some(tool_len as usize),
                    "allowed",
                    "",
                );
                return Ok((root, 0));
            }
        }
    }
    let mut turns = 0usize;
    let mut prompt = fabricate_user_prompt(tool);
    loop {
        if turns >= crate::virtual_tools::TURNS_MAX {
            return Err(SmahError::DirectiveLoopOverflow {
                tool: tool.name.clone(),
                turns,
            });
        }
        turns += 1;
        let reply = backend.stream_turn(
            model,
            FABRICATE_SYSTEM,
            &[ChatMessage::user(&prompt)],
            &mut |_| {},
        )?;
        match attempt(&reply, &root, dir) {
            Ok(count) => {
                std::fs::write(&marker, &digest).map_err(|err| SmahError::ToolFileInvalid {
                    path: marker.display().to_string(),
                    detail: format!("marker write: {}", support::one_line(&err.to_string())),
                })?;
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "vfs.tree",
                    "generate",
                    Some(reply.len()),
                    "allowed",
                    "",
                );
                let _ = count;
                return Ok((root, turns));
            }
            Err((reason, escape)) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "vfs.tree",
                    "generate",
                    Some(reply.len()),
                    "denied",
                    &reason,
                );
                if escape {
                    // Fabrication tried to escape containment: hard abort.
                    return Err(SmahError::DirectiveViolation {
                        tool: tool.name.clone(),
                        detail: format!(
                            "fabricated file tree attempted to escape its root: {reason}"
                        ),
                    });
                }
                if turns >= 2 {
                    return Err(SmahError::DirectiveLoopOverflow {
                        tool: tool.name.clone(),
                        turns,
                    });
                }
                prompt = format!(
                    "{}\n\nYour previous spec was rejected: {reason}\n\
                     Fix it and output only the corrected JSON object.",
                    fabricate_user_prompt(tool)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn spec(files: Json) -> Json {
        json!({"files": files})
    }

    #[test]
    fn parse_spec_accepts_valid_and_rejects_shape_errors() {
        let ok = spec(json!([
            {"path": "README.md", "content": "hello"},
            {"path": "a/b/c.json", "content": "{}"}
        ]));
        let files = parse_spec(&ok).expect("valid");
        assert_eq!(files.len(), 2);
        assert_eq!(files[1].path, "a/b/c.json");

        assert!(matches!(
            parse_spec(&json!({"nope": []})),
            Err(SpecError::Invalid(_))
        ));
        assert!(matches!(
            parse_spec(&json!({"files": [{"path": 1, "content": "x"}]})),
            Err(SpecError::Invalid(_))
        ));
        assert!(matches!(
            parse_spec(&json!({"files": [{"path": "a", "content": 3}]})),
            Err(SpecError::Invalid(_))
        ));
        let dup = spec(json!([
            {"path": "a", "content": "1"},
            {"path": "a", "content": "2"}
        ]));
        assert!(matches!(parse_spec(&dup), Err(SpecError::Invalid(_))));
        let big = spec(json!([{"path": "a", "content": "x".repeat(MAX_FILE_BYTES + 1)}]));
        assert!(matches!(parse_spec(&big), Err(SpecError::Invalid(_))));
    }

    #[test]
    fn parse_spec_escalates_escape_attempts() {
        for bad in ["../escape.txt", "/etc/passwd", "a/../../out"] {
            let spec = spec(json!([{"path": bad, "content": "x"}]));
            assert!(
                matches!(parse_spec(&spec), Err(SpecError::Escape(_))),
                "{bad}"
            );
        }
    }

    #[test]
    fn apply_files_round_trip_list_read_stat() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        let files = vec![
            FileSpec {
                path: "README.md".into(),
                content: "hello".into(),
            },
            FileSpec {
                path: "etc/users.csv".into(),
                content: "ada\ngrace\n".into(),
            },
        ];
        assert_eq!(apply_files(&root, &files, true).expect("apply"), 2);

        let listing = list(&root, "").expect("list root");
        assert!(listing.contains("README.md"));
        assert!(listing.contains("etc/users.csv"));

        let listing = list(&root, "etc").expect("list dir");
        assert!(listing.contains("users.csv"));

        assert_eq!(read(&root, "README.md").expect("read"), "hello");
        let st = stat(&root, "README.md").expect("stat");
        let v: Json = serde_json::from_str(&st).expect("json");
        assert_eq!(v["kind"], json!("file"));
        assert_eq!(v["bytes"], json!(5));
        let st = stat(&root, "nope").expect("stat missing");
        assert!(st.contains("missing"));
    }

    #[test]
    fn client_ops_refuse_escape_and_absolute_paths() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        assert!(write_client_file(&root, "../escape.txt", "x").is_err());
        assert!(write_client_file(&root, "/etc/evil", "x").is_err());
        assert!(read(&root, "../../etc/passwd").is_err());
        assert!(list(&root, "..").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn write_refuses_final_component_symlink() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        symlink("/etc/passwd", root.join("evil")).expect("symlink");
        assert!(write_client_file(&root, "evil", "x").is_err());
        assert!(read(&root, "evil").is_err());
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_directory_inside_root_cannot_redirect_writes() {
        use std::os::unix::fs::symlink;
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        let outside = dir.path().join("outside");
        std::fs::create_dir_all(&outside).expect("outside");
        symlink(&outside, root.join("real")).expect("symlink dir onto outside");
        // 'real' resolves to ./outside — outside the root, so refused.
        assert!(write_client_file(&root, "real/note.txt", "x").is_err());
    }

    #[test]
    fn write_caps_enforced() {
        let dir = tempfile::tempdir().expect("tempdir");
        let root = dir.path().join("root");
        std::fs::create_dir_all(&root).expect("root");
        let err = write_client_file(&root, "big.txt", &"x".repeat(WRITE_BYTES_MAX + 1))
            .expect_err("over cap");
        assert!(err.contains("max"));
    }

    #[test]
    fn extract_json_object_tolerates_fences_and_prose() {
        assert_eq!(extract_json_object("{}"), Some("{}"));
        assert_eq!(
            extract_json_object("```json\n{\"a\":1}\n```"),
            Some("{\"a\":1}")
        );
        assert_eq!(
            extract_json_object("here: {\"a\":1} done"),
            Some("{\"a\":1}")
        );
        assert_eq!(extract_json_object("nope"), None);
    }

    #[test]
    fn ensure_tree_reuses_on_matching_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let tool_dir = dir.path().join("tools");
        std::fs::create_dir_all(&tool_dir).expect("tools");
        let tool_path = tool_dir.join("tool.md");
        std::fs::write(&tool_path, "persona").expect("tool file");
        let tool = ToolDef {
            name: "docs".to_string(),
            description: "d".to_string(),
            model: None,
            driver: "vfs".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "persona".to_string(),
            path: tool_path.clone(),
        };
        struct OnceFabricated;
        impl ChatBackend for OnceFabricated {
            fn stream_turn(
                &self,
                _model: &str,
                _system: &str,
                _messages: &[ChatMessage],
                _on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                Ok("{\"files\":[{\"path\":\"a.txt\",\"content\":\"A\"}]}".to_string())
            }
        }
        let vfs_dir = dir.path().join("vfs");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let backend = OnceFabricated;
        let (root, turns) =
            ensure_tree(&tool, &backend, "m", &audit, "run", &vfs_dir, false).expect("fabricate");
        assert_eq!(turns, 1);
        assert_eq!(read(&root, "a.txt").expect("read"), "A");
        // Reuse: no backend call would panic-free return 0 turns.
        let (root2, turns2) =
            ensure_tree(&tool, &backend, "m", &audit, "run", &vfs_dir, false).expect("reuse");
        assert_eq!(turns2, 0);
        assert_eq!(root2, root);
        assert!(marker_matches(&vfs_dir, &tool_path));
    }

    fn marker_matches(vfs_dir: &Path, tool_path: &Path) -> bool {
        let (digest, _) = support::sha256_file(tool_path).expect("digest");
        std::fs::read_to_string(vfs_dir.join(support::MARKER_VFS))
            .map(|stored| stored.trim() == digest)
            .unwrap_or(false)
    }
}
