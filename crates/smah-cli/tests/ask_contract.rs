//! `smah ask` CLI contract (RA-01) over a mocked Anthropic endpoint: prose
//! streams to stdout while `<exec>` bodies render as numbered suggestion
//! blocks, destructive commands escalate to the banner, piped stdin rides
//! along with a loud truncation notice, and the exit-code table holds
//! (0 ok / 2 usage / 4 missing key). Fake key, fake endpoint — no real API.

use std::path::PathBuf;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// Per-test sandbox: config file + XDG state/cache (log + scratch live
/// under the tempdir).
struct Sandbox {
    _root: tempfile::TempDir,
    state: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    /// A config with a keyed anthropic provider block.
    fn new() -> Self {
        Self::with_config("vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n")
    }

    fn with_config(text: &str) -> Self {
        let root = tempfile::tempdir().expect("sandbox root");
        let state = root.path().join("state");
        let cache = root.path().join("cache");
        std::fs::create_dir_all(state.join("smah")).expect("state dir");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let config = root.path().join("config.yaml");
        std::fs::write(&config, text).expect("config fixture");
        Sandbox {
            _root: root,
            state,
            cache,
            config,
        }
    }

    /// A `smah ask` invocation wired to this sandbox + the mock endpoint,
    /// with any real ambient key removed. `global` args precede `ask`.
    fn ask(&self, server: &MockServer, global: &[&str], args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("ANTHROPIC_BASE_URL", server.uri())
            .env_remove("ANTHROPIC_API_KEY")
            .env_remove("SMAH_ANTHROPIC_API_KEY")
            .arg("-c")
            .arg(&self.config)
            .args(global)
            .arg("ask")
            .args(args);
        cmd
    }
}

/// A complete Anthropic SSE streaming response for `text`, split into two
/// deltas at a mid-tag boundary when one exists (exercises the stream
/// filter's split-tag holding).
fn anthropic_sse(text: &str) -> ResponseTemplate {
    let split_at = text.find("<exec").map(|i| i + 3).unwrap_or(text.len() / 2);
    let (first, second) = text.split_at(split_at.max(1).min(text.len().saturating_sub(1)));
    let mut sse = String::new();
    sse.push_str("event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"role\":\"assistant\"}}\n\n");
    sse.push_str("event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\"}}\n\n");
    for part in [first, second] {
        if part.is_empty() {
            continue;
        }
        let payload = serde_json::json!({
            "type": "content_block_delta",
            "index": 0,
            "delta": {"type": "text_delta", "text": part},
        });
        sse.push_str(&format!("event: content_block_delta\ndata: {payload}\n\n"));
    }
    sse.push_str(
        "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    );
    sse.push_str("event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"}}\n\n");
    sse.push_str("event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n");
    ResponseTemplate::new(200)
        .set_body_string(sse)
        .insert_header("content-type", "text/event-stream")
}

const GOOD_ANSWER: &str = "Purge old journals like this:\n<exec shell=\"bash\">\n<title>Purge journal logs</title>\n<purpose>Drop entries older than 7 days.</purpose>\n<command>journalctl --vacuum-time=7d</command>\n<risk>IRREVERSIBLE — deletes matched entries</risk>\n<touches>/var/log/journal/**</touches>\n</exec>\nThat frees space without touching live services.";

async fn mount_answer(server: &MockServer, text: &str) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(anthropic_sse(text))
        .expect(1)
        .mount(server)
        .await;
}

fn run(output: std::process::Output) -> (i32, String, String) {
    (
        output.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&output.stdout).to_string(),
        String::from_utf8_lossy(&output.stderr).to_string(),
    )
}

/// The first captured request body, parsed.
async fn first_request(server: &MockServer) -> serde_json::Value {
    let requests = server.received_requests().await.expect("requests recorded");
    assert!(!requests.is_empty(), "a request was expected");
    serde_json::from_slice(&requests[0].body).expect("request body json")
}

