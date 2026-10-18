//! Emulate-path integration battery (plan W4–W6): the real binary over the
//! real M2 client against wiremock fake providers. Every key here is a fake
//! (`test-key-0000`); nothing touches a real API.
//!
//! First-byte discipline note: wiremock 0.6 has no chunked-drip body API, so
//! the flush-per-delta guarantee is proven at the driver level
//! (`smah-core` virtual_tools_spec: `driver_streams_deltas_even_when_backend_then_fails`);
//! these tests exercise the streaming path end-to-end over SSE.

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A per-test sandbox: tools dir, notes root, config file, and the
/// XDG state/cache dirs — all injected into the child process's env, so
/// tests never touch the real user directories and never race each other.
struct Sandbox {
    /// Root tempdir (everything lives under it).
    _root: tempfile::TempDir,
    tools: PathBuf,
    notes: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("sandbox root");
        let tools = root.path().join("tools");
        let notes = root.path().join("notes");
        let state = root.path().join("state");
        let cache = root.path().join("cache");
        std::fs::create_dir_all(&tools).expect("tools dir");
        std::fs::create_dir_all(&notes).expect("notes root");
        std::fs::create_dir_all(state.join("smah")).expect("state dir");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let config = root.path().join("config.yaml");
        std::fs::write(
            &config,
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    anthropic:\n      name: Anthropic\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
        )
        .expect("config fixture");
        Sandbox {
            _root: root,
            tools,
            notes,
            state,
            cache,
            config,
        }
    }

    /// Install a tool file.
    fn tool(&self, name: &str, body: &str) {
        std::fs::write(
            self.tools.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: fixture {name}\nmodel: anthropic.claude-fixture-1\nallow_roots:\n  - \"{}\"\n---\n{body}\n",
                self.notes.display()
            ),
        )
        .expect("write tool fixture");
    }

    /// Install a tool with no allow_roots and no model line.
    fn bare_tool(&self, name: &str, body: &str) {
        std::fs::write(
            self.tools.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: fixture {name}\n---\n{body}\n"),
        )
        .expect("write tool fixture");
    }

    /// A `smah` invocation wired to this sandbox (env + `-c` only).
    fn base(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .arg("-c")
            .arg(&self.config);
        cmd
    }

    /// A `smah --emulate-cli` invocation wired to this sandbox.
    fn emulate(&self, tool: &str) -> Command {
        let mut cmd = self.base();
        cmd.arg("--emulate-cli").arg(tool);
        cmd
    }

    /// `smah <global-args...> --emulate-cli <tool>` — global flags must
    /// precede `--emulate-cli` (everything after the tool is tool data).
    fn emulate_with(&self, global: &[&str], tool: &str) -> Command {
        let mut cmd = self.base();
        cmd.args(global).arg("--emulate-cli").arg(tool);
        cmd
    }

    fn audit_log(&self) -> PathBuf {
        self.state.join("smah/audit-virtual-tools.jsonl")
    }
}

/// One Anthropic SSE delta event carrying `text`.
fn delta(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{escaped}\"}}}}\n\n"
    )
}

/// A complete, valid streaming response whose text is `text`.
fn sse(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"role\":\"assistant\"}}}}\n\n{}event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
            delta(text)
        ))
        .insert_header("content-type", "text/event-stream")
}

/// Mount a POST /v1/messages mock with an expectation. Same-priority mocks
/// match in insertion order and stop matching once exhausted, so mounting
/// turn-1 then turn-2 gives sequential scripted responses.
async fn mount(server: &MockServer, template: ResponseTemplate, expect: u64) {
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(template)
        .expect(expect)
        .mount(server)
        .await;
}

fn run(output: Output) -> (i32, String, String) {
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (code, stdout, stderr)
}

// ---------------------------------------------------------------------------
// Happy path
// ---------------------------------------------------------------------------

