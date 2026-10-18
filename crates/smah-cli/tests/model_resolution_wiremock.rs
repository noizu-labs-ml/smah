//! Registry model-name resolution over the wire (real-model e2e find): a
//! `provider.<display name>` spec must resolve to the catalog's raw id
//! before the request leaves, raw ids must pass through untouched, and an
//! unknown name must still be sent raw (provider 404 → exit 5, unchanged).
//! Every key here is fake (`test-key-0000`); nothing touches a real API.

use std::path::PathBuf;

use assert_cmd::Command;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// A per-test sandbox: tools dir, config file, XDG state/cache dirs.
struct Sandbox {
    /// Root tempdir (everything lives under it).
    _root: tempfile::TempDir,
    tools: PathBuf,
    state: PathBuf,
    cache: PathBuf,
    config: PathBuf,
}

impl Sandbox {
    fn new() -> Self {
        let root = tempfile::tempdir().expect("sandbox root");
        let tools = root.path().join("tools");
        let state = root.path().join("state");
        let cache = root.path().join("cache");
        std::fs::create_dir_all(&tools).expect("tools dir");
        std::fs::create_dir_all(state.join("smah")).expect("state dir");
        std::fs::create_dir_all(&cache).expect("cache dir");
        let config = root.path().join("config.yaml");
        std::fs::write(
            &config,
            "vsn: \"0.0.1\"\ninference:\n  vsn: \"0.0.1\"\n  providers:\n    alibaba:\n      name: Alibaba\n      enabled: true\n      settings:\n        api_key: test-key-0000\n      models: []\n",
        )
        .expect("config fixture");
        Sandbox {
            _root: root,
            tools,
            state,
            cache,
            config,
        }
    }

    /// A tool whose frontmatter names the model by display name.
    fn tool(&self, name: &str, model_line: &str) {
        std::fs::write(
            self.tools.join(format!("{name}.md")),
            format!(
                "---\nname: {name}\ndescription: fixture {name}\n{model_line}---\nJust answer.\n"
            ),
        )
        .expect("write tool fixture");
    }

    /// A `smah [global] --emulate-cli <tool>` invocation wired to this
    /// sandbox + the mock alibaba endpoint. Globals must precede the tool.
    fn emulate(&self, global: &[&str], tool: &str, server: &MockServer) -> Command {
        let mut cmd = Command::cargo_bin("smah").expect("smah binary");
        cmd.env("SMAH_VIRTUAL_TOOLS_DIR", &self.tools)
            .env("XDG_STATE_HOME", &self.state)
            .env("XDG_CACHE_HOME", &self.cache)
            .env("ALIBABA_BASE_URL", server.uri())
            .arg("-c")
            .arg(&self.config)
            .args(global)
            .arg("--emulate-cli")
            .arg(tool);
        cmd
    }
}

/// A complete OpenAI-compatible SSE streaming response.
fn qwen_sse(text: &str) -> ResponseTemplate {
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

/// DashScope-style model-not-found.
fn model_not_exist() -> ResponseTemplate {
    ResponseTemplate::new(404).set_body_json(serde_json::json!({
        "code": "InvalidParameter",
        "message": "Model not exist"
    }))
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
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    (code, stdout, stderr)
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
async fn display_name_spec_resolves_to_the_raw_id() {
    let sandbox = Sandbox::new();
    sandbox.tool("named", "model: alibaba.Qwen 3.8 Flash\n");
    let server = MockServer::start().await;
    mount(&server, qwen_sse("ok.\n"), 1).await;

    let output = sandbox
        .emulate(&[], "named", &server)
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "ok.\n");
    assert_eq!(
        first_request_model(&server).await,
        serde_json::json!("qwen3.8-flash"),
        "display name must resolve to the catalog id on the wire"
    );
}

#[tokio::test]
async fn raw_id_spec_passes_through_unchanged() {
    let sandbox = Sandbox::new();
    sandbox.tool("raw", "");
    let server = MockServer::start().await;
    mount(&server, qwen_sse("ok.\n"), 1).await;

    let output = sandbox
        .emulate(&["--model", "alibaba.qwen3.8-flash"], "raw", &server)
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, stdout, stderr) = run(output);

    assert_eq!(code, 0, "stderr: {stderr}");
    assert_eq!(stdout, "ok.\n");
    assert_eq!(
        first_request_model(&server).await,
        serde_json::json!("qwen3.8-flash")
    );
}

#[tokio::test]
async fn unknown_name_is_sent_raw_and_404s_exit_5() {
    let sandbox = Sandbox::new();
    sandbox.tool("mistyped", "");
    let server = MockServer::start().await;
    mount(&server, model_not_exist(), 1).await;

    let output = sandbox
        .emulate(&["--model", "alibaba.NoSuch Model"], "mistyped", &server)
        .write_stdin("")
        .output()
        .expect("run smah");
    let (code, _, stderr) = run(output);

    // No new error class: an unmatched name reaches the provider verbatim
    // and its own 404 maps to exit 5, exactly as before the fix.
    assert_eq!(code, 5, "stderr: {stderr}");
    assert_eq!(
        first_request_model(&server).await,
        serde_json::json!("NoSuch Model"),
        "unknown names must not be rewritten"
    );
}
