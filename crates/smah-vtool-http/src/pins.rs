//! Shared cache-pinning plumbing for the virtual-tools api (#20) and web
//! (#19) drivers: the tool-file sha256 staleness marker and markdown-fence
//! stripping for model output.

use std::path::Path;

use sha2::{Digest, Sha256};

/// Hex sha256 of a file's bytes — the cache staleness marker drivers pin
/// next to their generated artifacts. An unreadable file hashes to a stable
/// per-path fallback so the marker can never match a stale cache by
/// accident.
pub fn tool_sha256(path: &Path) -> String {
    match std::fs::read(path) {
        Ok(bytes) => {
            let digest = Sha256::digest(&bytes);
            digest.iter().map(|b| format!("{b:02x}")).collect()
        }
        Err(_) => format!("unreadable:{}", path.display()),
    }
}

/// Strip one wrapping markdown code fence, if the model added one despite
/// the "raw JSON only" instruction. Returns the trimmed inner text.
pub fn strip_fences(text: &str) -> &str {
    let trimmed = text.trim();
    let Some(after) = trimmed.strip_prefix("```") else {
        return trimmed;
    };
    // Skip the language tag on the opening line.
    let after = after
        .split_once('\n')
        .map(|(_, rest)| rest)
        .unwrap_or(after);
    let after = after.trim_start();
    match after.strip_suffix("```") {
        Some(inner) => inner.trim_end(),
        None => trimmed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_sha256_is_hex_and_stable() {
        let dir = tempfile::tempdir().expect("tempdir");
        let file = dir.path().join("tool.md");
        std::fs::write(&file, b"same bytes").expect("write");
        let first = tool_sha256(&file);
        assert_eq!(first, tool_sha256(&file));
        assert_eq!(first.len(), 64, "sha256 hex length");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));

        std::fs::write(&file, b"other bytes").expect("write");
        assert_ne!(first, tool_sha256(&file));

        // Unreadable path: stable fallback, never equal to a real digest.
        let missing = dir.path().join("missing.md");
        assert_eq!(tool_sha256(&missing), tool_sha256(&missing));
        assert!(tool_sha256(&missing).starts_with("unreadable:"));
    }

    #[test]
    fn strip_fences_variants() {
        assert_eq!(strip_fences("  {\"a\":1}  "), "{\"a\":1}");
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("```\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("{\"a\":1}\n"), "{\"a\":1}");
        // Unterminated fence: left as-is rather than half-stripped.
        assert_eq!(strip_fences("```json\n{\"a\":1}"), "```json\n{\"a\":1}");
    }
}
