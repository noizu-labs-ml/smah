//! SW-03 CLI integration: the `--prompt` compose-review flow against a
//! wiremock OpenAI-compatible endpoint (fake keys only — nothing touches a
//! real API). Covers the full contract: diff shown + nothing written on a
//! pipe, `--accept` applying with provenance + approval stamps, the
//! (wrapper, prompt) cache skipping the second model call, `--reject`,
//! unparseable answers mapping to exit 5, and the flag conflict guard.

use std::path::PathBuf;

use assert_cmd::Command;
use tempfile::TempDir;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A per-test sandbox: wrappers dir, tools dir, config file, XDG dirs —
/// never the real `~/.config` / `~/.cache`.
struct Sandbox {
    _root: TempDir,
    wrappers: PathBuf,
    tools: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("root tmp");
        let wrappers = root.path().join("wrappers");
        let tools = root.path().join("tools");
        let state = root.path().join("xdg-state");
        let cache = root.path().join("xdg-cache");
        std::fs::create_dir_all(&wrappers).expect("wrappers dir");
        std::fs::create_dir_all(&tools).expect("tools dir");
        let config = root.path().join("config.yaml");
        std::fs::write(
            &config,
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba:\n      name: Alibaba\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
        )
        .expect("config fixture");
        Sandbox {
            _root: root,
            wrappers,
            tools,
            state,
            cache,
            config,
        }
    }

    /// Install a wrapper file.
    fn wrapper(&self, name: &str, body: &str) {
        std::fs::write(self.wrappers.join(format!("{name}.md")), body).expect("wrapper fixture");
    }

    fn read_wrapper(&self, name: &str) -> String {
        std::fs::read_to_string(self.wrappers.join(format!("{name}.md"))).expect("read wrapper")
    }

    /// A `smah -c <config> ...` invocation wired to this sandbox + the mock
    /// alibaba endpoint.
    fn smah(&self, server: &MockServer, args: &[&str]) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("SMAH_WRAPPERS_DIR", &self.wrappers)
            .env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("ALIBABA_BASE_URL", server.uri())
            .arg("-c")
            .arg(&self.config)
            .args(args);
        cmd
    }
}

/// A wrapper whose `model:` frontmatter pins the (mock) provider + model.
const DEMO: &str = "---\n\
name: demo\ndescription: fixture wrapper\nmodel: alibaba.wrap-model\n\
steps:\n  - id: first\n    run: [\"echo\", \"one\"]\n---\nfixture body\n";

/// The model's structured answer: one appended step.
const CHANGES_JSON: &str = "{\"explanation\":\"appends a second step\",\"changes\":{\"add_steps\":[{\"after\":\"first\",\"id\":\"second\",\"run\":[\"echo\",\"two\"]}]}}";

/// A complete OpenAI-compatible SSE streaming response carrying `text`.
fn sse(text: &str) -> ResponseTemplate {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    ResponseTemplate::new(200)
        .set_body_string(format!(
            "data: {{\"choices\":[{{\"delta\":{{\"role\":\"assistant\"}}}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{\"content\":\"{escaped}\"}}}}]}}\n\ndata: {{\"choices\":[{{\"delta\":{{}}}}]}}\n\ndata: [DONE]\n\n"
        ))
        .insert_header("content-type", "text/event-stream")
}

async fn mount(server: &MockServer, template: ResponseTemplate, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/chat/completions"))
        .respond_with(template)
        .expect(expect)
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

/// The `model` field of the first captured request body.
async fn first_request_model(server: &MockServer) -> serde_json::Value {
    let requests = server.received_requests().await.expect("requests recorded");
    assert!(!requests.is_empty(), "at least one request expected");
    serde_json::from_slice::<serde_json::Value>(&requests[0].body).expect("request body json")
        ["model"]
        .clone()
}

#[tokio::test]
async fn prompt_prints_diff_and_writes_nothing_on_a_pipe() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;
    mount(&server, sse(CHANGES_JSON), 1).await;

    let output = sb
        .smah(
            &server,
            &["--wrapper", "demo", "--prompt", "add a second step"],
        )
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("proposed change"), "stdout: {stdout}");
    assert!(stdout.contains("appends a second step"));
    assert!(stdout.contains("+ step second (run) after first"));
    assert!(stdout.contains("id: second"));
    assert!(stdout.contains("# provenance:"), "the stamp is previewed");
    assert!(
        stdout.contains("nothing written"),
        "non-TTY must not block on a question"
    );
    assert_eq!(sb.read_wrapper("demo"), DEMO, "nothing was written");
}

