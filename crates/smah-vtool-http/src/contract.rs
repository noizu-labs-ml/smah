//! OpenAPI contract handling for the api driver (#20): structural
//! validation (validate-then-freeze), compact summaries for fabrication
//! prompts, atomic cache writes, and the `tool.sha256` staleness marker.
//!
//! The validator is deliberately shallow — a structural gate, not a full
//! OpenAPI linter. Anything that passes it is treated as the frozen
//! contract served at `/openapi.json`; endpoint *behavior* is fabricated at
//! request time against that spec.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::Value;

/// A contract that passed structural validation. `raw` is the exact bytes
/// served at `/openapi.json` — byte-for-byte, never re-serialized.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatedSpec {
    pub raw: String,
    /// `info.title`, when a string.
    pub title: Option<String>,
    /// `info.version`, when a string.
    pub version: Option<String>,
    /// Number of entries under `paths`.
    pub path_count: usize,
}

/// Poisoned/oversized cache guard: a contract larger than this is rejected
/// on load (the model is prompt-capped, so a bigger file is not model-
/// authored within the contract-generation loop).
pub const SPEC_MAX_BYTES: usize = 4 * 1024 * 1024;

/// Summary cap fed into fabrication prompts (paths are truncated at this
/// budget; the spec file itself remains the source of truth).
const SUMMARY_MAX_BYTES: usize = 4096;

