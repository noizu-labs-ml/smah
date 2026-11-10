//! RA sync coexistence: the rust-ask subcommand surface (ask/chat/resume/
//! config/doctor) and the script-wrapper flag/subcommand surface must live
//! in ONE binary — this is the sync-resolution contract (bare `smah` keeps
//! the rust-ask behaviour: help exit 0, config-init offer on a TTY without
//! config; `--wrapper` short-circuits before the subcommand match). Every
//! test runs in a fully sandboxed HOME/XDG tree under a tempdir — never the
//! real `~/.smah`, and nothing here touches the network or a keychain.

use assert_cmd::Command;
use predicates::prelude::*;
use tempfile::TempDir;

struct Sandbox {
    _root: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        let root = TempDir::new().expect("root tmp");
        Sandbox { _root: root }
    }

    /// A `smah` invocation wired to this sandbox (empty HOME — no config).
    fn smah(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("HOME", self._root.path())
            .env("XDG_CONFIG_HOME", self._root.path().join("xdg-config"))
            .env("XDG_STATE_HOME", self._root.path().join("xdg-state"))
            .env("XDG_CACHE_HOME", self._root.path().join("xdg-cache"))
            .env("SMAH_VIRTUAL_TOOLS_DIR", self._root.path().join("tools"))
            .env("SMAH_WRAPPERS_DIR", self._root.path().join("wrappers"));
        cmd
    }
}

#[test]
fn help_advertises_both_surfaces() {
    let sb = Sandbox::new();
    sb.smah()
        .arg("--help")
        .assert()
        .code(0)
        .stdout(predicates::str::contains("Usage: smah"))
        // rust-ask subcommands are present…
        .stdout(predicates::str::contains("ask"))
        .stdout(predicates::str::contains("chat"))
        .stdout(predicates::str::contains("resume"))
        .stdout(predicates::str::contains("doctor"))
        // …and so are the wrapper surfaces (flag + subcommand).
        .stdout(predicates::str::contains("--wrapper"))
        .stdout(predicates::str::contains("wrapper"));
}

#[test]
fn ask_and_wrapper_subcommand_helps_exist() {
    let sb = Sandbox::new();
    sb.smah()
        .args(["ask", "--help"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("--json"))
        .stdout(predicates::str::contains("--copy"));
    sb.smah()
        .args(["wrapper", "--help"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains("list"))
        .stdout(predicates::str::contains("run"));
}

#[test]
fn bare_smah_prints_help_exit_0_non_tty() {
    let sb = Sandbox::new();
    // Non-TTY (assert_cmd pipes stdin) + no config: rust-ask contract —
    // help on stdout, exit 0, and never the old develop "not in this build"
    // notice.
    sb.smah()
        .assert()
        .code(0)
        .stdout(predicates::str::contains("Usage: smah"))
        .stderr(predicates::str::contains("not in this build").not());
}

#[test]
fn wrapper_flag_still_resolves_plans() {
    let sb = Sandbox::new();
    let wrappers = sb._root.path().join("wrappers");
    std::fs::create_dir_all(&wrappers).expect("wrappers dir");
    std::fs::write(
        wrappers.join("echo-plan.md"),
        "---\n\
name: echo-plan\ndescription: Echo the label param\n\
params:\n  - name: label\n    required: true\n\
steps:\n  - id: say\n    run: [\"echo\", \"{{params.label}}\"]\n    timeout_secs: 10\n\
outputs: []\n---\n",
    )
    .expect("wrapper fixture");
    sb.smah()
        .args(["--wrapper", "echo-plan", "--param", "label=hi"])
        .assert()
        .code(0)
        .stdout(predicates::str::contains(
            "smah wrapper 'echo-plan' — Echo the label param",
        ))
        .stdout(predicates::str::contains(
            "smah-run echo-plan --param label=hi",
        ));
}

#[test]
fn wrapper_flag_wins_over_subcommand_presence() {
    let sb = Sandbox::new();
    // `--wrapper` with an unknown name fails at the flag path (exit 2,
    // wrapper-flavoured stderr) BEFORE any subcommand parsing could
    // misroute it — the flag short-circuits, exactly as the sync resolved.
    sb.smah()
        .args(["--wrapper", "does-not-exist"])
        .assert()
        .code(2)
        .stderr(predicates::str::contains("does-not-exist"));
}
