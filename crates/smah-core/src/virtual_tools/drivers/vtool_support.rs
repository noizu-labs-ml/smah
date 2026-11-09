//! Shared helpers for the mcp/db/vfs driver siblings (tickets #23/#24/#25).
//!
//! Lives as a sibling file so `drivers/mod.rs` stays untouched: the db
//! driver declares it with `#[path = "vtool_support.rs"] pub(crate) mod
//! support;` and the mcp/vfs drivers reach it through
//! `super::db::support` (same crate, one copy compiled).

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

use crate::virtual_tools::audit::{Audit, AuditRecord};

/// Hex sha256 of a byte slice (cache markers, memo keys).
pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let mut out = String::with_capacity(digest.len() * 2);
    for byte in digest {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Hex sha256 of a file plus its byte length (marker + audit `bytes`).
pub(crate) fn sha256_file(path: &Path) -> Result<(String, u64), String> {
    let bytes = std::fs::read(path).map_err(|err| one_line(&err.to_string()))?;
    let len = bytes.len() as u64;
    Ok((sha256_hex(&bytes), len))
}

/// Collapse whitespace to one line (audit reasons and error `<detail>`s).
pub(crate) fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The per-driver cache leaf dir:
/// `~/.cache/smah/virtual-tools/<tool>/<leaf>` (`leaf` ∈ `mcp|db|vfs`).
/// Honors `XDG_CACHE_HOME` through [`crate::virtual_tools::audit::cache_dir`].
pub(crate) fn cache_leaf(tool: &str, leaf: &str) -> PathBuf {
    crate::virtual_tools::audit::cache_dir()
        .join("virtual-tools")
        .join(tool)
        .join(leaf)
}

/// Marker file name for a driver cache leaf (NOT the shared cli marker):
/// `mcp.sha256` / `db.sha256` / `vfs.sha256`.
pub(crate) const MARKER_MCP: &str = "mcp.sha256";
pub(crate) const MARKER_DB: &str = "db.sha256";
pub(crate) const MARKER_VFS: &str = "vfs.sha256";

/// One metadata-only server-op audit line (`turn = 0`: server ops are not
/// model turns). `path` carries the op's *kind* (statement kind, virtual
/// path) — never statement text or file contents.
#[allow(clippy::too_many_arguments)]
pub(crate) fn audit_server_op(
    audit: &Audit,
    run_id: &str,
    tool: &str,
    op: &str,
    path: &str,
    bytes: Option<usize>,
    decision: &'static str,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: run_id.to_string(),
        tool: tool.to_string(),
        op: op.to_string(),
        path: path.to_string(),
        resolved: None,
        bytes,
        decision,
        reason: reason.to_string(),
        turn: 0,
    };
    if let Err(err) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {err}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_known_vectors() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn sha256_file_reports_digest_and_len() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("f.txt");
        std::fs::write(&path, b"abc").expect("write");
        let (digest, len) = sha256_file(&path).expect("hash");
        assert_eq!(digest, sha256_hex(b"abc"));
        assert_eq!(len, 3);
    }

    #[test]
    fn cache_leaf_is_under_the_virtual_tools_root() {
        // No env mutation (unit tests share the process): shape only.
        let leaf = cache_leaf("notes", "db");
        assert!(leaf.ends_with("virtual-tools/notes/db"));
        assert!(leaf.starts_with(crate::virtual_tools::audit::cache_dir()));
    }
}
