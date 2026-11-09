//! Secret stores (RA-10 F2, owner decision 2026-11-10): the operator's
//! provider API key never lives in `config.yaml` as a literal. The config
//! stores a **reference**; the key itself lives in one of:
//!
//! * the **OS secret store** — macOS Keychain (Security.framework via
//!   security-framework FFI), Windows Credential Manager (Win32 FFI), or on
//!   Linux/BSD the freedesktop Secret Service over **pure-Rust D-Bus**
//!   (`secret-service` crate atop `zbus`: a userspace D-Bus wire-protocol
//!   implementation over unix sockets — no libdbus, no libsecret, and no
//!   helper process; see the Cargo.toml note and docs/THREAT-MODEL.md for
//!   the no-spawn citation),
//! * a **fallback file** (`secrets.yaml` beside the config, created 0600
//!   atomically) when no OS store is reachable (headless Linux with no D-Bus
//!   session bus), or
//! * the **environment** (`env:VAR` reference; the variable must be set at
//!   run time).
//!
//! Reference syntax stored in `settings.api_key`:
//!
//! * `keychain:smah/<provider>` — OS store, service `smah`, account
//!   `<provider>`
//! * `file:secrets.yaml#<provider>` — entry `<provider>` in the named file
//!   (resolved relative to the config's directory; no absolute paths, no
//!   `..` components)
//! * `env:<VAR>` — environment variable
//!
//! `$VAR` (the RA-06 legacy spelling) keeps resolving everywhere; new writes
//! use `env:<VAR>`. A literal key still resolves (Python-era configs) —
//! `smah doctor` warns and `smah config migrate-secrets` moves it.
//!
//! Print-only invariant: none of the backends spawn a process. Every store
//! call happens in plain sync context (never inside an async runtime).
//!
//! [`write_private_file_atomic`] and [`create_private_file`] back every
//! private-file write in the tree (config, secrets file, session DB, edit
//! handoff): the file is created with owner-only permissions **from the
//! first byte** (temp file 0600 in the same directory, fsync, rename) —
//! there is no window where a secret sits in a world-readable file (F1).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Service name for OS-store entries (`keychain:smah/<provider>`).
pub const SERVICE: &str = "smah";

/// The secrets-file name beside the config (`file:secrets.yaml#<provider>`).
pub const SECRETS_FILE: &str = "secrets.yaml";

/// Which physical backend backs a [`SecretStore`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreKind {
    /// The OS keychain / credential manager / Secret Service (per OS).
    Keyring,
    /// The 0600 fallback file beside the config.
    File,
    /// Dispatch-by-reference (the runtime seam — [`RuntimeStore`]); never a
    /// chosen init backend.
    Runtime,
}

/// Where a key lives, parsed from a `settings.api_key` value. `None` from
/// [`parse_secret_ref`] means the value is a literal (or legacy `$VAR`,
/// which the settings layer resolves itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SecretRef {
    /// `env:<VAR>` — resolve through the environment at run time.
    EnvVar(String),
    /// `keychain:<service>/<account>` — an OS-store entry.
    Keyring { service: String, account: String },
    /// `file:<name>#<entry>` — an entry in a secrets file (path already
    /// resolved against the config directory).
    File { path: PathBuf, name: String },
}

impl SecretRef {
    /// The canonical string stored in `config.yaml`.
    pub fn encode(&self) -> String {
        match self {
            SecretRef::EnvVar(name) => format!("env:{name}"),
            SecretRef::Keyring { service, account } => format!("keychain:{service}/{account}"),
            SecretRef::File { path, name } => {
                let base = path
                    .file_name()
                    .map(|b| b.to_string_lossy().to_string())
                    .unwrap_or_default();
                format!("file:{base}#{name}")
            }
        }
    }
}

/// Parse a `settings.api_key` value into a reference. `config_dir` anchors
/// `file:` references (the config file's own directory). Returns `None` for
/// literals, legacy `$VAR`, and anything not matching the three shapes.
pub fn parse_secret_ref(field: &str, config_dir: &Path) -> Option<SecretRef> {
    if let Some(name) = field.strip_prefix("env:") {
        if is_env_name(name) {
            return Some(SecretRef::EnvVar(name.to_string()));
        }
        return None;
    }
    if let Some(rest) = field.strip_prefix("keychain:") {
        if let Some((service, account)) = rest.split_once('/') {
            if !service.is_empty() && !account.is_empty() {
                return Some(SecretRef::Keyring {
                    service: service.to_string(),
                    account: account.to_string(),
                });
            }
        }
        return None;
    }
    if let Some(rest) = field.strip_prefix("file:") {
        let (file_part, name) = rest.split_once('#')?;
        if name.is_empty() || file_part.is_empty() {
            return None;
        }
        let candidate = Path::new(file_part);
        if candidate.is_absolute()
            || candidate
                .components()
                .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return None;
        }
        return Some(SecretRef::File {
            path: config_dir.join(candidate),
            name: name.to_string(),
        });
    }
    None
}

