//! `smah chat` / `smah resume` CLI contract (RA-03): interactive-only —
//! non-TTY invocations are usage errors pointing at the pipe path, and the
//! one-shot `ask` never opens a session store when its input is piped.
//! The loop itself is unit-tested over scripted IO (smah-ask chat::tests);
//! no pty is driven here.

use std::path::PathBuf;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Sandbox {
    _root: tempfile::TempDir,
    state: PathBuf,
    cache: PathBuf,
    home: PathBuf,
    config: PathBuf,
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
        let config = root.path().join("config.yaml");
        std::fs::write(
            &config,
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
        )
        .expect("config fixture");
        Sandbox {
            _root: root,
            state,
            cache,
            home,
            config,
        }
    }

    /// `smah` with sandboxed HOME/XDG and the mock endpoint; stdin is a
    /// pipe (assert_cmd default) — i.e. NOT a terminal.
    fn smah(&self, server: &MockServer) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("HOME", &self.home)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("ANTHROPIC_BASE_URL", server.uri())
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("SMAH_ANTHROPIC_API_KEY")
            .arg("-c")
            .arg(&self.config);
        cmd
    }

    fn session_db(&self) -> PathBuf {
        self.config.parent().unwrap().join("smah.db")
    }
}

fn run(output: std::process::Output) -> (i32, String, String) {
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

#[tokio::test]
async fn chat_without_a_tty_is_a_usage_error_pointing_at_ask() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let output = sandbox
        .smah(&server)
        .args(["chat", "free disk?"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stdout.is_empty(), "no content on a gated run: {stdout}");
    assert!(stderr.contains("interactive terminal"), "{stderr}");
    assert!(
        stderr.contains("smah ask"),
        "points at the pipe path: {stderr}"
    );
    assert!(!sandbox.session_db().exists(), "no session side effects");
}

#[tokio::test]
async fn resume_without_a_tty_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    let output = sandbox
        .smah(&server)
        .args(["resume", "--last"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("interactive terminal"), "{stderr}");
}

#[tokio::test]
async fn piped_ask_stays_one_shot_and_opens_no_session_db() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    let sse = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n\
               event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n\
               event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"done\"}}\n\n\
               event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n\
               event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n\
               event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string(sse)
                .insert_header("content-type", "text/event-stream"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let output = sandbox
        .smah(&server)
        .args(["ask", "what is this?"])
        .write_stdin("piped data")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("done"), "{stdout}");
    assert!(
        !sandbox.session_db().exists(),
        "piped runs never open a session (TTY-only continuation)"
    );
}
