//! T-001 class fixture (plan §G M1): a registered secret must never reach
//! the log file — not in a plain DEBUG payload, not in a settings-style
//! save dump (the exact vector THREAT-MODEL T-001 documents).

use std::fs;
use std::path::PathBuf;

const FAKE_KEY: &str = "sk-t001-deadbeef-not-a-real-key";
const FAKE_KEY_ALT: &str = "sk-t001-feedface-secondary";

fn log_files(dir: &std::path::Path) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .expect("logs dir exists")
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .filter(|path| {
            path.file_name()
                .map(|n| {
                    let name = n.to_string_lossy();
                    name.starts_with("smah.") && name.ends_with(".log")
                })
                .unwrap_or(false)
        })
        .collect();
    paths.sort();
    paths
}

#[test]
fn t001_registered_secret_never_reaches_log() {
    let dir = tempfile::tempdir().expect("tempdir");
    let log_path = smah_core::logging::init_with_dir(dir.path()).expect("logging init");

    // Register before any payload flows (registration precedes logging).
    smah_core::logging::register_secret(FAKE_KEY);
    smah_core::logging::register_secret(FAKE_KEY_ALT);

    // 1. Plain DEBUG payload carrying the key.
    tracing::debug!("resolved provider key={}", FAKE_KEY);

    // 2. Settings-style save dump — the T-001 vector (Settings.log() parity).
    let dump = format!(
        "vsn: 0.0.1\nuser:\n  name: Keith\ninference:\n  providers:\n    anthropic:\n      settings:\n        api_key: {}\n      models:\n        - name: claude-haiku-5-5\n          key: {}\n",
        FAKE_KEY, FAKE_KEY_ALT
    );
    tracing::debug!("Settings YAML: {}", dump);

    // Both writes are synchronous through the subscriber; the file is ready.
    let files = log_files(dir.path());
    assert!(!files.is_empty(), "per-run log file must exist");
    assert!(
        files.iter().any(|p| p == &log_path),
        "returned path must be the active log file"
    );

    let content = fs::read_to_string(&log_path).expect("log readable");
    assert!(
        content.contains("Settings YAML:"),
        "the dump payload must have reached the file"
    );
    assert!(
        content.contains("***"),
        "secrets must be rewritten to *** in the file"
    );
    assert!(!content.contains(FAKE_KEY), "primary key leaked to log");
    assert!(
        !content.contains(FAKE_KEY_ALT),
        "secondary key leaked to log"
    );

    // The redacted form preserves the surrounding payload shape.
    assert!(
        content.contains("api_key: ***"),
        "expected the masked mapping line, got: {}",
        content
    );
}

#[test]
fn redact_is_idempotent_and_masks_all_occurrences() {
    smah_core::logging::register_secret("dup-secret-42");
    let once = smah_core::logging::redact("a dup-secret-42 b dup-secret-42 c");
    assert_eq!(once, "a *** b *** c");
    let twice = smah_core::logging::redact(&once);
    assert_eq!(twice, once, "mask output must not be re-mangled");
}

#[test]
fn empty_secrets_are_ignored() {
    smah_core::logging::register_secret("");
    smah_core::logging::register_secret("   ");
    // An empty needle would corrupt every write; verify it did not register.
    assert_eq!(smah_core::logging::redact("untouched"), "untouched");
}