/// Restrictive env-name check (`env:<VAR>` must name a real variable).
fn is_env_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(first) if first.is_ascii_alphabetic() || first == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The reference a chosen backend stores for `provider` (config init and
/// migrate-secrets write exactly this).
pub fn store_reference(kind: StoreKind, provider: &str, config_dir: &Path) -> SecretRef {
    match kind {
        StoreKind::Keyring => SecretRef::Keyring {
            service: SERVICE.to_string(),
            account: provider.to_string(),
        },
        StoreKind::File => SecretRef::File {
            path: config_dir.join(SECRETS_FILE),
            name: provider.to_string(),
        },
        StoreKind::Runtime => unreachable!("Runtime is a dispatch seam, not a backend"),
    }
}

/// One secret-store backend. Errors are single-line details (callers wrap
/// them in [`smah_core::errors::SmahError`]).
pub trait SecretStore {
    fn kind(&self) -> StoreKind;
    /// Human description for wizard output ("macOS Keychain", …).
    fn describe(&self) -> String;
    fn get(&self, reference: &SecretRef) -> Result<Option<String>, String>;
    fn set(&self, reference: &SecretRef, secret: &str) -> Result<(), String>;
    fn delete(&self, reference: &SecretRef) -> Result<(), String>;
}

/// The OS-store backend: macOS Keychain, Windows Credential Manager, or the
/// Secret Service (pure-Rust zbus D-Bus) per platform — selected by the
/// target-specific `keyring` features in Cargo.toml. Only serves
/// [`SecretRef::Keyring`] references.
pub struct KeyringStore;

impl SecretStore for KeyringStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Keyring
    }

    fn describe(&self) -> String {
        #[cfg(target_os = "macos")]
        {
            "macOS Keychain".to_string()
        }
        #[cfg(target_os = "windows")]
        {
            "Windows Credential Manager".to_string()
        }
        #[cfg(all(unix, not(target_os = "macos")))]
        {
            "freedesktop Secret Service (D-Bus keyring)".to_string()
        }
        #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
        {
            "OS secret store".to_string()
        }
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, String> {
        let entry = self.entry(reference)?;
        match entry.get_password() {
            Ok(secret) => Ok(Some(secret)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(err) => Err(err.to_string()),
        }
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<(), String> {
        self.entry(reference)?
            .set_password(secret)
            .map_err(|err| err.to_string())
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), String> {
        match self.entry(reference)?.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(err) => Err(err.to_string()),
        }
    }
}

impl KeyringStore {
    fn entry(&self, reference: &SecretRef) -> Result<keyring::Entry, String> {
        match reference {
            SecretRef::Keyring { service, account } => keyring::Entry::new(service, account)
                .map_err(|err| format!("cannot open OS secret store: {err}")),
            other => Err(format!(
                "the OS secret store can only serve keychain references, not '{}'",
                other.encode()
            )),
        }
    }
}

/// The 0600 fallback file backend (`secrets.yaml` beside the config): a
/// YAML mapping of entry name → secret. Created/replaced atomically with
/// owner-only permissions — on Windows the profile ACL applies (documented
/// in docs/THREAT-MODEL.md).
pub struct FileStore {
    /// Default location (used for `describe`); references carry their own
    /// resolved path.
    pub path: PathBuf,
}

impl FileStore {
    /// The resolved secrets-file path for a reference (its own path).
    fn path_for(reference: &SecretRef) -> Result<PathBuf, String> {
        match reference {
            SecretRef::File { path, .. } => Ok(path.clone()),
            other => Err(format!(
                "the file store can only serve file references, not '{}'",
                other.encode()
            )),
        }
    }

    fn load(path: &Path) -> Result<serde_yaml_ng::Mapping, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Default::default()),
            Err(err) => return Err(format!("cannot read {}: {err}", path.display())),
        };
        serde_yaml_ng::from_str::<serde_yaml_ng::Mapping>(&text)
            .map_err(|err| format!("{} is not a valid secrets file: {err}", path.display()))
    }

    fn store(path: &Path, entries: &serde_yaml_ng::Mapping) -> Result<(), String> {
        let text =
            serde_yaml_ng::to_string(entries).map_err(|err| format!("secrets encode: {err}"))?;
        write_private_file_atomic(path, text.as_bytes())
            .map_err(|err| format!("cannot write {} (0600): {err}", path.display()))
    }

    fn key(name: &str) -> serde_yaml_ng::Value {
        serde_yaml_ng::Value::String(name.to_string())
    }
}