#[tokio::test]
async fn happy_path_exit_zero_writes_file_and_streams() {
    let sandbox = Sandbox::new();
    sandbox.tool("saver", "Save a summary.");
    let server = MockServer::start().await;
    mount(
        &server,
        sse("Saving now.\n@@smah write {\"path\": \"out.md\", \"content\": \"the summary body\"}\nDone.\n"),
        1,
    )
    .await;

    let output = sandbox
        .emulate("saver")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "Saving now.\nDone.\n");
    assert!(
        !stdout.contains("@@smah"),
        "directive lines never reach stdout"
    );
    assert_eq!(
        std::fs::read_to_string(sandbox.notes.join("out.md")).expect("file written"),
        "the summary body"
    );
    // Roots disclosure + per-op notice on stderr; stdout purity kept.
    assert!(stderr.contains("file surface:"), "{stderr}");
    assert!(
        stderr.contains("wrote") && stderr.contains("out.md"),
        "{stderr}"
    );
    // Audit: exactly one allowed line for the write.
    let audit = std::fs::read_to_string(sandbox.audit_log()).expect("audit log");
    let line: serde_json::Value = serde_json::from_str(audit.trim()).expect("json line");
    assert_eq!(line["op"], "write");
    assert_eq!(line["decision"], "allowed");
    assert_eq!(line["tool"], "saver");
    assert_eq!(line["bytes"], 16);
    assert!(!audit.contains("the summary body"), "contents never logged");
}

// ---------------------------------------------------------------------------
// Provider / exit-code table
// ---------------------------------------------------------------------------

#[tokio::test]
async fn provider_401_exits_4() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Just answer.");
    let server = MockServer::start().await;
    mount(
        &server,
        ResponseTemplate::new(401).set_body_json(serde_json::json!({
            "type": "error",
            "error": {"type": "authentication_error", "message": "invalid x-api-key"}
        })),
        1,
    )
    .await;
    let (code, _, stderr) = run(sandbox
        .emulate("plain")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 4, "{stderr}");
    assert!(stderr.contains("rejected the API key"), "{stderr}");
}

#[tokio::test]
async fn provider_500_exits_5() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Just answer.");
    let server = MockServer::start().await;
    mount(
        &server,
        ResponseTemplate::new(500).set_body_string("boom"),
        1,
    )
    .await;
    let (code, _, stderr) = run(sandbox
        .emulate("plain")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 5, "{stderr}");
    assert!(stderr.contains("is failing"), "{stderr}");
}

#[tokio::test]
async fn network_refusal_exits_5() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Just answer.");
    let (code, _, stderr) = run(sandbox
        .emulate("plain")
        // nothing listens on port 1
        .env("ANTHROPIC_BASE_URL", "http://127.0.0.1:1")
        .output()
        .expect("run"));
    assert_eq!(code, 5, "{stderr}");
    assert!(stderr.contains("can't reach provider"), "{stderr}");
}

// ---------------------------------------------------------------------------
// Tool-file classes
// ---------------------------------------------------------------------------

#[test]
fn tool_not_found_exits_2_with_installed_hint() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("present", "Here.");
    let (code, _, stderr) = run(sandbox.emulate("absent").output().expect("run"));
    assert_eq!(code, 2);
    assert!(
        stderr.contains("no virtual tool named 'absent'"),
        "{stderr}"
    );
    assert!(stderr.contains("Installed: present"), "{stderr}");
}

#[test]
fn invalid_tool_file_exits_3_with_reason() {
    let sandbox = Sandbox::new();
    std::fs::write(
        sandbox.tools.join("broken.md"),
        "---\nname: broken\ndescription: x\nconstrants:\n  - typo\n---\nbody\n",
    )
    .expect("broken fixture");
    let (code, _, stderr) = run(sandbox.emulate("broken").output().expect("run"));
    assert_eq!(code, 3);
    assert!(stderr.contains("constrants"), "{stderr}");
}

