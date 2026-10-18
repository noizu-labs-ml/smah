//! VFS virtual-tool driver (ticket #25): fabricated file contents as real
//! files, served over code-owned MCP-style fs tools.
//!
//! The tree is fabricated (one bounded model turn → validated spec → real
//! files under `~/.cache/smah/virtual-tools/<tool>/vfs/root/`, regenerable
//! cache keyed by `vfs.sha256`) and served over JSON-RPC — batch stdio by
//! default, opt-in loopback TCP with `--tcp-port <n>`. The fs tool catalog is
//! **code-owned** (`fs_list`, `fs_read`, `fs_write`, `fs_stat`, `fs_generate`),
//! never fabricated, so containment policy cannot be talked around. Escape
//! attempts from clients are refused per-request (audited, server keeps
//! serving, T-026); an escape in the fabrication spec aborts at exit 3.
//!
//! Flags: `--stdio` (accepted no-op), `--no-serve` (fabricate only),
//! `--show-tree` (print the fabricated tree and exit), `--regen`,
//! `--tcp-port <n>`. Anything else is a Config-class error (exit 3).

use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::driver::{invocation_block, ChatMessage};
use crate::virtual_tools::drivers::db::support;
use crate::virtual_tools::drivers::mcp::catalog::CatalogEntry;
use crate::virtual_tools::drivers::mcp::jsonrpc::{Request, RpcError, RpcHandler};
use crate::virtual_tools::drivers::{ChatBackend, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;

#[path = "vfs_tree.rs"]
pub(crate) mod tree;

/// The code-owned fs tool catalog (never fabricated).
fn fixed_catalog() -> Vec<CatalogEntry> {
    vec![
        CatalogEntry {
            name: "fs_list".to_string(),
            description: "List entries under a directory of the virtual tree (\"\" = root)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string", "description": "directory path, relative to the tool root"}},
            }),
        },
        CatalogEntry {
            name: "fs_read".to_string(),
            description: "Read a text file from the virtual tree.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
            }),
        },
        CatalogEntry {
            name: "fs_write".to_string(),
            description: "Write (or overwrite) a text file inside the virtual tree (64 KiB cap)."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "path": {"type": "string"},
                    "content": {"type": "string"},
                },
                "required": ["path", "content"],
            }),
        },
        CatalogEntry {
            name: "fs_stat".to_string(),
            description: "Describe a path: kind (file/dir/missing), byte size.".to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"],
            }),
        },
        CatalogEntry {
            name: "fs_generate".to_string(),
            description: "Ask the tool's persona to fabricate additional files into the tree."
                .to_string(),
            input_schema: json!({
                "type": "object",
                "properties": {"hint": {"type": "string", "description": "what to add"}},
            }),
        },
    ]
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct VfsArgs {
    tcp_port: Option<u16>,
    no_serve: bool,
    show_tree: bool,
    regen: bool,
}

fn parse_args(tool: &ToolDef, args: &[String]) -> Result<VfsArgs, SmahError> {
    let invalid = |detail: String| SmahError::ToolFileInvalid {
        path: tool.path.display().to_string(),
        detail,
    };
    let mut parsed = VfsArgs {
        tcp_port: None,
        no_serve: false,
        show_tree: false,
        regen: false,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--stdio" => {}
            "--no-serve" => parsed.no_serve = true,
            "--show-tree" => parsed.show_tree = true,
            "--regen" => parsed.regen = true,
            "--tcp-port" => {
                let value = iter.next().ok_or_else(|| {
                    invalid(format!(
                        "the {} driver requires --tcp-port <n>",
                        tool.driver
                    ))
                })?;
                let port = value.parse::<u16>().map_err(|_| {
                    invalid(format!(
                        "the {} driver rejects --tcp-port value '{value}' \
                         (not a port number, 0-65535)",
                        tool.driver
                    ))
                })?;
                parsed.tcp_port = Some(port);
            }
            other => {
                return Err(invalid(format!(
                    "the vfs driver does not recognize argument '{other}' \
                     (supported: --stdio, --no-serve, --show-tree, --regen, --tcp-port <n>)"
                )))
            }
        }
    }
    Ok(parsed)
}