#[tokio::test]
async fn ask_streams_prose_and_renders_the_block() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, GOOD_ANSWER).await;

    let output = sandbox
        .ask(&server, &[], &["how do I free disk space?"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    // Prose on stdout, raw tags never on stdout.
    assert!(
        stdout.contains("Purge old journals like this:"),
        "stdout: {stdout}"
    );
    assert!(
        !stdout.contains("<exec"),
        "stdout must not leak tags: {stdout}"
    );
    // The numbered block (plain shape: the test pipe is not a TTY).
    assert!(stdout.contains("## Suggested command 1 of 1"));
    assert!(
        stdout.contains("command:\njournalctl --vacuum-time=7d"),
        "command line shape: {stdout}"
    );
    assert!(stdout.contains(
        "risk: IRREVERSIBLE — deletes matched entries (model guidance, not a guarantee)"
    ));
    assert!(stdout.contains("touches: /var/log/journal/**"));
    // Diagnostics on stderr: the standing non-execution notice.
    assert!(
        stderr.contains("SMAH does not run commands"),
        "stderr: {stderr}"
    );
    // The request carried the system contract + the question.
    let body = first_request(&server).await;
    assert!(body["system"]
        .as_str()
        .expect("system string")
        .contains("print-only"));
    assert!(body["messages"][0]["content"]
        .as_str()
        .expect("user content")
        .contains("how do I free disk space?"));
}

#[tokio::test]
async fn ask_destructive_command_escalates_to_the_banner() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    let answer =
        "Wipe everything:\n<exec><command>rm -rf /</command><risk>claimed safe</risk></exec>";
    mount_answer(&server, answer).await;

    let output = sandbox
        .ask(&server, &[], &["wipe it all"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, _) = run(output);

    assert_eq!(code, 0);
    assert!(
        stdout.contains("warning: [!] DESTRUCTIVE"),
        "local classifier banner must appear: {stdout}"
    );
    assert!(stdout.contains("guidance, not a guarantee"));
}

#[tokio::test]
async fn ask_without_a_query_is_a_usage_error() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let output = sandbox
        .ask(&server, &[], &[])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.to_lowercase().contains("ask"), "stderr: {stderr}");
}

#[tokio::test]
async fn ask_without_a_key_exits_4_with_guidance() {
    let sandbox = Sandbox::with_config("vsn: \"0.0.1\"\n");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let output = sandbox
        .ask(&server, &[], &["anything"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 4, "stderr: {stderr}");
    assert!(stderr.contains("anthropic"), "names the provider: {stderr}");
    assert!(stdout.is_empty(), "no content without a turn: {stdout}");
}

#[tokio::test]
async fn piped_input_rides_along_and_truncation_is_loud() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, "Looks fine.").await;

    let big_input = "x".repeat(5000);
    let output = sandbox
        .ask(&server, &["--max-input-chars", "100"], &["summarize this"])
        .write_stdin(big_input)
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("Looks fine."));
    assert!(
        stderr.contains("input truncated: sent 100 of 5000 bytes"),
        "truncation notice: {stderr}"
    );
    // The model was told about the truncation and got (only) the capped data.
    let body = first_request(&server).await;
    let content = body["messages"][0]["content"].as_str().expect("content");
    assert!(content.contains("only the first 100 of 5000 bytes"));
    assert!(content.contains("Piped input"));
    assert!(
        content.contains(&"x".repeat(100)),
        "the capped data rides along"
    );
    assert!(
        !content.contains(&"x".repeat(101)),
        "not a byte over the cap"
    );
}

// ---------------------------------------------------------------------------
// RA-04 — non-TTY contract + --json
// ---------------------------------------------------------------------------

