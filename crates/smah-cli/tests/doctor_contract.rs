//! `smah doctor` contract (RA-07): provider/config health with zero network
//! calls and zero key material on any stream. Keyed config → exit 0 with a
//! usable row; nothing usable → exit 4; unreadable config → exit 3; `--json`
//! is one parseable object.

use std::path::PathBuf;

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

    /// `smah` invocation with sandboxed HOME/XDG and every provider env key
    /// removed so usability assertions never depend on the host.
    fn smah(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("SMAH_ANTHROPIC_API_KEY")
            .env_remove("OPENAI_API_KEY")
            .env_remove("SMAH_OPENAI_API_KEY")
            .env_remove("ALIBABA_API_KEY")
            .env_remove("SMAH_ALIBABA_API_KEY");
        cmd
    }

    fn write_config(&self, text: &str) -> PathBuf {
        let path = self.home.join(".smah/config.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).expect("config dir");
        std::fs::write(&path, text).expect("config fixture");
        path
    }
}

fn run(output: std::process::Output) -> (i32, String, String) {
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

const KEYED: &str = "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n";

#[test]
fn keyed_config_doctor_exits_0_and_names_the_usable_provider() {
    let sandbox = Sandbox::new();
    sandbox.write_config(KEYED);
    let output = sandbox.smah().args(["doctor"]).output().expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("anthropic:"), "provider row: {stdout}");
    assert!(stdout.contains("key present"), "{stdout}");
    assert!(stdout.contains("usable"), "{stdout}");
    assert!(
        stdout.contains("1 usable provider"),
        "result line: {stdout}"
    );
    assert!(
        !stdout.contains("test-key-0000"),
        "key material never shown"
    );
    assert!(!stderr.contains("test-key-0000"), "nor on stderr");
}

#[test]
fn nothing_usable_exits_4_with_guidance() {
    let sandbox = Sandbox::new();
    let output = sandbox.smah().args(["doctor"]).output().expect("run smah");
    let (code, stdout, _) = run(output);

    assert_eq!(code, 4, "stdout: {stdout}");
    assert!(stdout.contains("no usable provider"), "{stdout}");
    assert!(
        stdout.contains("smah config init"),
        "points at init: {stdout}"
    );
}

#[test]
fn json_mode_is_one_parseable_object() {
    let sandbox = Sandbox::new();
    sandbox.write_config(KEYED);
    let output = sandbox
        .smah()
        .args(["doctor", "--json"])
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    let value: serde_json::Value = serde_json::from_str(stdout.trim()).expect("one json object");
    assert_eq!(value["usable_providers"], 1);
    let providers = value["providers"].as_array().expect("providers array");
    let anthropic = providers
        .iter()
        .find(|p| p["key"] == "anthropic")
        .expect("anthropic row");
    assert_eq!(anthropic["has_key"], true);
    assert_eq!(anthropic["enabled"], true);
    assert!(
        stdout.contains("share_profile"),
        "egress recorded: {stdout}"
    );
    assert!(!stdout.contains("test-key-0000"), "no key material");
}

#[test]
fn broken_config_exits_3() {
    let sandbox = Sandbox::new();
    sandbox.write_config("vsn: [not, a, mapping]\n");
    let output = sandbox.smah().args(["doctor"]).output().expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 3, "stderr: {stderr}");
}
