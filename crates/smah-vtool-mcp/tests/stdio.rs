//! Black-box integration tests for the mcp driver (ticket #23).
//!
//! Drives `smah_core::virtual_tools::drivers::mcp::run` exactly the way
//! `smah-cli` does: a scripted backend stands in for the model, the tool
//! file lives in a hermetic tempdir (`XDG_CACHE_HOME` repointed), and the
//! default batch-stdio transport is fed newline-delimited JSON-RPC 2.0.

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};

use smah_core::errors::SmahError;
use smah_core::virtual_tools::audit::Audit;
use smah_core::virtual_tools::drivers::mcp;
use smah_core::virtual_tools::drivers::{ChatBackend, ChatMessage, FsMode, Sink};
use smah_core::virtual_tools::loader::ToolDef;

/// Backend whose replies are consumed in order; records every turn so tests
/// can assert how many model calls a run actually made.
#[derive(Default)]
struct Scripted {
    replies: Mutex<Vec<String>>,
    calls: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(replies: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
            calls: Mutex::new(Vec::new()),
        })
    }
}

impl ChatBackend for Scripted {
    fn stream_turn(
        &self,
        _model: &str,
        system: &str,
        _messages: &[ChatMessage],
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        self.calls.lock().expect("calls").push(system.to_string());
        let mut replies = self.replies.lock().expect("replies");
        if replies.is_empty() {
            return Err(SmahError::ProviderRequest {
                provider: "scripted".to_string(),
                code: 0,
                detail: "script exhausted".to_string(),
            });
        }
        Ok(replies.remove(0))
    }
}

/// Sink collecting stdout content lines and stderr notices.
#[derive(Default)]
struct Collect {
    content: Vec<String>,
    notices: Vec<String>,
}

impl Sink for Collect {
    fn content(&mut self, piece: &str) {
        self.content.push(piece.to_string());
    }
    fn notice(&mut self, line: &str) {
        self.notices.push(line.to_string());
    }
}

/// Process-wide lock: tests repoint `XDG_CACHE_HOME` (process-global env),
/// so test bodies serialize for the lifetime of their tempdir.
static ENV: Mutex<()> = Mutex::new(());

/// A tool file in a hermetic cache dir; repoints `XDG_CACHE_HOME` so the
/// fabricated catalog lands in the tempdir, not the user's real cache.
/// Returns the env guard — hold it for the whole test body.
fn hermetic_tool(name: &str) -> (tempfile::TempDir, ToolDef, MutexGuard<'static, ()>) {
    let guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("XDG_CACHE_HOME", dir.path());
    let tool_path = dir.path().join(format!("{name}.md"));
    std::fs::write(&tool_path, "# crm\nSearch contacts.\n").expect("tool file");
    let tool = ToolDef {
        name: name.to_string(),
        description: "search contacts".to_string(),
        model: None,
        driver: "mcp".to_string(),
        interface: None,
        constraints: Vec::new(),
        allow_roots: Vec::new(),
        body: "You are the crm persona.".to_string(),
        path: tool_path,
    };
    (dir, tool, guard)
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

/// Parse the sink's content pieces into JSON-RPC response values.
fn responses(sink: &Collect) -> Vec<Value> {
    sink.content
        .iter()
        .flat_map(|piece| piece.lines())
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line.trim()).expect("json-rpc line"))
        .collect()
}

#[test]
fn batch_stdio_session_freezes_catalog_answers_and_memoizes() {
    let (dir, tool, _env) = hermetic_tool("crm");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

    // Turn 1: the fabricated tool catalog. Turn 2: the tools/call answer.
    let backend = Scripted::new(vec![
        json!([{
            "name": "search",
            "description": "search the contact list",
            "inputSchema": {
                "type": "object",
                "properties": {"q": {"type": "string"}},
                "required": ["q"]
            }
        }])
        .to_string(),
        "found 2 contacts: ada, grace".to_string(),
    ]);

    let stdin_text = format!(
        "{}\n{}\n{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
               "params": {"protocolVersion": "2025-06-18", "capabilities": {}}}),
        json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "search", "arguments": {"q": "ada"}}}),
    );

    let mut sink = Collect::default();
    let outcome = mcp::run(
        &tool,
        backend.as_ref(),
        &mut sink,
        "m",
        &args(&[]),
        Some(&stdin_text),
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect("batch session");

    // Both fabrication turns are reported: one for the catalog, one for
    // the tools/call answer.
    assert_eq!(outcome.turns, 2, "catalog + call fabrication turns");
    assert!(!outcome.wrote_files);
    let systems = backend.calls.lock().expect("calls").clone();
    assert_eq!(systems.len(), 2, "catalog + call turn; systems={systems:?}");

    let lines = responses(&sink);
    assert_eq!(
        lines.len(),
        3,
        "one response per request, none for the notification"
    );

    // initialize: protocol version echoed, server named after the tool.
    assert_eq!(lines[0]["id"], json!(1));
    assert_eq!(lines[0]["result"]["protocolVersion"], json!("2025-06-18"));
    assert_eq!(
        lines[0]["result"]["serverInfo"]["name"],
        json!("smah-vtool-crm")
    );

    // tools/list: the frozen catalog entry, wire-shaped.
    assert_eq!(lines[1]["id"], json!(2));
    assert_eq!(lines[1]["result"]["tools"][0]["name"], json!("search"));

    // tools/call: the fabricated text, wrapped as MCP content.
    assert_eq!(lines[2]["id"], json!(3));
    assert_eq!(
        lines[2]["result"]["content"][0]["text"],
        json!("found 2 contacts: ada, grace")
    );
    assert_eq!(lines[2]["result"]["isError"], json!(false));

    // The frozen catalog + memo cache landed in the hermetic leaf dir.
    let leaf = dir.path().join("smah/virtual-tools/crm/mcp");
    assert!(leaf.join("mcp-tools.json").is_file(), "frozen catalog");
    assert!(leaf.join("mcp.sha256").is_file(), "digest marker");
    let memo = std::fs::read_to_string(leaf.join("mcp-memo.jsonl")).expect("memo");
    assert!(
        memo.contains("found 2 contacts"),
        "memo carries the fabricated answer"
    );

    // A second identical call must be answered from the memo — no third turn.
    let stdin_again = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/call",
        "params": {"name": "search", "arguments": {"q": "ada"}}})
    .to_string();
    let mut sink_again = Collect::default();
    mcp::run(
        &tool,
        backend.as_ref(),
        &mut sink_again,
        "m",
        &args(&[]),
        Some(&format!("{stdin_again}\n")),
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect("memoized call");
    let lines_again = responses(&sink_again);
    assert_eq!(lines_again.len(), 1);
    assert_eq!(
        lines_again[0]["result"]["content"][0]["text"],
        json!("found 2 contacts: ada, grace")
    );
    assert_eq!(
        backend.calls.lock().expect("calls").len(),
        2,
        "no new model turn for the memo hit"
    );
}

