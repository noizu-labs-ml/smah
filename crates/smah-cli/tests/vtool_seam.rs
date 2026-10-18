//! PR-0 seam tests (virtual-tools family plan, PR-0 appendix): `--emulate`
//! dispatch, its equivalence with the forced-cli `--emulate-cli` form, the
//! tools-list driver column. Everything here is **lane-stable**: the eight
//! driver lanes replace `drivers/<name>.rs` stubs without touching this
//! file, so no test may pin a stub's refusal output (that is each stub
//! file's own in-file test, which dies with the stub).
//!
//! Sandbox pattern shared with `emulate_integration.rs`; every API key is a
//! fake (`test-key-0000`).

use std::path::PathBuf;
use std::process::Output;

use assert_cmd::Command;
use wiremock::matchers::{body_string_contains, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

struct Sandbox {
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

    /// Install a tool with no model line and no roots (pure persona body).
    fn bare_tool(&self, name: &str, body: &str) {
        std::fs::write(
            self.tools.join(format!("{name}.md")),
            format!("---\nname: {name}\ndescription: fixture {name}\n---\n{body}\n"),
        )
        .expect("write tool fixture");
    }

    /// Install a tool declaring `driver: <driver>`.
    fn driver_tool(&self, name: &str, driver: &str) {
        std::fs::write(
            self.tools.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: fixture {name}\ndriver: {driver}\n---\nbody\n"
            ),
        )
        .expect("write tool fixture");
    }

    fn base(&self) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .arg("-c")
            .arg(&self.config);
        cmd
    }

    /// `smah <global-args...> --emulate <tool>` — global flags precede the
    /// tool; everything after the tool name is tool data.
    fn emulate_with(&self, global: &[&str], tool: &str) -> Command {
        let mut cmd = self.base();
        cmd.args(global).arg("--emulate").arg(tool);
        cmd
    }

    fn audit_log(&self) -> PathBuf {
        self.state.join("smah/audit-virtual-tools.jsonl")
    }
}

fn delta(text: &str) -> String {
    let escaped = text
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n");
    format!(
        "event: content_block_delta\ndata: {{\"type\":\"content_block_delta\",\"index\":0,\"delta\":{{\"type\":\"text_delta\",\"text\":\"{escaped}\"}}}}\n\n"
    )
}

fn sse(text: &str) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .set_body_string(format!(
            "event: message_start\ndata: {{\"type\":\"message_start\",\"message\":{{\"role\":\"assistant\"}}}}\n\n{}event: message_delta\ndata: {{\"type\":\"message_delta\",\"delta\":{{\"stop_reason\":\"end_turn\"}}}}\n\nevent: message_stop\ndata: {{\"type\":\"message_stop\"}}\n\n",
            delta(text)
        ))
        .insert_header("content-type", "text/event-stream")
}

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
// --emulate ≡ --emulate-cli for a cli-driver tool
// ---------------------------------------------------------------------------

#[tokio::test]
async fn emulate_matches_emulate_cli_byte_for_byte() {
    // Two independent sandboxes + wiremock servers with identical scripts:
    // whichever form runs, the transcript must be identical. Byte-for-byte
    // stdout is the PR-0 invariant ("cli behavior unchanged").
    let write_reader = |sandbox: &Sandbox| {
        sandbox.bare_tool("reader", "Read the file.");
        std::fs::write(
            sandbox.tools.join("reader.md"),
            format!(
                "---\nname: reader\ndescription: fixture reader\nmodel: anthropic.claude-fixture-1\nallow_roots:\n  - \"{}\"\n---\nRead the file.\n",
                sandbox.notes.display()
            ),
        )
        .expect("write tool");
        std::fs::write(sandbox.notes.join("data.txt"), "42").expect("data");
    };

    let a = Sandbox::new();
    write_reader(&a);
    let server_a = MockServer::start().await;
    // Turn 1 issues a read directive; turn 2 (carrying the results block)
    // answers with the final text — the loop is what produces the audit row.
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("File surface results"))
        .respond_with(sse("The file says 42.\n"))
        .with_priority(1)
        .mount(&server_a)
        .await;
    mount(&server_a, sse("@@smah read {\"path\": \"data.txt\"}\n"), 1).await;
    let out_a = a
        .base()
        .arg("--emulate")
        .arg("reader")
        .env("ANTHROPIC_BASE_URL", server_a.uri())
        .write_stdin("")
        .output()
        .expect("run smah --emulate");

    let b = Sandbox::new();
    write_reader(&b);
    let server_b = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(body_string_contains("File surface results"))
        .respond_with(sse("The file says 42.\n"))
        .with_priority(1)
        .mount(&server_b)
        .await;
    mount(&server_b, sse("@@smah read {\"path\": \"data.txt\"}\n"), 1).await;
    let out_b = b
        .base()
        .arg("--emulate-cli")
        .arg("reader")
        .env("ANTHROPIC_BASE_URL", server_b.uri())
        .write_stdin("")
        .output()
        .expect("run smah --emulate-cli");

    let (code_a, stdout_a, stderr_a) = run(out_a);
    let (code_b, stdout_b, stderr_b) = run(out_b);
    assert_eq!(code_a, 0, "{stderr_a}");
    assert_eq!(code_b, 0, "{stderr_b}");
    assert_eq!(stdout_a, "The file says 42.\n");
    assert_eq!(stdout_a, stdout_b, "--emulate must match --emulate-cli");
    assert!(!stdout_a.contains("@@smah"));
    for sb in [&a, &b] {
        let audit = std::fs::read_to_string(sb.audit_log()).expect("audit log");
        assert!(audit.contains("\"decision\":\"allowed\""), "{audit}");
    }
}

