//! Black-box integration tests for the vfs driver (ticket #25).
//!
//! A scripted backend fabricates the tree; the batch stdio JSON-RPC
//! transport is exercised in-process through the same `run(...)` seam the
//! `smah-cli` dispatch uses. The hermetic-cache env lock mirrors the mcp
//! lane's harness (`XDG_CACHE_HOME` is process-global).

use std::sync::{Arc, Mutex, MutexGuard};

use serde_json::{json, Value};

use smah_core::errors::SmahError;
use smah_core::virtual_tools::audit::Audit;
use smah_core::virtual_tools::drivers::vfs;
use smah_core::virtual_tools::drivers::{ChatBackend, ChatMessage, FsMode, Sink};
use smah_core::virtual_tools::loader::ToolDef;

/// Process-wide lock: tests repoint `XDG_CACHE_HOME` (process-global env).
static ENV: Mutex<()> = Mutex::new(());

/// Backend whose replies are consumed in order; records every turn.
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

fn hermetic_tool(name: &str) -> (tempfile::TempDir, ToolDef, MutexGuard<'static, ()>) {
    let guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let dir = tempfile::tempdir().expect("tempdir");
    std::env::set_var("XDG_CACHE_HOME", dir.path());
    let tool_path = dir.path().join(format!("{name}.md"));
    std::fs::write(&tool_path, "# docs\nKeep the team's documents.\n").expect("tool file");
    let tool = ToolDef {
        name: name.to_string(),
        description: "keeps the team's documents".to_string(),
        model: None,
        driver: "vfs".to_string(),
        interface: None,
        constraints: Vec::new(),
        allow_roots: Vec::new(),
        body: "You are the docs persona.".to_string(),
        path: tool_path,
    };
    (dir, tool, guard)
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

fn responses(sink: &Collect) -> Vec<Value> {
    sink.content
        .iter()
        .flat_map(|piece| piece.lines())
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line.trim()).expect("json-rpc line"))
        .collect()
}

#[test]
fn batch_stdio_session_fabricates_tree_and_serves_fs_tools() {
    let (dir, tool, _env) = hermetic_tool("docs");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

    // Turn 1: startup fabrication (the tree). Turn 2: an fs_generate reply.
    let backend = Scripted::new(vec![
        json!({
            "files": [
                {"path": "README.md", "content": "hello"},
                {"path": "etc/users.csv", "content": "ada\ngrace\n"}
            ]
        })
        .to_string(),
        json!({"files": [{"path": "more/a.txt", "content": "A"}]}).to_string(),
    ]);

    let stdin_text = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}}),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list"}),
        json!({"jsonrpc": "2.0", "id": 3, "method": "tools/call",
               "params": {"name": "fs_read", "arguments": {"path": "README.md"}}}),
        json!({"jsonrpc": "2.0", "id": 4, "method": "tools/call",
               "params": {"name": "fs_write",
                          "arguments": {"path": "notes/x.txt", "content": "woven"}}}),
        // Escape attempt: refused per request, server keeps serving.
        json!({"jsonrpc": "2.0", "id": 5, "method": "tools/call",
               "params": {"name": "fs_read", "arguments": {"path": "../../etc/passwd"}}}),
        json!({"jsonrpc": "2.0", "id": 6, "method": "tools/call",
               "params": {"name": "fs_generate", "arguments": {"hint": "a couple of logs"}}}),
    );

    let mut sink = Collect::default();
    let outcome = vfs::run(
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

    assert_eq!(outcome.turns, 2, "startup fabrication + fs_generate turn");
    assert!(outcome.wrote_files, "the tree was written");

    let lines = responses(&sink);
    assert_eq!(lines.len(), 6, "one response per request");

    assert_eq!(lines[0]["id"], json!(1));
    assert_eq!(
        lines[0]["result"]["serverInfo"]["name"],
        json!("smah-vtool-vfs-docs")
    );

    // The catalog is the code-owned fs tool set, in fixed order.
    assert_eq!(lines[1]["id"], json!(2));
    let names: Vec<&str> = lines[1]["result"]["tools"]
        .as_array()
        .expect("tools array")
        .iter()
        .map(|tool| tool["name"].as_str().expect("tool name"))
        .collect();
    assert_eq!(
        names,
        vec!["fs_list", "fs_read", "fs_write", "fs_stat", "fs_generate"],
        "fixed catalog, never fabricated"
    );

    assert_eq!(lines[2]["id"], json!(3));
    assert_eq!(lines[2]["result"]["content"][0]["text"], json!("hello"));

    assert_eq!(lines[3]["id"], json!(4));
    assert_eq!(
        lines[3]["result"]["content"][0]["text"],
        json!("wrote 5 bytes to notes/x.txt")
    );

    assert_eq!(lines[4]["id"], json!(5));
    assert_eq!(
        lines[4]["error"]["code"],
        json!(-32602),
        "escape attempt refused per request (INVALID_PARAMS)"
    );
    assert!(lines[4]["error"]["message"]
        .as_str()
        .expect("reason")
        .contains(".."),);

    assert_eq!(lines[5]["id"], json!(6));
    assert_eq!(
        lines[5]["result"]["content"][0]["text"],
        json!("added 1 file(s): more/a.txt")
    );

    // The tree really is on disk under the hermetic cache leaf.
    let root = dir.path().join("smah/virtual-tools/docs/vfs/root");
    assert_eq!(
        std::fs::read_to_string(root.join("notes/x.txt")).expect("written file"),
        "woven"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("more/a.txt")).expect("generated file"),
        "A"
    );
    assert!(dir
        .path()
        .join("smah/virtual-tools/docs/vfs/manifest.json")
        .is_file());
    assert!(dir
        .path()
        .join("smah/virtual-tools/docs/vfs/vfs.sha256")
        .is_file());

    // Both fabrication turns hit the model exactly once each.
    assert_eq!(backend.calls.lock().expect("calls").len(), 2);
}