#[test]
fn unknown_flag_refuses_before_any_model_call() {
    let (_dir, tool, _env) = hermetic_tool("crm2");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");
    let backend = Scripted::new(Vec::new());

    let err = mcp::run(
        &tool,
        backend.as_ref(),
        &mut Collect::default(),
        "m",
        &args(&["--fusion"]),
        None,
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect_err("unknown flag");

    assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    assert_eq!(err.exit_code(), smah_core::exit_codes::ExitCode::Config);
    assert!(err.to_string().contains("mcp driver"));
    assert!(
        backend.calls.lock().expect("calls").is_empty(),
        "no model call"
    );
}

/// ChatBackend adapter over the real Alibaba (DashScope) provider, for the
/// env-gated e2e smoke below. Lives in the test, never in crate src.
struct RealBackend(smah_providers::AlibabaProvider);

impl ChatBackend for RealBackend {
    fn stream_turn(
        &self,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        use smah_core::virtual_tools::drivers::Role;
        let mut wire = vec![smah_providers::ChatMessage::system(system)];
        for message in messages {
            let converted = match message.role {
                Role::User => smah_providers::ChatMessage::user(message.content.clone()),
                Role::Assistant => smah_providers::ChatMessage::assistant(message.content.clone()),
            };
            wire.push(converted);
        }
        let mut request = smah_providers::CompletionRequest::new(model, wire);
        request.max_tokens = Some(2048);
        let completion = use_provider_complete(&self.0, &request)?;
        on_delta(&completion);
        Ok(completion)
    }
}

/// `Provider::complete` via the trait (kept tiny so the adapter stays readably flat).
fn use_provider_complete(
    provider: &smah_providers::AlibabaProvider,
    request: &smah_providers::CompletionRequest,
) -> Result<String, SmahError> {
    use smah_providers::Provider as _;
    provider.complete(request).map(|completion| completion.text)
}

/// Real-model e2e: one DashScope turn freezes a catalog, one answers a call.
/// Runs ONLY when `SMAH_E2E_DASHSCOPE=1` AND `SMAH_ALIBABA_API_KEY` are set;
/// skips cleanly otherwise (CI never sets the gate). The key is never printed.
#[test]
#[ignore = "real-endpoint smoke: requires SMAH_E2E_DASHSCOPE=1 + SMAH_ALIBABA_API_KEY"]
fn alibaba_real_fabrication_smoke() {
    let gated = std::env::var("SMAH_E2E_DASHSCOPE")
        .map(|value| value.trim() == "1")
        .unwrap_or(false);
    let api_key = std::env::var("SMAH_ALIBABA_API_KEY")
        .ok()
        .filter(|key| !key.trim().is_empty());
    if !gated || api_key.is_none() {
        eprintln!("skipping alibaba_real_fabrication_smoke: gate env not set");
        return;
    }

    let (_env_guard, dir, tool) = {
        let guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
        let dir = tempfile::tempdir().expect("tempdir");
        std::env::set_var("XDG_CACHE_HOME", dir.path());
        let tool_path = dir.path().join("crm.md");
        std::fs::write(
            &tool_path,
            "# crm\nAnswer questions about a small contact list.\n",
        )
        .expect("tool file");
        let tool = ToolDef {
            name: "crm".to_string(),
            description: "answers contact questions over a virtual MCP server".to_string(),
            model: None,
            driver: "mcp".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "You fabricate plausible contact-search tool catalogs.".to_string(),
            path: tool_path,
        };
        (guard, dir, tool)
    };
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

    let provider = smah_providers::AlibabaProvider::new(api_key).expect("construct provider");
    let backend = RealBackend(provider);

    let stdin_text = format!(
        "{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
    );
    let mut sink = Collect::default();
    let outcome = mcp::run(
        &tool,
        &backend,
        &mut sink,
        "qwen3.8-flash",
        &args(&[]),
        Some(&stdin_text),
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect("real batch session");
    assert_eq!(outcome.turns, 1, "one catalog fabrication turn");

    let lines = responses(&sink);
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[1]["id"], json!(2));
    let tools = lines[1]["result"]["tools"].as_array().expect("tools array");
    assert!(
        !tools.is_empty(),
        "real model fabricated a non-empty catalog"
    );

    let leaf = dir.path().join("smah/virtual-tools/crm/mcp");
    assert!(leaf.join("mcp-tools.json").is_file(), "catalog frozen");
}