#[tokio::test]
async fn json_mode_emits_one_parseable_object() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, GOOD_ANSWER).await;

    let output = sandbox
        .ask(&server, &[], &["--json", "free disk"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    // The whole stdout is one JSON object — no prose, no blocks, no ANSI.
    let value: serde_json::Value = serde_json::from_str(stdout.trim())
        .unwrap_or_else(|err| panic!("stdout is not pure JSON ({err}): {stdout}"));
    assert_eq!(value["provider"], "anthropic");
    assert_eq!(value["model"], "claude-opus-5-5");
    assert!(value["answer"]
        .as_str()
        .expect("answer")
        .contains("Purge old journals"));
    assert_eq!(
        value["suggestions"][0]["command"],
        serde_json::json!("journalctl --vacuum-time=7d")
    );
    assert_eq!(
        value["suggestions"][0]["classification"]["level"],
        serde_json::json!("benign")
    );
    assert!(
        !stdout.contains('\x1b'),
        "no ANSI escapes in JSON: {stdout}"
    );
    // No streamed prose leaked before the object.
    assert!(stdout.trim_start().starts_with('{'));
    // Diagnostics still stderr-only; nothing human on stdout.
    assert!(stderr.contains("SMAH does not run commands") || stderr.is_empty());
}

#[tokio::test]
async fn pipe_mode_stdout_carries_no_ansi_or_chrome() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, GOOD_ANSWER).await;

    let output = sandbox
        .ask(&server, &[], &["free disk"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, _) = run(output);

    assert_eq!(code, 0);
    assert!(!stdout.contains('\x1b'), "no ANSI on a non-TTY stdout");
    assert!(
        !stdout.contains("SMAH does not run commands"),
        "chrome stays on stderr"
    );
    assert!(stdout.contains("## Suggested command 1 of 1"));
}

#[tokio::test]
async fn provider_500_maps_to_exit_5() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .expect(1)
        .mount(&server)
        .await;

    let output = sandbox
        .ask(&server, &[], &["anything"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 5, "stderr: {stderr}");
    assert!(
        stdout.is_empty(),
        "no content after a provider failure: {stdout}"
    );
}

#[tokio::test]
async fn provider_401_maps_to_exit_4() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "authentication_error", "message": "invalid x-api-key"}
        })))
        .expect(1)
        .mount(&server)
        .await;

    let output = sandbox
        .ask(&server, &[], &["anything"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 4, "stderr: {stderr}");
}

#[tokio::test]
async fn json_mode_error_paths_keep_exit_codes() {
    // --json does not change failure behavior: usage error still exits 2
    // with human guidance on stderr and nothing on stdout.
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let output = sandbox
        .ask(&server, &["--json"], &["--max-input-chars"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, _) = run(output);

    assert_eq!(code, 2);
    assert!(stdout.is_empty());
}

// ---------------------------------------------------------------------------
// RA-05 — clipboard handoff
// ---------------------------------------------------------------------------

#[tokio::test]
async fn single_command_answer_reports_copy_outcome() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, GOOD_ANSWER).await;

    let output = sandbox
        .ask(&server, &[], &["free disk"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    // Copy success depends on the host (headless CI has no pasteboard);
    // both outcomes are stderr chrome and never change the exit code or
    // stdout content.
    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("journalctl --vacuum-time=7d"));
    assert!(
        stderr.contains("copied to clipboard: journalctl --vacuum-time=7d")
            || stderr.contains("clipboard unavailable")
            || stderr.contains("clipboard write failed"),
        "copy outcome reported on stderr: {stderr}"
    );
}

#[tokio::test]
async fn no_copy_flag_skips_the_clipboard_entirely() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    mount_answer(&server, GOOD_ANSWER).await;

    let output = sandbox
        .ask(&server, &[], &["--no-copy", "free disk"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stderr.contains("clipboard: disabled for this run (--no-copy)"),
        "stderr: {stderr}"
    );
}

#[tokio::test]
async fn multi_command_answer_is_not_bulk_copied() {
    let sandbox = Sandbox::new();
    let server = MockServer::start().await;
    let answer = "two steps:\n<exec><command>df -h</command></exec>\nthen:\n<exec><command>du -sh .</command></exec>";
    mount_answer(&server, answer).await;

    let output = sandbox
        .ask(&server, &[], &["--copy", "disk usage"])
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("## Suggested command 2 of 2"));
    assert!(
        stderr.contains("multiple commands"),
        "no bulk copy of several commands: {stderr}"
    );
}