#[test]
fn tool_with_unusable_root_exits_3() {
    let sandbox = Sandbox::new();
    // allow_roots entry points at a nonexistent directory — invalid at
    // load-for-invoke.
    std::fs::write(
        sandbox.tools.join("badroot.md"),
        format!(
            "---\nname: badroot\ndescription: x\nallow_roots:\n  - \"{}\"\n---\nbody\n",
            sandbox._root.path().join("does-not-exist").display()
        ),
    )
    .expect("fixture");
    let (code, _, stderr) = run(sandbox.emulate("badroot").output().expect("run"));
    assert_eq!(code, 3);
    assert!(stderr.contains("not an existing directory"), "{stderr}");
}

// ---------------------------------------------------------------------------
// File-surface behavior end to end
// ---------------------------------------------------------------------------

#[tokio::test]
async fn directive_violation_exits_3_and_stops() {
    let sandbox = Sandbox::new();
    sandbox.tool("attacker", "Escalate.");
    let server = MockServer::start().await;
    mount(
        &server,
        sse(
            "Sure.\n@@smah write {\"path\": \"/etc/passwd\", \"content\": \"pwned\"}\nMore text.\n",
        ),
        1,
    )
    .await;
    let (code, stdout, stderr) = run(sandbox
        .emulate("attacker")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 3, "{stderr}");
    assert!(stderr.contains("FORBIDDEN"), "{stderr}");
    let audit = std::fs::read_to_string(sandbox.audit_log()).expect("audit log");
    assert!(audit.contains("\"decision\":\"denied\""), "{audit}");
    let _ = stdout;
}

#[tokio::test]
async fn no_fs_flag_converts_directives_to_soft_results() {
    let sandbox = Sandbox::new();
    sandbox.tool("saver", "Save it.");
    let server = MockServer::start().await;
    // Turn 1: a write directive; turn 2 (after the soft refusal): final text.
    // Turn 2's request carries the results block; give that mock top
    // priority so it wins on turn 2 regardless of insertion-order matching.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("File surface results"))
        .respond_with(sse("Fine, here is the answer inline.\n"))
        .with_priority(1)
        .mount(&server)
        .await;
    mount(
        &server,
        sse("@@smah write {\"path\": \"out.md\", \"content\": \"x\"}\n"),
        1,
    )
    .await;

    let (code, stdout, stderr) = run(sandbox
        .emulate_with(&["--no-fs"], "saver")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "Fine, here is the answer inline.\n");
    assert!(!sandbox.notes.join("out.md").exists(), "nothing written");
    assert!(stderr.contains("disabled (--no-fs)"), "{stderr}");
}

#[tokio::test]
async fn read_directive_loops_with_file_results() {
    let sandbox = Sandbox::new();
    sandbox.tool("reader", "Read the file.");
    std::fs::write(sandbox.notes.join("data.txt"), "42").expect("data");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("File surface results"))
        .respond_with(sse("The file says 42.\n"))
        .with_priority(1)
        .mount(&server)
        .await;
    mount(&server, sse("@@smah read {\"path\": \"data.txt\"}\n"), 1).await;

    let (code, stdout, stderr) = run(sandbox
        .emulate("reader")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "The file says 42.\n");
    assert!(
        stderr.contains("read") && stderr.contains("data.txt"),
        "{stderr}"
    );
}

// ---------------------------------------------------------------------------
// Stdin
// ---------------------------------------------------------------------------

#[tokio::test]
async fn piped_stdin_reaches_the_model() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("echo-ish", "Echo the input's key point.");
    let server = MockServer::start().await;
    let marker = "UNIQUE-STDIN-MARKER-7f3a";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains(marker))
        .respond_with(sse("key point found.\n"))
        .expect(1)
        .mount(&server)
        .await;

    let (code, stdout, stderr) = run(sandbox
        .emulate("echo-ish")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .write_stdin(format!("{marker} and some data\n"))
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "key point found.\n");
    // The body_string_contains matcher proved the pipe reached the provider:
    // an unmatched request would 404 and fail the run with exit 5.
    let requests = server.received_requests().await.expect("requests");
    let body = String::from_utf8_lossy(&requests[0].body).to_string();
    assert!(body.contains(marker), "{body}");
}