/// Validate a prospective contract.
///
/// Requires: valid JSON object; `"openapi"` a non-empty string; `"paths"`
/// an object whose entries are all objects. Everything else (info, servers,
/// schemas) is accepted as-is.
pub fn validate(text: &str) -> Result<ValidatedSpec, ContractError> {
    if text.len() > SPEC_MAX_BYTES {
        return Err(ContractError::TooLarge {
            bytes: text.len(),
            max: SPEC_MAX_BYTES,
        });
    }
    let value: Value =
        serde_json::from_str(text).map_err(|err| ContractError::InvalidJson(err.to_string()))?;
    let obj = value.as_object().ok_or(ContractError::NotAnObject)?;
    match obj.get("openapi") {
        Some(Value::String(s)) if !s.is_empty() => {}
        Some(_) => return Err(ContractError::OpenapiNotAString),
        None => return Err(ContractError::MissingOpenapi),
    }
    let paths = match obj.get("paths") {
        Some(Value::Object(map)) => map,
        Some(_) => return Err(ContractError::PathsNotAnObject),
        None => return Err(ContractError::MissingPaths),
    };
    for (path, item) in paths {
        if !item.is_object() {
            return Err(ContractError::PathNotAnObject(path.clone()));
        }
    }
    let info = obj.get("info").and_then(|v| v.as_object());
    Ok(ValidatedSpec {
        raw: text.to_string(),
        title: info
            .and_then(|i| i.get("title"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        version: info
            .and_then(|i| i.get("version"))
            .and_then(|v| v.as_str())
            .map(str::to_string),
        path_count: paths.len(),
    })
}

/// Compact one-line summary for fabrication prompts: title/version plus the
/// method+path inventory, truncated at [`SUMMARY_MAX_BYTES`].
pub fn summary(spec: &ValidatedSpec) -> String {
    let value: Value = match serde_json::from_str(&spec.raw) {
        Ok(v) => v,
        Err(_) => return "(spec unparseable)".to_string(),
    };
    let mut out = String::new();
    if let (Some(title), Some(version)) = (&spec.title, &spec.version) {
        out.push_str(&format!("{title} (version {version}). "));
    } else if let Some(title) = &spec.title {
        out.push_str(&format!("{title}. "));
    }
    out.push_str(&format!("Endpoints ({}): ", spec.path_count));
    let mut lines: Vec<String> = Vec::new();
    if let Some(paths) = value.get("paths").and_then(|v| v.as_object()) {
        for (path, item) in paths {
            let methods: Vec<String> = item
                .as_object()
                .map(|m| {
                    m.keys()
                        .filter(|k| {
                            matches!(
                                k.as_str(),
                                "get" | "post" | "put" | "patch" | "delete" | "head" | "options"
                            )
                        })
                        .map(|k| k.to_uppercase())
                        .collect()
                })
                .unwrap_or_default();
            if methods.is_empty() {
                lines.push(path.clone());
            } else {
                lines.push(format!("{} {path}", methods.join(",")));
            }
        }
    }
    lines.sort();
    let mut inventory = lines.join("; ");
    if inventory.len() > SUMMARY_MAX_BYTES {
        inventory.truncate(SUMMARY_MAX_BYTES);
        // Cut at a boundary, not mid-token.
        if let Some(pos) = inventory.rfind("; ") {
            inventory.truncate(pos);
        }
        inventory.push_str("; ...");
    }
    out.push_str(&inventory);
    out
}

/// The staleness marker path: `<contract>.sha256` next to the contract.
pub fn marker_path(contract: &Path) -> PathBuf {
    let mut p = contract.as_os_str().to_os_string();
    p.push(".sha256");
    PathBuf::from(p)
}

/// Read the recorded tool-file sha, when present.
pub fn read_marker(contract: &Path) -> Option<String> {
    fs::read_to_string(marker_path(contract))
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Record the tool-file sha for a freshly generated contract.
pub fn write_marker(contract: &Path, sha: &str) -> io::Result<()> {
    fs::write(marker_path(contract), sha)
}

/// Atomic write (tmp + rename in the same directory) so a reader never sees
/// a half-written contract or content file.
pub fn write_atomic(path: &Path, bytes: &str) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes)?;
    fs::rename(&tmp, path)
}

/// Contract-validation failures (cache poisoning guard, T-021).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ContractError {
    /// Not valid JSON.
    #[error("contract is not valid JSON: {0}")]
    InvalidJson(String),
    /// Top-level value is not a JSON object.
    #[error("contract root must be a JSON object")]
    NotAnObject,
    /// `openapi` key missing.
    #[error("contract missing the 'openapi' version string")]
    MissingOpenapi,
    /// `openapi` present but not a non-empty string.
    #[error("'openapi' must be a non-empty string")]
    OpenapiNotAString,
    /// `paths` key missing.
    #[error("contract missing the 'paths' object")]
    MissingPaths,
    /// `paths` present but not an object.
    #[error("'paths' must be a JSON object")]
    PathsNotAnObject,
    /// A `paths` entry is not an object.
    #[error("paths entry '{0}' must be a JSON object")]
    PathNotAnObject(String),
    /// Contract exceeds the size cap.
    #[error("contract exceeds the {max} byte cap ({bytes} bytes)")]
    TooLarge {
        /// Actual size.
        bytes: usize,
        /// The cap.
        max: usize,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    const GOOD: &str = r#"{
        "openapi": "3.1.0",
        "info": {"title": "Widget API", "version": "1.0"},
        "paths": {
            "/widgets": {"get": {"summary": "list"}},
            "/widgets/{id}": {"get": {"summary": "one"}, "delete": {"summary": "del"}}
        }
    }"#;

    #[test]
    fn valid_minimal_spec() {
        let spec = validate(GOOD).expect("valid");
        assert_eq!(spec.title.as_deref(), Some("Widget API"));
        assert_eq!(spec.version.as_deref(), Some("1.0"));
        assert_eq!(spec.path_count, 2);
        // raw is byte-for-byte
        assert_eq!(spec.raw, GOOD);
    }

    #[test]
    fn rejects_broken_contracts() {
        assert!(matches!(
            validate("not json"),
            Err(ContractError::InvalidJson(_))
        ));
        assert!(matches!(validate("[1]"), Err(ContractError::NotAnObject)));
        assert!(matches!(validate("{}"), Err(ContractError::MissingOpenapi)));
        assert!(matches!(
            validate(r#"{"openapi": 3}"#),
            Err(ContractError::OpenapiNotAString)
        ));
        assert!(matches!(
            validate(r#"{"openapi": "3.1.0"}"#),
            Err(ContractError::MissingPaths)
        ));
        assert!(matches!(
            validate(r#"{"openapi": "3.1.0", "paths": []}"#),
            Err(ContractError::PathsNotAnObject)
        ));
        assert!(matches!(
            validate(r#"{"openapi": "3.1.0", "paths": {"/a": "x"}}"#),
            Err(ContractError::PathNotAnObject(p)) if p == "/a"
        ));
        let big = format!(
            "{{\"openapi\":\"3.1.0\",\"x\":\"{}\"}}",
            "y".repeat(SPEC_MAX_BYTES)
        );
        assert!(matches!(
            validate(&big),
            Err(ContractError::TooLarge { .. })
        ));
    }

    #[test]
    fn summary_lists_methods_and_paths() {
        let spec = validate(GOOD).expect("valid");
        let s = summary(&spec);
        assert!(s.contains("Widget API (version 1.0)"), "{s}");
        assert!(s.contains("GET /widgets"), "{s}");
        assert!(s.contains("GET,DELETE /widgets/{id}"), "{s}");
    }

    #[test]
    fn atomic_write_and_marker_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let contract = dir.path().join("api/openapi.json");
        write_atomic(&contract, GOOD).expect("write");
        assert_eq!(fs::read_to_string(&contract).expect("read"), GOOD);
        assert!(!contract.with_extension("tmp").exists());

        write_marker(&contract, "abc123").expect("marker");
        assert_eq!(read_marker(&contract).as_deref(), Some("abc123"));
        assert!(read_marker(&dir.path().join("missing.json")).is_none());
    }
}