impl SecretStore for FileStore {
    fn kind(&self) -> StoreKind {
        StoreKind::File
    }

    fn describe(&self) -> String {
        format!("secrets file {} (created 0600)", self.path.display())
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, String> {
        let path = Self::path_for(reference)?;
        let name = match reference {
            SecretRef::File { name, .. } => name.clone(),
            _ => unreachable!("path_for validated the kind"),
        };
        let entries = Self::load(&path)?;
        match entries.get(Self::key(&name)) {
            Some(serde_yaml_ng::Value::String(secret)) => Ok(Some(secret.clone())),
            Some(_) => Err(format!(
                "entry '{name}' in {} is not a string",
                path.display()
            )),
            None => Ok(None),
        }
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<(), String> {
        let path = Self::path_for(reference)?;
        let name = match reference {
            SecretRef::File { name, .. } => name.clone(),
            _ => unreachable!("path_for validated the kind"),
        };
        let mut entries = Self::load(&path)?;
        entries.insert(
            Self::key(&name),
            serde_yaml_ng::Value::String(secret.to_string()),
        );
        Self::store(&path, &entries)
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), String> {
        let path = Self::path_for(reference)?;
        let name = match reference {
            SecretRef::File { name, .. } => name.clone(),
            _ => unreachable!("path_for validated the kind"),
        };
        let mut entries = Self::load(&path)?;
        entries.remove(Self::key(&name));
        Self::store(&path, &entries)
    }
}

/// Test seam: an in-memory store keyed by [`SecretRef::encode`]. Never
/// touches a real keychain (CI-safe).
#[derive(Debug, Default)]
pub struct MockStore {
    entries: RefCell<BTreeMap<String, String>>,
}

impl MockStore {
    pub fn new() -> Self {
        Self::default()
    }

    /// Pre-seed one entry (tests arrange store state without real OS calls).
    pub fn with_entry(self, reference: &SecretRef, secret: &str) -> Self {
        self.entries
            .borrow_mut()
            .insert(reference.encode(), secret.to_string());
        self
    }
}

impl SecretStore for MockStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Runtime
    }

    fn describe(&self) -> String {
        "mock secret store (tests)".to_string()
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, String> {
        Ok(self.entries.borrow().get(&reference.encode()).cloned())
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<(), String> {
        self.entries
            .borrow_mut()
            .insert(reference.encode(), secret.to_string());
        Ok(())
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), String> {
        self.entries.borrow_mut().remove(&reference.encode());
        Ok(())
    }
}

/// The runtime seam: dispatches each reference to its physical backend
/// (keychain refs → [`KeyringStore`], file refs → [`FileStore`]). Env
/// references resolve through the environment in the credentials layer, not
/// here. Tests inject [`MockStore`] instead of touching real OS stores.
pub struct RuntimeStore;

impl SecretStore for RuntimeStore {
    fn kind(&self) -> StoreKind {
        StoreKind::Runtime
    }

    fn describe(&self) -> String {
        "resolved secret store".to_string()
    }

    fn get(&self, reference: &SecretRef) -> Result<Option<String>, String> {
        match reference {
            SecretRef::Keyring { .. } => KeyringStore.get(reference),
            SecretRef::File { .. } => FileStore {
                path: PathBuf::new(),
            }
            .get(reference),
            SecretRef::EnvVar(_) => Err("env references resolve through the environment".into()),
        }
    }

    fn set(&self, reference: &SecretRef, secret: &str) -> Result<(), String> {
        match reference {
            SecretRef::Keyring { .. } => KeyringStore.set(reference, secret),
            SecretRef::File { .. } => FileStore {
                path: PathBuf::new(),
            }
            .set(reference, secret),
            SecretRef::EnvVar(_) => {
                Err("env references are not stored — export the variable".into())
            }
        }
    }

    fn delete(&self, reference: &SecretRef) -> Result<(), String> {
        match reference {
            SecretRef::Keyring { .. } => KeyringStore.delete(reference),
            SecretRef::File { .. } => FileStore {
                path: PathBuf::new(),
            }
            .delete(reference),
            SecretRef::EnvVar(_) => Ok(()),
        }
    }
}

/// Whether the OS store is reachable on this platform.
///
/// * macOS / Windows: the OS credential APIs are always present.
/// * Linux/BSD: the Secret Service needs a D-Bus session bus — a documented
///   heuristic (env `DBUS_SESSION_BUS_ADDRESS`, or the standard
///   `$XDG_RUNTIME_DIR/bus` socket). A store call that still fails errors
///   loudly rather than silently degrading (propagation over silent
///   fallback — F1 discipline).
pub fn keyring_available() -> bool {
    #[cfg(target_os = "macos")]
    {
        true
    }
    #[cfg(target_os = "windows")]
    {
        true
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        if std::env::var_os("DBUS_SESSION_BUS_ADDRESS")
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            return true;
        }
        std::env::var_os("XDG_RUNTIME_DIR")
            .map(|dir| PathBuf::from(dir).join("bus").exists())
            .unwrap_or(false)
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows", unix)))]
    {
        false
    }
}

/// The best init backend for this platform, with the human reason the
/// wizard prints. Linux without a session bus falls back to the 0600 file.
pub fn best_available_store_kind() -> (StoreKind, String) {
    if keyring_available() {
        (StoreKind::Keyring, KeyringStore.describe())
    } else {
        (
            StoreKind::File,
            "no OS secret store reachable (headless? no D-Bus session bus) — using the 0600 secrets file".to_string(),
        )
    }
}

/// Create `path` with owner-only permissions from the first byte if it does
/// not exist yet (the session DB pre-create; F6). Existing files keep their
/// permissions. On Windows the profile ACL applies (no POSIX modes).
pub fn create_private_file(path: &Path) -> std::io::Result<()> {
    if path.exists() {
        return Ok(());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)?
            .sync_all()?;
        Ok(())
    }
    #[cfg(not(unix))]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?
            .sync_all()?;
        Ok(())
    }
}