#[test]
fn escape_in_startup_fabrication_aborts_with_directive_violation() {
    let (_dir, tool, _env) = hermetic_tool("docs2");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

    // The model tries to escape the tree during startup fabrication.
    let backend = Scripted::new(vec![json!({
        "files": [{"path": "../../outside.txt", "content": "nope"}]
    })
    .to_string()]);

    let err = vfs::run(
        &tool,
        backend.as_ref(),
        &mut Collect::default(),
        "m",
        &args(&[]),
        None,
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect_err("fabrication escape");

    assert!(
        matches!(err, SmahError::DirectiveViolation { .. }),
        "escape aborts the run: {err}"
    );
    assert_eq!(err.exit_code(), smah_core::exit_codes::ExitCode::Config);
}

#[test]
fn unknown_flag_refuses_before_any_model_call() {
    let (_dir, tool, _env) = hermetic_tool("docs3");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");
    let backend = Scripted::new(Vec::new());

    let err = vfs::run(
        &tool,
        backend.as_ref(),
        &mut Collect::default(),
        "m",
        &args(&["--bogus"]),
        None,
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect_err("unknown flag");

    assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    assert_eq!(err.exit_code(), smah_core::exit_codes::ExitCode::Config);
    assert!(err.to_string().contains("vfs driver"));
    assert!(
        backend.calls.lock().expect("calls").is_empty(),
        "no model call"
    );
}

/// Real-model e2e: one DashScope turn fabricates the tree, fs_read serves it.
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
        let tool_path = dir.path().join("docs.md");
        std::fs::write(&tool_path, "# docs\nA tiny documentation set.\n").expect("tool file");
        let tool = ToolDef {
            name: "docs".to_string(),
            description: "serves a tiny documentation set over the vfs driver".to_string(),
            model: None,
            driver: "vfs".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "You fabricate small plausible documentation trees.".to_string(),
            path: tool_path,
        };
        (guard, dir, tool)
    };
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

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
                    Role::Assistant => {
                        smah_providers::ChatMessage::assistant(message.content.clone())
                    }
                };
                wire.push(converted);
            }
            let mut request = smah_providers::CompletionRequest::new(model, wire);
            request.max_tokens = Some(2048);
            let text = {
                use smah_providers::Provider as _;
                self.0.complete(&request).expect("real completion").text
            };
            on_delta(&text);
            Ok(text)
        }
    }
    let backend = RealBackend(smah_providers::AlibabaProvider::new(api_key).expect("provider"));

    let stdin_text = json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": {"name": "fs_list", "arguments": {"path": ""}}
    })
    .to_string();
    let mut sink = Collect::default();
    vfs::run(
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

    let lines = responses(&sink);
    assert_eq!(lines.len(), 1);
    let listing = lines[0]["result"]["content"][0]["text"]
        .as_str()
        .expect("listing text");
    assert!(
        !listing.trim().is_empty(),
        "real model fabricated a non-empty tree"
    );

    let root = dir.path().join("smah/virtual-tools/docs/vfs/root");
    assert!(root.is_dir(), "tree exists on disk");
}