#[tokio::test]
async fn emulate_cli_forces_the_cli_driver_despite_the_file() {
    // Permanent contract: `--emulate-cli` runs the cli driver even when the
    // file declares another family driver. Here the file says `driver: tui`
    // — today that stub refuses via --emulate; via --emulate-cli it must
    // run as a normal cli tool (exit 0 over a mocked provider).
    let sandbox = Sandbox::new();
    std::fs::write(
        sandbox.tools.join("hybrid.md"),
        format!(
            "---\nname: hybrid\ndescription: fixture hybrid\nmodel: anthropic.claude-fixture-1\nallow_roots:\n  - \"{}\"\ndriver: tui\n---\nbody\n",
            sandbox.notes.display()
        ),
    )
    .expect("write tool");
    let server = MockServer::start().await;
    mount(&server, sse("cli ran.\n"), 1).await;
    let out = sandbox
        .base()
        .arg("--emulate-cli")
        .arg("hybrid")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .write_stdin("")
        .output()
        .expect("run");
    let (code, stdout, stderr) = run(out);
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "cli ran.\n");
}

// ---------------------------------------------------------------------------
// Argument hazards via the new form (plan D6 analogues)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn flags_after_tool_reach_the_model_verbatim_via_emulate() {
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
        .emulate_with(&[], "edit")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .args(["--version", "--unknown", "--", "--help"])
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
}

#[tokio::test]
async fn inline_equals_form_invokes_the_tool() {
    // `--emulate=<tool>` (single argv item) must work like the two-arg form.
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Answer.");
    let server = MockServer::start().await;
    mount(&server, sse("ok.\n"), 1).await;

    let (code, stdout, stderr) = run(sandbox
        .base()
        .arg("--emulate=plain")
        .env("ANTHROPIC_BASE_URL", server.uri())
        .output()
        .expect("run"));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, "ok.\n");
}

// ---------------------------------------------------------------------------
// tools list driver column (PR-0)
// ---------------------------------------------------------------------------

#[test]
fn tools_list_table_carries_the_driver_column() {
    let sandbox = Sandbox::new();
    sandbox.bare_tool("plain", "Default driver.");
    sandbox.driver_tool("panel", "tui");
    std::fs::write(
        sandbox.tools.join("broken.md"),
        "---\nname: broken\ndescription: x\ndriver: fortran\n---\nbody\n",
    )
    .expect("broken fixture");

    let output = Command::cargo_bin("smah")
        .expect("smah binary")
        .env("SMAH_VIRTUAL_TOOLS_DIR", &sandbox.tools)
        .args(["tools", "list"])
        .output()
        .expect("run");
    let (code, stdout, stderr) = run(output);
    assert_eq!(code, 0);
    assert!(stdout.contains("DRIVER"), "{stdout}");
    let plain = stdout
        .lines()
        .find(|l| l.starts_with("plain "))
        .expect("plain row");
    assert!(plain.split_whitespace().any(|c| c == "cli"), "{plain}");
    let panel = stdout
        .lines()
        .find(|l| l.starts_with("panel "))
        .expect("panel row");
    assert!(panel.split_whitespace().any(|c| c == "tui"), "{panel}");
    let broken = stdout
        .lines()
        .find(|l| l.starts_with("broken "))
        .expect("broken row");
    assert!(
        broken.split_whitespace().any(|c| c == "invalid"),
        "{broken}"
    );
    assert!(stderr.contains("unknown driver 'fortran'"), "{stderr}");
}

#[test]
fn tools_list_json_carries_the_driver_field() {
    let sandbox = Sandbox::new();
    sandbox.driver_tool("panel", "tui");
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
    let (code, stdout, _) = run(output);
    assert_eq!(code, 0);
    let rows: serde_json::Value = serde_json::from_str(stdout.trim()).expect("json rows");
    let rows = rows.as_array().expect("rows");
    let panel = rows.iter().find(|r| r["name"] == "panel").expect("panel");
    assert_eq!(panel["driver"], "tui");
    assert_eq!(panel["status"], "valid");
    let broken = rows.iter().find(|r| r["name"] == "broken").expect("broken");
    assert_eq!(broken["status"], "invalid");
    assert!(broken["driver"].is_null(), "invalid rows have no driver");
}