#[tokio::test]
async fn stdin_truncation_is_loud_but_not_fatal() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("summarizer", "Summarize.");
    let server = MockServer::start().await;
    let tail = "TAIL-THAT-MUST-NOT-BE-SENT";
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .respond_with(sse("summary of the head.\n"))
        .expect(1)
        .mount(&server)
        .await;

    let big_input = format!("{}{}\n", "a".repeat(500), tail);
    let (code, _, stderr) = run(sandbox
        .emulate_with(&["--max-input-chars", "256"], "summarizer")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .write_stdin(big_input)
        .output()
        .expect("run"));
    assert_eq!(code, 0);
    assert!(stderr.contains("input truncated: sent 256 of"), "{stderr}");
    // The tail must not have been sent.
    let requests = server.received_requests().await.expect("requests");
    let body = String::from_utf8_lossy(&requests[0].body).to_string();
    assert!(
        !body.contains(tail),
        "truncated tail must not reach the provider"
    );
}

// ---------------------------------------------------------------------------
// Argument hazards (plan D6 mandatory tests)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flags_after_tool_name_reach_the_model_verbatim() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("edit", "Edit per instructions.");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("--version"))
        .and(body_string_contains("--unknown"))
        .and(body_string_contains("--help"))
        .respond_with(sse("edited.\n"))
        .expect(1)
        .mount(&server)
        .await;

    let (code, _, stderr) = run(sandbox
        .emulate("edit")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .args(["--version", "--unknown", "--", "--help"])
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
}

#[tokio::test]
async fn global_model_flag_precedes_tool_and_wins() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Answer.");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("claude-global-override"))
        .respond_with(sse("ok.\n"))
        .expect(1)
        .mount(&server)
        .await;

    let (code, _, stderr) = run(sandbox
        .emulate_with(&["--model", "anthropic.claude-global-override"], "plain")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
}

#[test]
fn bare_smah_is_the_honest_notice_exit_2() {
    let sandbox = Sandbox::new();
    let (code, _, stderr) = run(Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", &sandbox.tools)
        .output()
        .expect("run"));
    assert_eq!(code, 2);
    assert!(stderr.contains("not in this build yet"), "{stderr}");
}

// ---------------------------------------------------------------------------
// tools list
// ---------------------------------------------------------------------------

#[test]
fn tools_list_json_reports_valid_and_invalid() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("good", "A fine tool.");
    std::fs::write(
        sandbox.tools.join("broken.md"),
        "---\nname: broken\n---\nbody\n", // no description
    )
    .expect("broken fixture");
    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", &sandbox.tools)
        .args(["tools", "list", "--json"])
        .output()
        .expect("run");
    let (code, stdout, stderr) = run(output);
    assert_eq!(code, 0);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json rows");
    assert_eq!(rows.as_array().expect("rows").len(), 2);
    let broken = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "broken")
        .expect("broken row");
    let good = rows
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["name"] == "good")
        .expect("good row");
    assert_eq!(broken["status"], "invalid");
    assert!(broken["reason"].as_str().unwrap().contains("description"));
    assert_eq!(good["status"], "valid");
    // Invalid files are also warned on stderr.
    assert!(stderr.contains("invalid tool file"), "{stderr}");
}

#[test]
fn tools_list_table_and_empty_dir_notice() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("alpha", "First.");
    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", &sandbox.tools)
        .args(["tools", "list"])
        .output()
        .expect("run");
    let (code, stdout, _) = run(output);
    assert_eq!(code, 0);
    assert!(
        stdout.contains("alpha") && stdout.contains("valid"),
        "{stdout}"
    );

    let empty = tempfile::tempdir().expect("empty dir");
    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", empty.path())
        .args(["tools", "list"])
        .output()
        .expect("run");
    let (code, stdout, stderr) = run(output);
    assert_eq!(code, 0);
    assert!(
        stdout.trim().is_empty() || stdout.contains("NAME"),
        "{stdout}"
    );
    assert!(stderr.contains("no virtual tools installed"), "{stderr}");
}