/// Method dispatcher over the fabricated tree: handshake, discovery, and the
/// contained fs tools. Client escape attempts are refused per-request and
/// audited; the server keeps serving (T-026).
struct VfsHandler<'a> {
    tool: &'a ToolDef,
    backend: &'a dyn ChatBackend,
    model: &'a str,
    audit: &'a Audit,
    run_id: &'a str,
    dir: PathBuf,
    root: PathBuf,
    /// Fabrication turns spent on `fs_generate` — folded into the run's
    /// `RunOutcome::turns` so callers see total model exposure.
    call_turns: std::cell::Cell<usize>,
}

impl VfsHandler<'_> {
    fn initialize(&self, params: &Value) -> Result<Value, RpcError> {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let version = if requested == "2024-11-05" || requested == "2025-06-18" {
            requested
        } else {
            "2024-11-05"
        };
        Ok(json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": format!("smah-vtool-vfs-{}", self.tool.name),
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": format!(
                "Virtual filesystem for '{}' (FABRICATED DATA). All paths are relative to the tool root.",
                self.tool.name
            ),
        }))
    }

    fn audit_call(
        &self,
        op: &str,
        path: &str,
        bytes: Option<usize>,
        decision: &'static str,
        reason: &str,
    ) {
        support::audit_server_op(
            self.audit,
            self.run_id,
            &self.tool.name,
            "vfs.call",
            &format!("{op}:{path}"),
            bytes,
            decision,
            reason,
        );
    }

    fn text_result(text: String) -> Value {
        json!({"content": [{"type": "text", "text": text}], "isError": false})
    }

    fn fs_list(&mut self, args: &Value) -> Result<Value, RpcError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        match tree::list(&self.root, path) {
            Ok(listing) => {
                self.audit_call("fs_list", path, Some(listing.len()), "allowed", "");
                Ok(Self::text_result(listing))
            }
            Err(reason) => {
                self.audit_call("fs_list", path, None, "denied", &reason);
                Err(RpcError::new(RpcErrorCode::params(), reason))
            }
        }
    }

    fn fs_read(&mut self, args: &Value) -> Result<Value, RpcError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(RpcErrorCode::params(), "'path' must be a string"))?;
        match tree::read(&self.root, path) {
            Ok(content) => {
                self.audit_call("fs_read", path, Some(content.len()), "allowed", "");
                Ok(Self::text_result(content))
            }
            Err(reason) => {
                self.audit_call("fs_read", path, None, "denied", &reason);
                Err(RpcError::new(RpcErrorCode::params(), reason))
            }
        }
    }

    fn fs_write(&mut self, args: &Value) -> Result<Value, RpcError> {
        let path = args
            .get("path")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(RpcErrorCode::params(), "'path' must be a string"))?;
        let content = args
            .get("content")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(RpcErrorCode::params(), "'content' must be a string"))?;
        match tree::write_client_file(&self.root, path, content) {
            Ok(bytes) => {
                self.audit_call("fs_write", path, Some(bytes), "allowed", "");
                Ok(Self::text_result(format!("wrote {bytes} bytes to {path}")))
            }
            Err(reason) => {
                self.audit_call("fs_write", path, Some(content.len()), "denied", &reason);
                Err(RpcError::new(RpcErrorCode::params(), reason))
            }
        }
    }

    fn fs_stat(&mut self, args: &Value) -> Result<Value, RpcError> {
        let path = args.get("path").and_then(Value::as_str).unwrap_or("");
        match tree::stat(&self.root, path) {
            Ok(desc) => {
                self.audit_call("fs_stat", path, None, "allowed", "");
                Ok(Self::text_result(desc))
            }
            Err(reason) => {
                self.audit_call("fs_stat", path, None, "denied", &reason);
                Err(RpcError::new(RpcErrorCode::params(), reason))
            }
        }
    }

    /// Runtime fabrication: one bounded turn adds files to the existing tree.
    /// An escape attempt is refused here (audited) — the server keeps serving;
    /// hard-abort is reserved for driver-startup fabrication.
    fn fs_generate(&mut self, args: &Value) -> Result<Value, RpcError> {
        let hint = args.get("hint").and_then(Value::as_str).unwrap_or("");
        let mut user = String::new();
        user.push_str("VIRTUAL FILESYSTEM PERSONA:\n");
        user.push_str(&self.tool.body);
        user.push_str("\n\nThe tree already contains (from its manifest):\n");
        match std::fs::read_to_string(tree::manifest_path(&self.dir)) {
            Ok(manifest) => user.push_str(&manifest),
            Err(_) => user.push_str("(empty — nothing fabricated yet)"),
        }
        if !hint.is_empty() {
            user.push_str("\n\nThe client asks you to add:\n");
            user.push_str(hint);
        }
        user.push_str("\n\nOutput only the JSON object with the additional files.");
        let reply = self
            .backend
            .stream_turn(
                self.model,
                tree::FABRICATE_SYSTEM,
                &[ChatMessage::user(&user)],
                &mut |_| {},
            )
            .map_err(|err| RpcError::new(RpcErrorCode::internal(), err.to_string()))?;
        self.call_turns.set(self.call_turns.get() + 1);
        let text = tree::extract_json_object(&reply).ok_or_else(|| {
            RpcError::new(
                RpcErrorCode::internal(),
                "generation produced no JSON object",
            )
        })?;
        let parsed: Value = serde_json::from_str(text).map_err(|err| {
            RpcError::new(
                RpcErrorCode::internal(),
                format!("spec did not parse: {err}"),
            )
        })?;
        let files = tree::parse_spec(&parsed).map_err(|err| match err {
            tree::SpecError::Escape(detail) => {
                self.audit_call("fs_generate", hint, Some(reply.len()), "denied", &detail);
                RpcError::new(RpcErrorCode::params(), format!("refused: {detail}"))
            }
            tree::SpecError::Invalid(detail) => RpcError::new(
                RpcErrorCode::internal(),
                format!("invalid generated spec: {detail}"),
            ),
        })?;
        tree::apply_files(&self.root, &files, false)
            .map_err(|err| RpcError::new(RpcErrorCode::internal(), format!("tree apply: {err}")))?;
        self.append_manifest(&files);
        let summary: Vec<String> = files.iter().map(|f| f.path.clone()).collect();
        self.audit_call("fs_generate", hint, Some(reply.len()), "allowed", "");
        Ok(Self::text_result(format!(
            "added {} file(s): {}",
            files.len(),
            summary.join(", ")
        )))
    }

    /// Extend the fabrication receipt with runtime-generated files.
    fn append_manifest(&self, files: &[tree::FileSpec]) {
        let path = tree::manifest_path(&self.dir);
        let mut entries: Vec<Value> = Vec::new();
        if let Ok(raw) = std::fs::read_to_string(&path) {
            if let Ok(Value::Object(existing)) = serde_json::from_str::<Value>(&raw) {
                if let Some(Value::Array(files_array)) = existing.get("files") {
                    entries = files_array.clone();
                }
            }
        }
        for file in files {
            entries.push(json!({"path": file.path, "bytes": file.content.len()}));
        }
        if let Ok(text) = serde_json::to_string_pretty(&json!({"files": entries})) {
            let _ = std::fs::write(path, text);
        }
    }
}

