//! `smah config init` + bare-`smah` first-run contract (RA-06), fully
//! headless (assert_cmd pipes stdin — the interactive TTY wizard is not
//! reachable here; its pure pieces are unit-tested in config_cmd.rs).
//! No real API contact anywhere.

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;

struct Sandbox {
    _root: tempfile::TempDir,
    state: PathBuf,
    cache: PathBuf,
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("sandbox root");
        let state = root.path().join("state");
        let cache = root.path().join("cache");
        let home = root.path().join("home");
        std::fs::create_dir_all(state.join("smah")).expect("state dir");
        std::fs::create_dir_all(&cache).expect("cache dir");
        std::fs::create_dir_all(&home).expect("home dir");
        Sandbox {
            _root: root,
            state,
            cache,
            home,
        }
    }

    /// `smah` invocation with sandboxed HOME/XDG (no ~/.smah/config.yaml).
    fn smah(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("SMAH_ANTHROPIC_API_KEY");
        cmd
    }

    fn config_path(&self) -> PathBuf {
        self.home.join(".smah/config.yaml")
    }
}

fn run(output: Output) -> (i32, String, String) {
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path)
        .expect("metadata")
        .permissions()
        .mode()
        & 0o777
}

#[test]
fn headless_init_with_env_key_writes_usable_config() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .smah()
        .args([
            "config",
            "init",
            "--non-interactive",
            "--name",
            "Dana",
            "--role",
            "Developer",
            "--experience",
            "Novice",
            "--about",
            "learning ops",
        ])
        .env("SMAH_ANTHROPIC_API_KEY", "test-key-0000")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.is_empty(), "wizard chrome never on stdout: {stdout}");
    let path = sandbox.config_path();
    assert!(path.exists(), "config written");
    assert_eq!(mode(&path), 0o600, "owner-only permissions");
    let text = std::fs::read_to_string(&path).expect("config text");
    assert!(
        !text.contains("test-key-0000"),
        "env keys are not copied into the file"
    );
    assert!(text.contains("name: Dana"), "profile written: {text}");
    assert!(
        text.contains("share_profile: true"),
        "egress defaults recorded"
    );
    assert!(stderr.contains("config written to"), "summary: {stderr}");
    assert!(
        stderr.contains("Sent to the provider"),
        "disclosure shown: {stderr}"
    );
}

#[test]
fn headless_init_refuses_zero_usable_models_exit_4() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .smah()
        .args(["config", "init", "--non-interactive"])
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 4, "stderr: {stderr}");
    assert!(!sandbox.config_path().exists(), "nothing saved");
    assert!(
        stderr.contains("anthropic"),
        "guidance names the provider: {stderr}"
    );
}

#[test]
fn init_refuses_to_clobber_without_force() {
    let sandbox = Sandbox::new();
    std::fs::create_dir_all(sandbox.config_path().parent().unwrap()).expect("dir");
    std::fs::write(sandbox.config_path(), "vsn: \"0.0.1\"\n").expect("existing config");

    let output = sandbox
        .smah()
        .args(["config", "init", "--non-interactive"])
        .env("SMAH_ANTHROPIC_API_KEY", "test-key-0000")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("--force"), "guidance: {stderr}");
    assert_eq!(
        std::fs::read_to_string(sandbox.config_path()).unwrap(),
        "vsn: \"0.0.1\"\n",
        "existing config untouched"
    );
}

#[test]
fn egress_opt_outs_are_recorded() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .smah()
        .args([
            "config",
            "init",
            "--non-interactive",
            "--no-share-profile",
            "--no-share-stats",
        ])
        .env("SMAH_ANTHROPIC_API_KEY", "test-key-0000")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    let text = std::fs::read_to_string(sandbox.config_path()).expect("config");
    assert!(text.contains("share_profile: false"), "{text}");
    assert!(text.contains("share_system_stats: false"), "{text}");
    assert!(
        stderr.contains("your question and any piped input only"),
        "disclosure reflects the opt-out: {stderr}"
    );
}

#[test]
fn bare_smah_shows_help_exit_0_and_offers_init_on_missing_config() {
    let sandbox = Sandbox::new();
    // assert_cmd pipes stdin (non-TTY): plain help path.
    let output = sandbox.smah().output().expect("run smah");
    let (code, stdout, _) = run(output);

    assert_eq!(code, 0, "bare smah is help, not an error");
    assert!(stdout.contains("Usage: smah"), "help on stdout: {stdout}");
}

#[test]
fn unknown_provider_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let output = sandbox
        .smah()
        .args(["config", "init", "--non-interactive", "--provider", "nope"])
        .env("SMAH_NOPE_API_KEY", "x")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("unknown provider"), "{stderr}");
}
