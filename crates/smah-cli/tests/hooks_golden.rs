//! Hooks, binary-driven: empty/missing tools dir, envrc output, shim sync,
//! stdout purity (plan W5). The zsh/bash snippet goldens live as unit tests
//! in `src/hooks.rs` (smah-cli is a binary crate — integration tests cannot
//! import its modules).

use assert_cmd::Command;

#[test]
fn empty_tool_set_is_an_empty_snippet_with_warning_exit_zero() {
    let dir = tempfile::tempdir().expect("tmp");
    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", dir.path())
        .args(["hooks", "zsh"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    // Header comment only — no functions.
    assert_eq!(stdout.lines().count(), 1, "{stdout}");
    assert!(stdout.starts_with("# smah virtual tools — generated"));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("no virtual tools installed"), "{stderr}");
}

#[test]
fn envrc_prints_path_add_only() {
    let dir = tempfile::tempdir().expect("tmp");
    let state = tempfile::tempdir().expect("state");
    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", dir.path())
        .env("XDG_STATE_HOME", state.path())
        .args(["hooks", "envrc"])
        .output()
        .expect("run");
    assert_eq!(output.status.code(), Some(0));
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.starts_with("PATH_add "), "{stdout}");
    assert!(stdout.contains("bin"), "{stdout}");
}

#[test]
fn shims_write_refresh_and_remove_stale() {
    let sandbox = tempfile::tempdir().expect("sandbox");
    let tools = sandbox.path().join("tools");
    let bin = sandbox.path().join("bin");
    std::fs::create_dir_all(&tools).expect("tools");

    let write_tool = |name: &str| {
        std::fs::write(
            tools.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: {name}\n---\nbody\n"),
        )
        .expect("tool");
    };
    write_tool("alpha");
    write_tool("beta");

    let run_shims = || {
        Command::cargo_bin("smah")
            .expect("smah binary")
            .env("SMAH_VIRTUAL_TOOLS_DIR", &tools)
            .args(["hooks", "shims", "--dir"])
            .arg(&bin)
            .output()
            .expect("run")
    };

    let output = run_shims();
    assert_eq!(output.status.code(), Some(0));
    assert!(
        output.stdout.is_empty(),
        "shims is a write command: stdout must be empty"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("2 shim(s) written"), "{stderr}");

    use std::os::unix::fs::PermissionsExt;
    for name in ["smah.alpha", "smah.beta"] {
        let path = bin.join(name);
        let meta = std::fs::metadata(&path).expect(name);
        assert_eq!(meta.permissions().mode() & 0o777, 0o755, "{name}");
        let script = std::fs::read_to_string(&path).expect(name);
        assert!(
            script.starts_with("#!/bin/sh\nexec smah --emulate-cli "),
            "{script}"
        );
    }

    // Retire beta, add gamma; re-run: beta's shim is removed.
    std::fs::remove_file(tools.join("beta.md")).expect("remove");
    write_tool("gamma");
    let output = run_shims();
    assert_eq!(output.status.code(), Some(0));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("2 shim(s) written, 1 stale removed"),
        "{stderr}"
    );
    assert!(!bin.join("smah.beta").exists(), "stale shim removed");
    assert!(bin.join("smah.alpha").exists());
    assert!(bin.join("smah.gamma").exists());
}