/// Local alias so the error constructors read cleanly.
struct RpcErrorCode;
impl RpcErrorCode {
    fn params() -> i64 {
        crate::virtual_tools::drivers::mcp::jsonrpc::INVALID_PARAMS
    }
    fn internal() -> i64 {
        crate::virtual_tools::drivers::mcp::jsonrpc::INTERNAL
    }
}

impl RpcHandler for VfsHandler<'_> {
    fn call(&mut self, request: &Request) -> Result<Value, RpcError> {
        match request.method.as_str() {
            "initialize" => self.initialize(&request.params),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": fixed_catalog().iter().map(CatalogEntry::to_wire).collect::<Vec<_>>()
            })),
            "tools/call" => {
                let name = request
                    .params
                    .get("name")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        RpcError::new(RpcErrorCode::params(), "'name' must be a string")
                    })?;
                let args = request
                    .params
                    .get("arguments")
                    .cloned()
                    .unwrap_or_else(|| json!({}));
                match name {
                    "fs_list" => self.fs_list(&args),
                    "fs_read" => self.fs_read(&args),
                    "fs_write" => self.fs_write(&args),
                    "fs_stat" => self.fs_stat(&args),
                    "fs_generate" => self.fs_generate(&args),
                    other => Err(RpcError::new(
                        RpcErrorCode::params(),
                        format!(
                            "no such tool '{other}' (this server exposes only the fixed fs tool set)"
                        ),
                    )),
                }
            }
            method if method.starts_with("notifications/") => Err(RpcError::new(
                RpcErrorCode::request(),
                "notifications must not carry an id",
            )),
            _ => Err(RpcError::new(
                RpcErrorCode::not_found(),
                format!(
                    "unknown method '{}' (supported: initialize, ping, tools/list, tools/call)",
                    request.method
                ),
            )),
        }
    }
}