#[tokio::test]
async fn accept_applies_with_provenance_and_approval() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;
    mount(&server, sse(CHANGES_JSON), 1).await;

    let output = sb
        .smah(
            &server,
            &[
                "--wrapper",
                "demo",
                "--prompt",
                "add a second step",
                "--accept",
            ],
        )
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("applied + approved"), "stdout: {stdout}");
    let file = sb.read_wrapper("demo");
    assert!(file.contains("id: second"), "applied: {file}");
    assert!(
        file.contains("approved:"),
        "acceptance is the review: {file}"
    );
    assert!(file.contains("# provenance:"), "provenance stamped: {file}");
    assert!(file.contains("prompt_sha256="));
    // The wrapper's own model: won precedence over the spec default (owner
    // decision Q4) — proven on the wire.
    assert_eq!(
        first_request_model(&server).await,
        serde_json::json!("wrap-model")
    );
    // The approved file still plans cleanly through the deterministic path.
    let plan = sb
        .smah(&server, &["--wrapper", "demo"])
        .write_stdin("")
        .output()
        .expect("plan smah");
    let (pcode, pout, perr) = run(plan);
    assert_eq!(pcode, 0, "stderr: {perr}");
    assert!(
        pout.contains("approved 2"),
        "plan shows the approval: {pout}"
    );
}

#[tokio::test]
async fn repeat_prompt_is_a_cache_hit_not_a_model_call() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;
    // Exactly ONE wire request across BOTH runs.
    mount(&server, sse(CHANGES_JSON), 1).await;

    let first = sb
        .smah(
            &server,
            &["--wrapper", "demo", "--prompt", "add a second step"],
        )
        .write_stdin("")
        .output()
        .expect("first run");
    assert_eq!(run(first).0, 0);

    let second = sb
        .smah(
            &server,
            &["--wrapper", "demo", "--prompt", "add a second step"],
        )
        .write_stdin("")
        .output()
        .expect("second run");
    let (code, stdout, stderr) = run(second);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(
        stdout.contains("cache hit"),
        "second identical prompt must be served from the cache: {stdout}"
    );
    assert_eq!(
        sb.read_wrapper("demo"),
        DEMO,
        "cache-hit review still writes nothing without --accept"
    );
    // wiremock's .expect(1) verifies exactly one request hit the server.
    server.verify().await;
}

#[tokio::test]
async fn reject_keeps_the_file() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;
    mount(&server, sse(CHANGES_JSON), 1).await;

    let output = sb
        .smah(
            &server,
            &[
                "--wrapper",
                "demo",
                "--prompt",
                "add a second step",
                "--reject",
            ],
        )
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert!(stdout.contains("rejected"), "stdout: {stdout}");
    assert_eq!(sb.read_wrapper("demo"), DEMO);
}

#[tokio::test]
async fn unparseable_answer_is_exit_5_and_touches_nothing() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;
    mount(&server, sse("I would rather not change anything."), 1).await;

    let output = sb
        .smah(
            &server,
            &["--wrapper", "demo", "--prompt", "delete everything"],
        )
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 5, "provider-contract failure: {stderr}");
    assert!(stderr.contains("unusable change"));
    assert_eq!(sb.read_wrapper("demo"), DEMO);
}

#[tokio::test]
async fn accept_and_reject_together_is_usage_exit_2() {
    let sb = Sandbox::new();
    sb.wrapper("demo", DEMO);
    let server = MockServer::start().await;

    let output = sb
        .smah(
            &server,
            &["--wrapper", "demo", "--prompt", "x", "--accept", "--reject"],
        )
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    assert_eq!(code, 2, "stderr: {stderr}");
    assert!(stderr.contains("mutually exclusive"));
    // The guard fires before any provider call — no wire traffic.
    server.verify().await;
}
