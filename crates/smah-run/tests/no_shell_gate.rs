//! The no-shell gate contract, tested from the crate whose existence
//! created the exception (SW-05): every crate except `smah-run` stays
//! gated, and the exemption is scoped to the exact path — not the pattern.
//!
//! These tests run `scripts/no-shell-gate.sh` against synthetic trees:
//! a real workspace member with an offender must FAIL the gate; the
//! smah-run path with the same offender must PASS; the real workspace must
//! be clean.

use std::path::PathBuf;
use std::process::Command as StdCommand;

fn gate_script() -> PathBuf {
    // CARGO_MANIFEST_DIR = crates/smah-run
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/no-shell-gate.sh")
}

fn run_gate(root: &std::path::Path) -> (i32, String) {
    let out = StdCommand::new("sh")
        .arg(gate_script())
        .arg(root)
        .output()
        .expect("run gate script");
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

fn fake_workspace(offender_crate: &str) -> PathBuf {
    let dir = tempfile::tempdir().expect("tmp workspace");
    let src = dir.path().join("crates").join(offender_crate).join("src");
    std::fs::create_dir_all(&src).expect("crates dir");
    std::fs::write(
        src.join("lib.rs"),
        "fn main() { let _ = std::process::Command::new(\"ls\"); }\n",
    )
    .expect("offender fixture");
    dir.keep()
}

#[test]
fn other_crates_with_spawns_fail_the_gate() {
    let root = fake_workspace("smah-core");
    let (code, stderr) = run_gate(&root);
    assert_eq!(code, 1, "an offending non-runner crate must fail: {stderr}");
    assert!(stderr.contains("spawns processes"), "{stderr}");
}

#[test]
fn the_smah_run_exception_is_scoped_to_its_exact_path() {
    // The same offender under crates/smah-run/src is the sanctioned case.
    let root = fake_workspace("smah-run");
    let (code, stderr) = run_gate(&root);
    assert_eq!(code, 0, "smah-run/src is the exempted path: {stderr}");
}

#[test]
fn lookalike_names_are_not_exempt() {
    // The exemption matches the exact path, not a substring.
    let root = fake_workspace("smah-runner");
    let (code, _) = run_gate(&root);
    assert_eq!(code, 1, "smah-runner must NOT inherit the exemption");
}

#[test]
fn the_real_workspace_is_clean() {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
    let (code, stderr) = run_gate(&root);
    assert_eq!(code, 0, "the workspace must pass its own gate: {stderr}");
}
