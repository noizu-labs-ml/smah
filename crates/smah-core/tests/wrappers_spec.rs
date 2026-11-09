//! SW-01 integration spec: wrapper-file parsing and discovery against
//! tempdir fixtures (never the real `~/.config` — the dir seam is explicit).

use std::collections::HashSet;
use std::sync::{Mutex, MutexGuard, OnceLock};

use smah_core::wrappers::loader::{discover_in, load_by_name, load_with_tool_names, wrappers_dir};

/// Serializes the env-mutating tests (one process, parallel threads).
fn env_lock() -> MutexGuard<'static, ()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

fn write(dir: &std::path::Path, name: &str, content: &str) -> std::path::PathBuf {
    use std::io::Write;
    let mut f = std::fs::File::create(dir.join(format!("{name}.md"))).expect("create");
    f.write_all(content.as_bytes()).expect("write");
    dir.join(format!("{name}.md"))
}

/// A maximal, realistic wrapper: params with defaults, a `use` step with
/// `with` bindings, a `run` step with cwd/timeout/output, declared outputs,
/// preconditions, approval, model, behavior prompt, provenance comment.
const FULL: &str = r#"---
# provenance: 2026-11-09T10:00:00-08:00 model=anthropic.claude-fixture-1 run=00000000-0000-4000-8000-000000000000 prompt_sha256=abc
name: session-transcript
description: Turn an asciinema session recording into a text transcript
model: anthropic.claude-fixture-1
behavior_prompt: |
  You are a terminal-session assistant. You adjust transcript recipes.
approved:
  at: 2026-11-09T12:00:00-08:00
  sha256: 0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
params:
  - name: cast_file
    description: Path to the .cast recording
    required: true
  - name: out_label
    description: Label printed before the transcript
    default: transcript
steps:
  - id: to-text
    use: cast-to-text
    description: Convert cast events to a text transcript
    with:
      cast_file: "{{params.cast_file}}"
    output: transcript_text
  - id: label
    run: ["echo", "{{params.out_label}}:", "{{steps.to-text.output}}"]
    cwd: "/tmp"
    timeout_secs: 30
outputs:
  - name: transcript_text
    description: Plain-text transcript of the session
preconditions:
  - "{{params.cast_file}} must exist and be readable"
---
# session-transcript

Recipe body (documentation).
"#;

#[test]
fn full_wrapper_parses_with_every_field() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = write(dir.path(), "session-transcript", FULL);
    let def = load_with_tool_names(&path, &HashSet::new()).expect("loads");
    assert_eq!(def.name, "session-transcript");
    assert_eq!(def.model.as_deref(), Some("anthropic.claude-fixture-1"));
    assert!(def.approved.is_some());
    assert_eq!(def.params.len(), 2);
    assert!(def.params[0].required);
    assert!(!def.params[1].required);
    assert_eq!(def.params[1].default.as_deref(), Some("transcript"));
    assert_eq!(def.steps.len(), 2);
    assert_eq!(def.steps[0].use_.as_deref(), Some("cast-to-text"));
    assert_eq!(
        def.steps[0].with,
        vec![("cast_file".to_string(), "{{params.cast_file}}".to_string())]
    );
    assert_eq!(def.steps[0].output.as_deref(), Some("transcript_text"));
    assert_eq!(
        def.steps[1].run,
        Some(vec![
            "echo".to_string(),
            "{{params.out_label}}:".to_string(),
            "{{steps.to-text.output}}".to_string()
        ])
    );
    assert_eq!(def.steps[1].cwd.as_deref(), Some("/tmp"));
    assert_eq!(def.steps[1].timeout_secs, Some(30));
    assert_eq!(def.outputs.len(), 1);
    assert_eq!(def.preconditions.len(), 1);
    assert!(def.body.contains("Recipe body"));
}

#[test]
fn discover_reports_valid_and_invalid_rows() {
    let dir = tempfile::tempdir().expect("tmp");
    write(
        dir.path(),
        "good",
        "---\nname: good\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n",
    );
    write(
        dir.path(),
        "bad",
        "---\nname: bad\ndescription: d\nsteps: []\n---\n",
    );
    let (valid, invalid) = discover_in(dir.path());
    assert_eq!(valid.len(), 1);
    assert_eq!(valid[0].name, "good");
    assert_eq!(invalid.len(), 1);
    assert_eq!(invalid[0].name, "bad");
    assert!(invalid[0].reason.contains("at least one step"));
}

#[test]
fn discover_missing_dir_is_empty_not_error() {
    let dir = tempfile::tempdir().expect("tmp");
    let (valid, invalid) = discover_in(&dir.path().join("nope"));
    assert!(valid.is_empty());
    assert!(invalid.is_empty());
}

#[test]
fn load_by_name_missing_names_installed_set() {
    let _guard = env_lock();
    // Point the dir seam at an isolated tempdir via the process env; restore
    // before asserting.
    let dir = tempfile::tempdir().expect("tmp");
    std::env::set_var("SMAH_WRAPPERS_DIR", dir.path());
    let err = load_by_name("ghost").expect_err("missing");
    std::env::remove_var("SMAH_WRAPPERS_DIR");
    assert!(err.starts_with("not found:"), "got: {err}");
    assert!(err.contains("Installed: (none)"));
}

#[test]
fn load_by_name_resolves_installed_wrapper() {
    let _guard = env_lock();
    let dir = tempfile::tempdir().expect("tmp");
    write(
        dir.path(),
        "demo",
        "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n",
    );
    std::env::set_var("SMAH_WRAPPERS_DIR", dir.path());
    let def = load_by_name("demo");
    std::env::remove_var("SMAH_WRAPPERS_DIR");
    assert_eq!(def.expect("loads").name, "demo");
}

#[test]
fn wrappers_dir_env_seam_takes_precedence() {
    let _guard = env_lock();
    std::env::set_var("SMAH_WRAPPERS_DIR", "/tmp/wrappers-seam-test");
    assert_eq!(
        wrappers_dir(),
        std::path::PathBuf::from("/tmp/wrappers-seam-test")
    );
    std::env::remove_var("SMAH_WRAPPERS_DIR");
    let fallback = wrappers_dir();
    assert!(
        fallback.ends_with("smah/wrappers"),
        "fallback dir: {}",
        fallback.display()
    );
}

#[test]
fn reserved_word_wrapper_is_rejected_as_wrapper_name() {
    let dir = tempfile::tempdir().expect("tmp");
    let path = write(
        dir.path(),
        "wrapper",
        "---\nname: wrapper\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n",
    );
    assert!(load_with_tool_names(&path, &HashSet::new())
        .unwrap_err()
        .contains("reserved"));
}