impl RpcErrorCode {
    fn request() -> i64 {
        crate::virtual_tools::drivers::mcp::jsonrpc::INVALID_REQUEST
    }
    fn not_found() -> i64 {
        crate::virtual_tools::drivers::mcp::jsonrpc::METHOD_NOT_FOUND
    }
}

/// `--show-tree`: walk the fabricated tree and print relative paths + sizes.
fn show_tree(tool: &ToolDef, root: &Path, sink: &mut dyn Sink) {
    sink.content(&format!(
        "virtual tree '{}' — FABRICATED DATA under {}\n",
        tool.name,
        root.display()
    ));
    fn walk_print(dir: &Path, prefix_len: usize, depth: usize, sink: &mut dyn Sink) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<_> = entries.filter_map(Result::ok).collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            let rel = path
                .to_string_lossy()
                .get(prefix_len..)
                .unwrap_or("")
                .to_string();
            let indent = "  ".repeat(depth + 1);
            match entry.file_type() {
                Ok(ft) if ft.is_dir() => {
                    sink.content(&format!("{indent}{rel}/\n"));
                    walk_print(&path, prefix_len, depth + 1, sink);
                }
                _ => {
                    let bytes = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    sink.content(&format!("{indent}{rel}  {bytes} bytes\n"));
                }
            }
        }
    }
    if let Ok(canonical) = std::fs::canonicalize(root) {
        walk_print(&canonical, canonical.as_os_str().len(), 0, sink);
    }
}

