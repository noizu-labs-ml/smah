//! Per-install audit HMAC key (`~/.smah/audit.key`, M1 follow-up to the
//! SW-07 review).
//!
//! smah-run's audit JSONL records every param value as a
//! `hmac:<16-hex>` prefix of an HMAC-SHA256 under this 32-byte key
//! (mode 0600). An UNKEYED hash prefix is brute-forceable offline for
//! low-entropy values (a 4-digit PIN, a short token); a keyed one is not —
//! without the key file there is no way to test candidate values against
//! the log, while within one install correlation still works (same value ⇒
//! same token). The key is load-or-created atomically on first use
//! (0700 dir, tmp write at 0600 + fsync + rename — the same
//! atomic-private-write pattern as the approval stamp). Any failure to
//! create or read the key is an `Err` — smah-run refuses to execute
//! (fail-closed, consistent with the audit-before-exec rule).

use std::path::{Path, PathBuf};

/// Key length: 32 bytes (256 bits) of OS randomness.
pub const KEY_LEN: usize = 32;

/// Default key path: `~/.smah/audit.key` (alongside `~/.smah/config.yaml`).
pub fn default_path() -> PathBuf {
    let mut path = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
    path.push(".smah");
    path.push("audit.key");
    path
}

/// Load the per-install audit key, creating it atomically on first use.
/// `Err` means "unusable" — the caller must fail closed.
pub fn load_or_create() -> Result<[u8; KEY_LEN], String> {
    load_or_create_at(&default_path())
}

/// [`load_or_create`] against an explicit path (the test seam).
pub fn load_or_create_at(path: &Path) -> Result<[u8; KEY_LEN], String> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() == KEY_LEN {
            let mut key = [0u8; KEY_LEN];
            key.copy_from_slice(&bytes);
            return Ok(key);
        }
        return Err(format!(
            "exists but is {} bytes, expected {KEY_LEN} — refusing to guess",
            bytes.len()
        ));
    }

    let key = random_key().ok_or_else(|| "cannot read an OS random source".to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("{} has no parent directory", path.display()))?;
    std::fs::create_dir_all(parent)
        .map_err(|err| format!("cannot create {}: {err}", parent.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Ok(meta) = std::fs::metadata(parent) {
            let mut perms = meta.permissions();
            perms.set_mode(0o700);
            let _ = std::fs::set_permissions(parent, perms);
        }
    }

    // Atomic private write: the tmp file is 0600 BEFORE any key bytes hit
    // disk, fsync'd, then renamed into place — a reader never sees a
    // partial key, and nothing world-readable ever exists.
    let tmp = path.with_extension("key.tmp");
    let mut file =
        open_new_private(&tmp).map_err(|err| format!("cannot write {}: {err}", tmp.display()))?;
    {
        use std::io::Write;
        file.write_all(&key)
            .and_then(|_| file.sync_data())
            .map_err(|err| format!("cannot write {}: {err}", tmp.display()))?;
    }
    drop(file);
    std::fs::rename(&tmp, path)
        .map_err(|err| format!("cannot finalize {}: {err}", path.display()))?;
    Ok(key)
}

/// Create a NEW private file for writing (0600 on unix — created before
/// any content is written; platform family matches smah-run's unix-first
/// process-group handling).
#[cfg(unix)]
fn open_new_private(path: &Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn open_new_private(path: &Path) -> std::io::Result<std::fs::File> {
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
}

/// HMAC-SHA256 (RFC 2104) over `data` under `key`, full 64-hex lowercase.
/// Implemented directly over `sha2` (no new dependency): the standard
/// ipad/opad construction with 64-byte blocks and key hashing above the
/// block size. smah-run truncates to the first 16 hex for audit tokens.
pub fn hmac_sha256_hex(key: &[u8], data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut k = [0u8; BLOCK];
    if key.len() > BLOCK {
        k[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; BLOCK];
    let mut opad = [0x5cu8; BLOCK];
    for (i, byte) in k.iter().enumerate() {
        ipad[i] ^= byte;
        opad[i] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner);
    outer
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// 32 bytes from the OS RNG (`/dev/urandom` on unix; unix-only platform
/// family, matching smah-run's `cfg(unix)` process-group handling).
fn random_key() -> Option<[u8; KEY_LEN]> {
    #[cfg(unix)]
    {
        use std::io::Read;
        let mut file = std::fs::File::open("/dev/urandom").ok()?;
        let mut key = [0u8; KEY_LEN];
        file.read_exact(&mut key).ok()?;
        Some(key)
    }
    #[cfg(not(unix))]
    {
        None // no random source — callers fail closed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// RFC 4231 test case 1 (key = 20×0x0b, data = "Hi There").
    #[test]
    fn hmac_matches_rfc4231_vector() {
        let key = [0x0bu8; 20];
        let mac = hmac_sha256_hex(&key, b"Hi There");
        assert_eq!(
            mac,
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    /// RFC 4231 test case 2 (short key "Jefe", longer data).
    #[test]
    fn hmac_matches_rfc4231_vector_two() {
        let mac = hmac_sha256_hex(b"Jefe", b"what do ya want for nothing?");
        assert_eq!(
            mac,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    /// A key longer than the 64-byte block is hashed, not truncated
    /// (RFC 4231 test case 6: 131-byte 0xaa key over its data string).
    #[test]
    fn hmac_hashes_long_keys() {
        let key = [0xaau8; 131];
        let data = b"Test Using Larger Than Block-Size Key - Hash Key First";
        let mac = hmac_sha256_hex(&key, data);
        assert_eq!(
            mac,
            "60e431591ee0b67f0d8a26aacbf5b77f8e0bc6213728c5140546040f0ee37f54"
        );
    }

    #[test]
    fn load_or_create_is_atomic_private_and_stable() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join(".smah").join("audit.key");

        let first = load_or_create_at(&path).expect("created");
        assert_eq!(first.len(), KEY_LEN, "32 bytes of key material");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "key file is 0600");
            let dir_mode = std::fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(dir_mode & 0o777, 0o700, "key directory is 0700");
        }

        // Second load returns the SAME key — the file is authoritative.
        let second = load_or_create_at(&path).expect("reloaded");
        assert_eq!(first, second, "key is stable across loads");
        // No tmp artifact left behind.
        assert!(!path.with_extension("key.tmp").exists(), "tmp renamed away");
    }

    #[test]
    fn corrupted_key_refuses() {
        // Fail closed: a wrong-length key file is an error, never a
        // silently different token space.
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("audit.key");
        std::fs::write(&path, vec![0u8; 8]).expect("short key");
        assert!(load_or_create_at(&path).is_err(), "8-byte key refused");
    }

    #[test]
    fn tokens_correlate_within_an_install() {
        let key = load_or_create_at(&tempfile::tempdir().expect("tmp").path().join("audit.key"))
            .expect("key");
        let a = &hmac_sha256_hex(&key, b"hunter2")[..16];
        let b = &hmac_sha256_hex(&key, b"hunter2")[..16];
        let c = &hmac_sha256_hex(&key, b"hunter3")[..16];
        assert_eq!(a, b, "same value ⇒ same 16-hex token");
        assert_ne!(a, c, "different value ⇒ different token");
    }
}