/// Write `data` to `path` so the content is **never readable by anyone but
/// the owner** (F1): a temp file is created in the same directory with mode
/// 0600 from the first byte (unix `OpenOptions::mode(0o600)`), written,
/// fsynced, then renamed over the target — atomic on unix, no window where
/// the secret sits in a world-readable file. On Windows there are no POSIX
/// modes: the temp file inherits the user-profile ACLs of its directory,
/// which is the documented platform posture. The directory is fsynced after
/// the rename so the replacement itself is durable. All errors propagate —
/// callers must not claim success on a failed write.
pub fn write_private_file_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(dir)?;
    let name = path
        .file_name()
        .ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidInput, "path has no file name")
        })?
        .to_string_lossy()
        .to_string();
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = dir.join(format!(".{name}.tmp-{}-{nanos}", std::process::id()));

    let result = (|| -> std::io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&tmp)?;
            file.write_all(data)?;
            file.sync_all()?;
            drop(file);
            std::fs::rename(&tmp, path)?;
        }
        #[cfg(not(unix))]
        {
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&tmp)?;
            file.write_all(data)?;
            file.sync_all()?;
            drop(file);
            // Windows rename fails if the target exists — replace explicitly.
            if path.exists() {
                std::fs::remove_file(path)?;
            }
            std::fs::rename(&tmp, path)?;
        }
        #[cfg(unix)]
        {
            // Make the rename itself durable (best effort — content is
            // already fsynced; a directory-fsync failure must not fail the
            // write that succeeded).
            if let Ok(dir_file) = std::fs::File::open(dir) {
                let _ = dir_file.sync_all();
            }
        }
        Ok(())
    })();

    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refs_round_trip_through_encode_and_parse() {
        let dir = Path::new("/home/op/.smah");
        for encoded in [
            "env:SMAH_ANTHROPIC_API_KEY",
            "keychain:smah/anthropic",
            "file:secrets.yaml#anthropic",
        ] {
            let parsed = parse_secret_ref(encoded, dir).unwrap_or_else(|| panic!("{encoded}"));
            assert_eq!(parsed.encode(), encoded, "round trip: {encoded}");
        }
    }

    #[test]
    fn file_refs_resolve_against_the_config_dir() {
        let dir = Path::new("/home/op/.smah");
        match parse_secret_ref("file:secrets.yaml#anthropic", dir) {
            Some(SecretRef::File { path, name }) => {
                assert_eq!(path, dir.join("secrets.yaml"));
                assert_eq!(name, "anthropic");
            }
            other => panic!("expected file ref, got {other:?}"),
        }
    }

    #[test]
    fn hostile_file_refs_are_rejected() {
        let dir = Path::new("/home/op/.smah");
        for bad in [
            "file:/etc/passwd#x",
            "file:../secrets.yaml#x",
            "file:secrets.yaml#",
            "file:#x",
            "env:",
            "env:9BAD",
            "env:HAS SPACE",
            "keychain:smah/",
            "keychain:/anthropic",
            "plain-literal-key",
            "$SMAH_ANTHROPIC_API_KEY",
        ] {
            assert!(parse_secret_ref(bad, dir).is_none(), "rejected: {bad}");
        }
    }

    #[test]
    fn store_reference_shapes_match_the_documented_syntax() {
        let dir = Path::new("/home/op/.smah");
        assert_eq!(
            store_reference(StoreKind::Keyring, "anthropic", dir).encode(),
            "keychain:smah/anthropic"
        );
        assert_eq!(
            store_reference(StoreKind::File, "anthropic", dir).encode(),
            "file:secrets.yaml#anthropic"
        );
    }

    #[test]
    fn file_store_round_trip_is_private_and_atomic() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("secrets.yaml");
        let store = FileStore { path: path.clone() };
        let reference = SecretRef::File {
            path: path.clone(),
            name: "anthropic".to_string(),
        };
        store.set(&reference, "test-key-0000").expect("set");
        assert_eq!(
            store.get(&reference).expect("get").as_deref(),
            Some("test-key-0000")
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "secrets file is owner-only");
        }
        // Second entry preserves the first; delete removes it.
        let other = SecretRef::File {
            path: path.clone(),
            name: "alibaba".to_string(),
        };
        store.set(&other, "k2").expect("set 2");
        assert_eq!(
            store.get(&reference).expect("get"),
            Some("test-key-0000".to_string())
        );
        store.delete(&reference).expect("delete");
        assert_eq!(store.get(&reference).expect("get"), None);
        assert_eq!(store.get(&other).expect("get"), Some("k2".to_string()));
    }

    #[test]
    fn file_store_missing_file_reads_as_empty() {
        let dir = tempfile::tempdir().expect("tmp");
        let store = FileStore {
            path: dir.path().join("absent.yaml"),
        };
        let reference = SecretRef::File {
            path: dir.path().join("absent.yaml"),
            name: "x".to_string(),
        };
        assert_eq!(store.get(&reference).expect("get"), None);
    }

    #[test]
    fn mock_store_round_trip() {
        let store = MockStore::new();
        let reference = SecretRef::Keyring {
            service: "smah".to_string(),
            account: "anthropic".to_string(),
        };
        assert_eq!(store.get(&reference).expect("get"), None);
        store.set(&reference, "test-key-0000").expect("set");
        assert_eq!(
            store.get(&reference).expect("get"),
            Some("test-key-0000".to_string())
        );
        store.delete(&reference).expect("delete");
        assert_eq!(store.get(&reference).expect("get"), None);
    }

    #[cfg(unix)]
    #[test]
    fn atomic_write_is_owner_only_from_the_start() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("config.yaml");
        write_private_file_atomic(&path, b"hello").expect("write");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "created 0600, never world-readable");
        assert_eq!(std::fs::read(&path).expect("read"), b"hello");
        // Replacement keeps the discipline.
        write_private_file_atomic(&path, b"second").expect("rewrite");
        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "replacement is 0600 too");
        // No temp litter left behind.
        let litter: Vec<_> = std::fs::read_dir(dir.path())
            .expect("dir")
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .collect();
        assert_eq!(litter, vec!["config.yaml".to_string()], "temp cleaned up");
    }

    #[test]
    fn atomic_write_errors_propagate() {
        // Parent is a regular file — create_dir_all must fail, and the
        // error must reach the caller (no swallowed chmod, F1).
        let dir = tempfile::tempdir().expect("tmp");
        let blocker = dir.path().join("blocker");
        std::fs::write(&blocker, b"x").expect("file");
        let path = blocker.join("config.yaml");
        assert!(
            write_private_file_atomic(&path, b"x").is_err(),
            "error propagates"
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_create_is_0600_and_idempotent() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("smah.db");
        create_private_file(&path).expect("create");
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode();
        assert_eq!(mode & 0o777, 0o600, "DB created owner-only");
        // Existing files are left alone (both perms and content).
        std::fs::write(&path, b"data").expect("fill");
        create_private_file(&path).expect("second open is a no-op");
        assert_eq!(std::fs::read(&path).expect("read"), b"data");
    }
}