/// Run the vfs driver: fabricate (or reuse) the tree, then serve or print.
#[allow(clippy::too_many_arguments)]
pub fn run(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    _fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    let parsed = parse_args(tool, args)?;
    let dir = support::cache_leaf(&tool.name, "vfs");

    let (root, turns) = tree::ensure_tree(tool, backend, model, audit, run_id, &dir, parsed.regen)?;

    let outcome = |turns: usize, messages: Vec<ChatMessage>| RunOutcome {
        turns,
        wrote_files: true,
        messages,
    };
    let invocation = vec![ChatMessage::user(invocation_block(
        args,
        stdin,
        stdin_partial,
    ))];

    if parsed.show_tree {
        show_tree(tool, &root, sink);
        return Ok(outcome(turns, invocation));
    }
    if parsed.no_serve {
        sink.notice(&format!(
            "smah(vfs): fabricated virtual file tree for '{}' is ready under {} \
             (FABRICATED DATA; rerun without --no-serve to serve it over the fs tools)",
            tool.name,
            root.display()
        ));
        return Ok(outcome(turns, invocation));
    }

    let mut handler = VfsHandler {
        tool,
        backend,
        model,
        audit,
        run_id,
        dir,
        root,
        call_turns: std::cell::Cell::new(0),
    };

    if let Some(port) = parsed.tcp_port {
        let handler_turns = handler.call_turns.get();
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|err| {
            SmahError::ToolFileInvalid {
                path: format!("127.0.0.1:{port}"),
                detail: format!(
                    "the vfs driver could not bind its loopback TCP listener: {}",
                    support::one_line(&err.to_string())
                ),
            }
        })?;
        let bound = listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or(port);
        sink.notice(&format!(
            "smah(vfs): '{}' virtual filesystem (FABRICATED DATA) on 127.0.0.1:{bound} — \
             loopback only, newline-delimited JSON-RPC 2.0, sequential connections (ctrl-c to stop)",
            tool.name
        ));
        support::audit_server_op(
            audit,
            run_id,
            &tool.name,
            "vfs.serve",
            "tcp-loopback",
            Some(bound as usize),
            "allowed",
            "",
        );
        crate::virtual_tools::drivers::mcp::jsonrpc::serve_tcp(
            listener,
            handler,
            &mut |line: &str| sink.notice(line),
        )
        .map_err(|err| SmahError::Internal {
            detail: format!("vfs tcp serve: {}", support::one_line(&err.to_string())),
        })?;
        return Ok(outcome(turns + handler_turns, invocation)); // unreachable in practice
    }

    // Default: newline-delimited batch over the invocation's stdin.
    let stdin_text = stdin.unwrap_or("");
    if stdin_text.trim().is_empty() {
        sink.notice(
            "smah(vfs): no requests on stdin (batch mode reads stdin to EOF); \
             start an interactive client with --tcp-port <n> or use --show-tree / --no-serve",
        );
        return Ok(outcome(turns, invocation));
    }
    let answered = crate::virtual_tools::drivers::mcp::jsonrpc::serve_stdio_batch(
        stdin_text,
        stdin_partial,
        sink,
        &mut handler,
    );
    support::audit_server_op(
        audit,
        run_id,
        &tool.name,
        "vfs.serve",
        "stdio-batch",
        Some(answered),
        "allowed",
        "",
    );
    Ok(outcome(turns + handler.call_turns.get(), invocation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_tools::drivers::stub_test_support::{NullBackend, NullSink};

    fn tool() -> ToolDef {
        ToolDef {
            name: "docs".to_string(),
            description: "fixture docs".to_string(),
            model: None,
            driver: "vfs".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "persona".to_string(),
            path: std::path::PathBuf::from("docs.md"),
        }
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn args_parse_the_contract_flags() {
        let t = tool();
        let parsed = parse_args(&t, &[]).expect("empty");
        assert_eq!(
            parsed,
            VfsArgs {
                tcp_port: None,
                no_serve: false,
                show_tree: false,
                regen: false
            }
        );
        let parsed = parse_args(&t, &s(&["--stdio", "--no-serve", "--show-tree", "--regen"]))
            .expect("flags");
        assert!(parsed.no_serve && parsed.show_tree && parsed.regen);
        let parsed = parse_args(&t, &s(&["--tcp-port", "0"])).expect("ephemeral");
        assert_eq!(parsed.tcp_port, Some(0));
    }

    #[test]
    fn args_reject_unknown_flags_and_bad_ports() {
        let t = tool();
        for bad in [vec!["--tree"], vec!["extra"], vec!["--tcp-port", "http"]] {
            let err = parse_args(&t, &s(&bad)).expect_err(bad.join(" ").as_str());
            assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Config);
            assert!(err.to_string().contains("vfs driver"), "{err}");
        }
    }

    fn handler<'a>(
        t: &'a ToolDef,
        dir: &'a std::path::Path,
        root: &'a std::path::Path,
        backend: &'a dyn ChatBackend,
        audit: &'a Audit,
    ) -> VfsHandler<'a> {
        VfsHandler {
            tool: t,
            backend,
            model: "m",
            audit,
            run_id: "run",
            dir: dir.to_path_buf(),
            root: root.to_path_buf(),
            call_turns: std::cell::Cell::new(0),
        }
    }

    fn seeded_root(dir: &std::path::Path) -> PathBuf {
        let root = dir.join("root");
        std::fs::create_dir_all(root.join("etc")).expect("tree");
        std::fs::write(root.join("README.md"), "hello").expect("file");
        std::fs::write(root.join("etc/users.csv"), "ada\ngrace\n").expect("file");
        root
    }

    fn call_req(method: &str, params: Value, id: i64) -> Request {
        Request {
            method: method.to_string(),
            params,
            id: Some(json!(id)),
        }
    }

    #[test]
    fn handler_serves_the_fixed_fs_tools() {
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let root = seeded_root(dir.path());
        let backend = NullBackend;
        let mut h = handler(&t, dir.path(), &root, &backend, &audit);

        let list = h
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_list", "arguments": {}}),
                1,
            ))
            .expect("list");
        assert!(list["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("README.md"));

        let read = h
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_read", "arguments": {"path": "etc/users.csv"}}),
                2,
            ))
            .expect("read");
        assert_eq!(read["content"][0]["text"], json!("ada\ngrace\n"));

        let write = h
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_write", "arguments": {"path": "notes/x.txt", "content": "note"}}),
                3,
            ))
            .expect("write");
        assert!(write["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("wrote 4 bytes"));
        assert_eq!(
            std::fs::read_to_string(root.join("notes/x.txt")).expect("written"),
            "note"
        );

        let stat = h
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_stat", "arguments": {"path": "README.md"}}),
                4,
            ))
            .expect("stat");
        assert!(
            stat["content"][0]["text"]
                .as_str()
                .expect("text")
                .contains("\"kind\": \"file\"")
                || stat["content"][0]["text"]
                    .as_str()
                    .expect("text")
                    .contains("\"kind\":\"file\"")
        );

        let list_res = h
            .call(&call_req("tools/list", json!({}), 5))
            .expect("tools/list");
        let names: Vec<&str> = list_res["tools"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|t| t["name"].as_str())
            .collect();
        assert_eq!(
            names,
            vec!["fs_list", "fs_read", "fs_write", "fs_stat", "fs_generate"]
        );
    }

    #[test]
    fn handler_refuses_escape_attempts_and_keeps_catalog_fixed() {
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let root = seeded_root(dir.path());
        let backend = NullBackend;
        let mut h = handler(&t, dir.path(), &root, &backend, &audit);

        for bad in ["../escape.txt", "/etc/passwd"] {
            let err = h
                .call(&call_req(
                    "tools/call",
                    json!({"name": "fs_write", "arguments": {"path": bad, "content": "x"}}),
                    1,
                ))
                .expect_err(bad);
            assert_eq!(
                err.code,
                crate::virtual_tools::drivers::mcp::jsonrpc::INVALID_PARAMS
            );
        }
        // The unknown-tool refusal names the fixed set (no fabricated tools).
        let err = h
            .call(&call_req(
                "tools/call",
                json!({"name": "rm_rf", "arguments": {}}),
                2,
            ))
            .expect_err("unknown tool");
        assert!(err.message.contains("fixed fs tool set"), "{err:?}");

        let err = h
            .call(&call_req("resources/list", json!({}), 3))
            .expect_err("unknown method");
        assert_eq!(
            err.code,
            crate::virtual_tools::drivers::mcp::jsonrpc::METHOD_NOT_FOUND
        );
    }

    #[test]
    fn fs_generate_adds_files_from_one_fabrication_turn() {
        struct Scripted;
        impl ChatBackend for Scripted {
            fn stream_turn(
                &self,
                _model: &str,
                _system: &str,
                _messages: &[ChatMessage],
                _on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                Ok("{\"files\":[{\"path\":\"meeting-notes/2026-10-18.md\",\"content\":\"# notes\"}]}"
                    .to_string())
            }
        }
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let root = seeded_root(dir.path());
        let backend = Scripted;
        let mut h = handler(&t, dir.path(), &root, &backend, &audit);
        let result = h
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_generate", "arguments": {"hint": "today's notes"}}),
                1,
            ))
            .expect("generate");
        assert!(result["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("added 1 file"));
        assert_eq!(
            std::fs::read_to_string(root.join("meeting-notes/2026-10-18.md")).expect("written"),
            "# notes"
        );
        // Escape attempt at generate time: refused, not fatal.
        struct Escapist;
        impl ChatBackend for Escapist {
            fn stream_turn(
                &self,
                _model: &str,
                _system: &str,
                _messages: &[ChatMessage],
                _on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                Ok("{\"files\":[{\"path\":\"../escape\",\"content\":\"x\"}]}".to_string())
            }
        }
        let mut h2 = handler(&t, dir.path(), &root, &Escapist, &audit);
        let err = h2
            .call(&call_req(
                "tools/call",
                json!({"name": "fs_generate", "arguments": {}}),
                2,
            ))
            .expect_err("escape refused");
        assert!(err.message.contains("refused"), "{err:?}");
    }

    #[test]
    fn run_refuses_unknown_flags_before_any_model_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let t = tool();
        let mut sink = NullSink;
        let err = run(
            &t,
            &NullBackend,
            &mut sink,
            "m",
            &s(&["--tree"]),
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
        )
        .expect_err("unknown flag");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    }
}
