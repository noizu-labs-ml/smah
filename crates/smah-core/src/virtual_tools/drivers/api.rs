//! The **api** virtual-tool driver (#20): interface-first emulated web APIs.
//!
//! The tool file's `interface:` key declares the contract; the driver makes
//! the model author an `openapi.json` for it (validate-then-freeze), then
//! HOSTS that contract on loopback: `GET /openapi.json` serves the frozen
//! bytes verbatim and every other request is emulated by a bounded
//! fabrication turn against the contract. Print-only throughout — the only
//! listener is loopback HTTP (T-020), and the contract cache is sha-pinned
//! to the tool file (T-021).
//!
//! Threading: the HTTP server runs on a scoped worker thread; fabrication
//! requests round-trip over a channel so every model call happens on the
//! caller's thread (the `&dyn ChatBackend` never has to be `Send`).
//!
//! Driver flags (parsed from the invocation args; everything else is the
//! contract-authoring request): `--port <n>` (default: ephemeral),
//! `--no-serve` (generate/freeze the contract and exit).
//!
//! Output discipline: stdout stays EMPTY (the user is curling a server);
//! every driver message — serving URL included — goes through the notice
//! channel (stderr).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::Value;
use smah_vtool_http::contract::{self, ContractError, ValidatedSpec};
use smah_vtool_http::pins;
use smah_vtool_http::server::{FabOutcome, FabRequest, Fabricate};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{cache_dir, Audit, AuditRecord};
use crate::virtual_tools::driver::{
    invocation_block, ChatBackend, ChatMessage, FsMode, RunOutcome, Sink,
};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::{TURNS_MAX, WRITE_CONTENT_MAX};

/// Generation attempts for the contract (bounded provider spend).
const CONTRACT_ATTEMPTS: usize = 3;

/// Fabrication attempts per emulated request before answering 500.
const FABRICATION_ATTEMPTS: usize = 2;

/// Request-body bytes embedded into a fabrication prompt (bigger bodies are
/// truncated with a note).
const FAB_BODY_MAX: usize = 64 * 1024;

/// Prior-state ledger cap: (endpoint key, ≤512B digest) pairs.
const LEDGER_MAX: usize = 8;
const LEDGER_DIGEST_MAX: usize = 512;

/// Run the api driver.
#[allow(clippy::too_many_arguments)]
pub fn run(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    run_with_dirs(
        tool,
        backend,
        sink,
        model,
        args,
        stdin,
        stdin_partial,
        fs_mode,
        audit,
        run_id,
        TURNS_MAX,
        None,
        None,
    )
}

/// [`run`] with injectable cache root + shutdown signal (tests; production
/// passes `None`/`None`).
#[allow(clippy::too_many_arguments)]
pub fn run_with_dirs(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
    max_turns: usize,
    cache_root: Option<PathBuf>,
    shutdown: Option<Arc<smah_vtool_http::server::ShutdownSignal>>,
) -> Result<RunOutcome, SmahError> {
    let _ = fs_mode; // the api driver has no file surface
    let (port, no_serve, invocation_args) = parse_args(args)?;

    // Interface-first: the tool file must declare the contract.
    let interface = match &tool.interface {
        Some(text) if !text.trim().is_empty() => text.clone(),
        Some(_) => return Err(SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail:
                "interface: is present but empty — the api driver requires a contract description"
                    .to_string(),
        }),
        None => {
            return Err(SmahError::ToolFileInvalid {
                path: tool.path.display().to_string(),
                detail: "the api driver requires an `interface:` key describing the contract"
                    .to_string(),
            })
        }
    };

    // Contract lifecycle: cache + sha-pin to the tool file (T-021).
    let base = cache_root.unwrap_or_else(cache_dir);
    let contract_path = base
        .join("virtual-tools")
        .join(&tool.name)
        .join("api")
        .join("openapi.json");
    let tool_sha = pins::tool_sha256(&tool.path);

    if contract::read_marker(&contract_path).as_deref() == Some(tool_sha.as_str()) {
        let cached = std::fs::read_to_string(&contract_path)
            .map_err(|err| err.to_string())
            .and_then(|text| contract::validate(&text).map_err(|err| err.to_string()));
        match cached {
            Ok(spec) => {
                sink.notice(&format!(
                    "smah(virtual): {} contract cache hit ({} endpoints, sha-pinned to the tool file)",
                    tool.name, spec.path_count
                ));
                return serve_or_dry(
                    tool,
                    backend,
                    sink,
                    audit,
                    run_id,
                    &contract_path,
                    spec,
                    model,
                    port,
                    no_serve,
                    0,
                    shutdown,
                );
            } // Poisoned/wrong-sha/corrupt cache: regenerate — validate-on-load
            // is the gate, the cache is never trusted blindly (T-021).
            Err(reason) => {
                sink.notice(&format!(
                    "smah(virtual): {} contract cache rejected ({reason}); regenerating",
                    tool.name
                ));
            }
        }
    }

    let (spec, turns_used) = generate_contract(
        tool,
        backend,
        sink,
        model,
        &interface,
        &invocation_args,
        stdin,
        stdin_partial,
        audit,
        run_id,
        &contract_path,
        &tool_sha,
        max_turns.min(CONTRACT_ATTEMPTS),
    )?;

    serve_or_dry(
        tool,
        backend,
        sink,
        audit,
        run_id,
        &contract_path,
        spec,
        model,
        port,
        no_serve,
        turns_used,
        shutdown,
    )
}

/// Serve the frozen contract (until ctrl-c / the test signal), or exit as a
/// `--no-serve` dry run.
#[allow(clippy::too_many_arguments)]
fn serve_or_dry(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    audit: &Audit,
    run_id: &str,
    contract_path: &std::path::Path,
    spec: ValidatedSpec,
    model: &str,
    port: u16,
    no_serve: bool,
    turns_used: usize,
    shutdown: Option<Arc<smah_vtool_http::server::ShutdownSignal>>,
) -> Result<RunOutcome, SmahError> {
    if no_serve {
        sink.notice(&format!(
            "smah(virtual): {} contract ready at {} (--no-serve: not starting the server)",
            tool.name,
            contract_path.display()
        ));
        return Ok(RunOutcome {
            turns: turns_used,
            wrote_files: false,
            messages: Vec::new(),
        });
    }

    let spec = Arc::new(spec);
    let spec_summary = contract::summary(&spec);
    let ledger: Arc<Mutex<Vec<(String, String)>>> = Arc::new(Mutex::new(Vec::new()));

    // Fabrication handoff: the server thread relays requests here; every
    // model call happens on the caller's thread (backend is not Send).
    let (req_tx, req_rx) = std::sync::mpsc::channel::<(
        FabRequest,
        std::sync::mpsc::Sender<Result<FabOutcome, String>>,
    )>();
    let (port_tx, port_rx) = std::sync::mpsc::channel::<u16>();

    let fabricate: Fabricate = Arc::new(move |request: FabRequest| {
        let (reply_tx, reply_rx) = std::sync::mpsc::channel();
        req_tx
            .send((request, reply_tx))
            .map_err(|_| "emulation responder is gone".to_string())?;
        reply_rx
            .recv()
            .map_err(|_| "emulation responder dropped the reply".to_string())?
    });

    let tool_name_serve = tool.name.clone();
    let contract_display = contract_path.display().to_string();
    let spec_len = spec.raw.len();

    let outcome: Result<(), SmahError> = std::thread::scope(|scope| {
        scope.spawn(move || {
            let on_bound: Box<dyn FnOnce(u16) + Send> = Box::new(move |actual_port| {
                let _ = port_tx.send(actual_port);
            });
            if let Err(err) = smah_vtool_http::server::serve_blocking(
                port,
                Arc::from(spec.raw.as_str()),
                fabricate,
                shutdown,
                Some(on_bound),
            ) {
                eprintln!("smah(virtual): {tool_name_serve} server error: {err}");
            }
        });

        match port_rx.recv() {
            Ok(actual_port) => {
                sink.notice(&format!(
                    "smah(virtual): {} serving http://127.0.0.1:{actual_port} (ctrl-c to stop)",
                    tool.name
                ));
            }
            Err(_) => {
                // The server thread exited without binding (bind failure).
                return Err(SmahError::ToolFileInvalid {
                    path: contract_display,
                    detail: "could not bind the loopback server (is the port taken?)".to_string(),
                });
            }
        }
        audit_record_named(
            audit,
            run_id,
            &tool.name,
            "api.serve",
            &contract_path.display().to_string(),
            Some(spec_len),
            "allowed",
            "",
        );

        // Responder loop: one bounded fabrication per emulated request, in
        // arrival order. Ends when the server shuts down and drops its
        // closure (ctrl-c, or the test signal).
        for (request, reply) in req_rx {
            let result =
                fabricate_endpoint(request, model, backend, &spec_summary, &tool.name, &ledger);
            let _ = reply.send(result);
        }
        Ok(())
    });
    outcome?;

    audit_record_named(
        audit,
        run_id,
        &tool.name,
        "api.stop",
        &contract_path.display().to_string(),
        None,
        "allowed",
        "",
    );
    sink.notice(&format!("smah(virtual): {} server stopped", tool.name));

    Ok(RunOutcome {
        turns: turns_used,
        wrote_files: false,
        messages: Vec::new(),
    })
}

/// Contract generation loop: up to `attempts` bounded turns, each validated
/// structurally; the first valid contract is frozen (atomic write + sha
/// marker + audit). Returns the spec and the turns consumed.
#[allow(clippy::too_many_arguments)]
fn generate_contract(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    interface: &str,
    invocation_args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    audit: &Audit,
    run_id: &str,
    contract_path: &std::path::Path,
    tool_sha: &str,
    attempts: usize,
) -> Result<(ValidatedSpec, usize), SmahError> {
    let system = generation_prompt(tool, interface);
    let mut messages = vec![ChatMessage::user(invocation_block(
        invocation_args,
        stdin,
        stdin_partial,
    ))];
    let mut last_reason = String::new();

    for attempt in 1..=attempts {
        if attempt > 1 {
            sink.notice(&format!(
                "smah(virtual): {} contract generation attempt {attempt}/{attempts}",
                tool.name
            ));
        }
        let answer = backend.stream_turn(model, &system, &messages, &mut |_piece| {
            // Contract turns are machine-to-machine; nothing streams to the
            // user (stdout must stay clean for a server run).
        })?;
        let candidate = pins::strip_fences(&answer);
        match contract::validate(candidate) {
            Ok(spec) => {
                contract::write_atomic(contract_path, &spec.raw).map_err(|err| {
                    SmahError::ToolFileInvalid {
                        path: contract_path.display().to_string(),
                        detail: format!("cannot write contract cache: {err}"),
                    }
                })?;
                contract::write_marker(contract_path, tool_sha).map_err(|err| {
                    SmahError::ToolFileInvalid {
                        path: contract_path.display().to_string(),
                        detail: format!("cannot write contract marker: {err}"),
                    }
                })?;
                audit_record_named(
                    audit,
                    run_id,
                    &tool.name,
                    "contract.generate",
                    &contract_path.display().to_string(),
                    Some(spec.raw.len()),
                    "allowed",
                    "",
                );
                sink.notice(&format!(
                    "smah(virtual): {} contract frozen: {} ({} endpoints)",
                    tool.name,
                    contract_path.display(),
                    spec.path_count
                ));
                return Ok((spec, attempt));
            }
            Err(reason) => {
                last_reason = contract_error_text(&reason);
                messages.push(ChatMessage::assistant(answer.clone()));
                messages.push(ChatMessage::user(format!(
                    "The contract failed validation: {last_reason}\n\
Respond again with the complete corrected openapi.json — raw JSON only, \
no code fences, no commentary."
                )));
            }
        }
    }

    audit_record_named(
        audit,
        run_id,
        &tool.name,
        "contract.generate",
        &contract_path.display().to_string(),
        None,
        "denied",
        &format!("validation failed after {attempts} attempts: {last_reason}"),
    );
    Err(SmahError::DirectiveLoopOverflow {
        tool: tool.name.clone(),
        turns: attempts,
    })
}

fn contract_error_text(err: &ContractError) -> String {
    err.to_string()
}

/// The code-owned contract-authoring contract.
fn generation_prompt(tool: &ToolDef, interface: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You author OpenAPI contracts for an emulated web API. Respond with the \
complete openapi.json content — raw JSON only, no code fences, no commentary, \
no markdown.\n\n\
Requirements: the root is a JSON object; it has \"openapi\": \"3.1.0\"; an \
\"info\" object with a \"title\"; and a \"paths\" object covering every \
endpoint the contract description below requires. Document request bodies, \
parameters, and responses for each operation with JSON Schema where useful.\n\
Keep the document focused and reasonably small (under 4 MiB; much smaller is \
better).\n\n\
CONTRACT DESCRIPTION (from the tool author — the source of truth for what \
this API does):\n",
    );
    prompt.push_str(interface.trim_end());
    prompt.push_str("\n\n# Tool persona\n\n");
    prompt.push_str(&tool.body);
    prompt.push('\n');
    if !tool.constraints.is_empty() {
        prompt.push_str("\nHard constraints:\n");
        for constraint in &tool.constraints {
            prompt.push_str(&format!("- {constraint}\n"));
        }
    }
    prompt
}

/// One bounded fabrication (with one validation retry) for an emulated
/// request. Runs on the driver's thread.
fn fabricate_endpoint(
    request: FabRequest,
    model: &str,
    backend: &dyn ChatBackend,
    spec_summary: &str,
    tool_name: &str,
    ledger: &Mutex<Vec<(String, String)>>,
) -> Result<FabOutcome, String> {
    let system = fabrication_prompt(spec_summary, tool_name);
    let mut messages = vec![ChatMessage::user(fabrication_request(&request, ledger))];

    for _attempt in 0..FABRICATION_ATTEMPTS {
        let answer = backend
            .stream_turn(model, &system, &messages, &mut |_piece| {})
            .map_err(|err| err.to_string())?;
        match parse_fab_outcome(pins::strip_fences(&answer)) {
            Ok(outcome) => {
                record_ledger(ledger, &request, &outcome.body);
                return Ok(outcome);
            }
            Err(reason) => {
                messages.push(ChatMessage::assistant(answer.clone()));
                messages.push(ChatMessage::user(format!(
                    "Your response was invalid for this protocol: {reason}\n\
Respond again with exactly one JSON object: {{\"status\": <int 200-599>, \
\"body\": <any JSON value>}}."
                )));
            }
        }
    }
    Err("fabrication did not converge (protocol)".to_string())
}

/// The fabrication system contract: frozen summary + response protocol.
fn fabrication_prompt(spec_summary: &str, tool_name: &str) -> String {
    format!(
        "You emulate a web API endpoint for the virtual tool '{tool_name}'. \
The API's contract is frozen (served verbatim at /openapi.json); your job is \
one thing: given a request, produce the JSON response the contract's \
behavior implies. Be internally consistent across requests for the same \
resource.\n\n\
FROZEN CONTRACT SUMMARY: {spec_summary}\n\n\
RESPONSE PROTOCOL — respond with exactly one JSON object and nothing else:\n\
{{\"status\": <HTTP status int, 200-599>, \"body\": <any JSON value>}}\n\
No code fences, no commentary. The body must be valid JSON (objects, arrays, \
strings, numbers, booleans, or null)."
    )
}

/// The fabrication user message: request + capped body + prior-state ledger.
fn fabrication_request(request: &FabRequest, ledger: &Mutex<Vec<(String, String)>>) -> String {
    let mut message = format!("REQUEST: {} {}\n", request.method, request.path_and_query);
    match &request.body {
        Some(body) if body.len() > FAB_BODY_MAX => {
            message.push_str(&format!(
                "BODY (first {FAB_BODY_MAX} of {} bytes, truncated):\n{}\n\n(the body was truncated)",
                body.len(),
                &body[..FAB_BODY_MAX]
            ));
        }
        Some(body) => {
            message.push_str(&format!("BODY:\n{body}\n"));
        }
        None => {
            message.push_str("BODY: (none)\n");
        }
    }
    let ledger = ledger.lock().expect("ledger lock");
    if !ledger.is_empty() {
        message.push_str("\nPRIOR STATE (recent emulated endpoints, oldest first):\n");
        for (key, digest) in ledger.iter() {
            message.push_str(&format!("- {key}: {digest}\n"));
        }
    }
    message
}

fn parse_fab_outcome(answer: &str) -> Result<FabOutcome, String> {
    let value: Value = serde_json::from_str(answer.trim())
        .map_err(|err| format!("response is not valid JSON: {err}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "response must be a JSON object".to_string())?;
    let status = obj
        .get("status")
        .and_then(Value::as_u64)
        .ok_or_else(|| "'status' must be an integer".to_string())?;
    if !(200..=599).contains(&status) {
        return Err(format!("'status' must be 200-599 (got {status})"));
    }
    let body_value = obj.get("body").cloned().unwrap_or(Value::Null);
    let body = serde_json::to_string(&body_value)
        .map_err(|err| format!("'body' is not encodable JSON: {err}"))?;
    if body.len() > WRITE_CONTENT_MAX {
        return Err(format!("'body' exceeds {WRITE_CONTENT_MAX} bytes"));
    }
    Ok(FabOutcome {
        status: status as u16,
        body,
    })
}

fn record_ledger(ledger: &Mutex<Vec<(String, String)>>, request: &FabRequest, body: &str) {
    let key = format!("{} {}", request.method, request.path_and_query);
    let mut digest: String = body.chars().take(LEDGER_DIGEST_MAX).collect();
    if body.len() > LEDGER_DIGEST_MAX {
        digest.push_str("...");
    }
    let mut ledger = ledger.lock().expect("ledger lock");
    if let Some(existing) = ledger.iter_mut().find(|(k, _)| *k == key) {
        existing.1 = digest;
    } else {
        ledger.push((key, digest));
        if ledger.len() > LEDGER_MAX {
            ledger.remove(0);
        }
    }
}

/// Parse driver flags from the invocation args; the remaining args are the
/// contract-authoring request.
fn parse_args(args: &[String]) -> Result<(u16, bool, Vec<String>), SmahError> {
    let mut port: u16 = 0; // 0 = ephemeral
    let mut no_serve = false;
    let mut rest: Vec<String> = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--no-serve" {
            no_serve = true;
        } else if arg == "--port" {
            let value = iter.next().ok_or_else(|| SmahError::Usage {
                message: "--port requires a value (e.g. --port 8080)".to_string(),
            })?;
            port = value.parse().map_err(|_| SmahError::Usage {
                message: format!("--port expects a port number, got '{value}'"),
            })?;
        } else if let Some(value) = arg.strip_prefix("--port=") {
            port = value.parse().map_err(|_| SmahError::Usage {
                message: format!("--port expects a port number, got '{value}'"),
            })?;
        } else {
            rest.push(arg.clone());
        }
    }
    Ok((port, no_serve, rest))
}

/// Metadata-only audit line (no tool object needed; best-effort like the
/// other drivers).
#[allow(clippy::too_many_arguments)]
fn audit_record_named(
    audit: &Audit,
    run_id: &str,
    tool_name: &str,
    op: &str,
    path: &str,
    bytes: Option<usize>,
    decision: &'static str,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: run_id.to_string(),
        tool: tool_name.to_string(),
        op: op.to_string(),
        path: path.to_string(),
        resolved: None,
        bytes,
        decision,
        reason: reason.to_string(),
        turn: 0,
    };
    if let Err(reason) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_tools::driver::Role;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // ------------------------------------------------------------------
    // Doubles
    // ------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct VecSink {
        lines: Rc<RefCell<Vec<String>>>,
    }

    impl VecSink {
        fn notices(&self) -> Vec<String> {
            self.lines.borrow().clone()
        }

        fn find(&self, needle: &str) -> Option<String> {
            self.lines
                .borrow()
                .iter()
                .find(|l| l.contains(needle))
                .cloned()
        }
    }

    impl Sink for VecSink {
        fn content(&mut self, piece: &str) {
            self.lines.borrow_mut().push(piece.to_string());
        }
        fn notice(&mut self, line: &str) {
            self.lines.borrow_mut().push(format!("{line}\n"));
        }
    }

    /// Scripted backend replaying answers (pops from the END).
    struct Scripted {
        answers: std::sync::Mutex<Vec<String>>,
        calls: std::sync::Mutex<Vec<Vec<(Role, String)>>>,
        systems: std::sync::Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(answers: Vec<String>) -> Self {
            Scripted {
                answers: std::sync::Mutex::new(answers),
                calls: std::sync::Mutex::new(Vec::new()),
                systems: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn last_messages(&self) -> Vec<(Role, String)> {
            self.calls
                .lock()
                .expect("lock")
                .last()
                .cloned()
                .unwrap_or_default()
        }

        fn last_system(&self) -> String {
            self.systems
                .lock()
                .expect("lock")
                .last()
                .cloned()
                .unwrap_or_default()
        }
    }

    impl ChatBackend for Scripted {
        fn stream_turn(
            &self,
            model: &str,
            system: &str,
            messages: &[ChatMessage],
            _on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, SmahError> {
            let answer = self
                .answers
                .lock()
                .expect("lock")
                .pop()
                .expect("Scripted: more model turns requested than scripted");
            self.systems.lock().expect("lock").push(system.to_string());
            self.calls.lock().expect("lock").push(
                messages
                    .iter()
                    .map(|m| (m.role, m.content.clone()))
                    .collect(),
            );
            let _ = model;
            Ok(answer)
        }
    }

    /// Sink forwarding lines over a channel (for the serving thread).
    struct ChanSink(mpsc::Sender<String>);

    impl Sink for ChanSink {
        fn content(&mut self, _piece: &str) {}
        fn notice(&mut self, line: &str) {
            let _ = self.0.send(format!("{line}\n"));
        }
    }

    fn tool(interface: Option<String>) -> ToolDef {
        ToolDef {
            name: "widget-api".to_string(),
            description: "test".to_string(),
            model: None,
            driver: "api".to_string(),
            interface,
            constraints: vec![],
            allow_roots: vec![],
            body: "Widget API persona.".to_string(),
            path: PathBuf::from("widget-api.md"),
        }
    }

    fn write_tool_file(dir: &std::path::Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "tool file bytes for sha").expect("write");
        path
    }

    const GOOD_SPEC: &str = r#"{"openapi":"3.1.0","info":{"title":"Widgets","version":"1.0"},"paths":{"/widgets":{"get":{"responses":{}}}}}"#;

    fn tmp_audit(tag: &str) -> (tempfile::TempDir, Audit) {
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join(format!("{tag}.jsonl")), "run");
        (dir, audit)
    }

    fn audit_text(dir: &tempfile::TempDir, tag: &str) -> String {
        std::fs::read_to_string(dir.path().join(format!("{tag}.jsonl"))).unwrap_or_default()
    }

    // ------------------------------------------------------------------
    // Unit
    // ------------------------------------------------------------------

    #[test]
    fn parse_args_flags_and_rest() {
        let (port, no_serve, rest) = parse_args(&[
            "--port".into(),
            "8080".into(),
            "list".into(),
            "widgets".into(),
        ])
        .expect("parse");
        assert_eq!(port, 8080);
        assert!(!no_serve);
        assert_eq!(rest, vec!["list".to_string(), "widgets".to_string()]);

        let (port, no_serve, rest) =
            parse_args(&["--port=9090".into(), "--no-serve".into()]).expect("parse");
        assert_eq!(port, 9090);
        assert!(no_serve);
        assert!(rest.is_empty());

        let (port, _, _) = parse_args(&[]).expect("parse");
        assert_eq!(port, 0);

        assert!(parse_args(&["--port".into(), "http".into()]).is_err());
        assert!(parse_args(&["--port".into()]).is_err());
    }

    #[test]
    fn fab_outcome_parsing() {
        let ok = parse_fab_outcome("{\"status\": 201, \"body\": {\"id\": 7}}").expect("ok");
        assert_eq!(ok.status, 201);
        assert_eq!(ok.body, "{\"id\":7}");

        let null_body = parse_fab_outcome("{\"status\": 204}").expect("ok");
        assert_eq!(null_body.body, "null");

        assert!(parse_fab_outcome("{\"status\": 42, \"body\": null}").is_err());
        assert!(parse_fab_outcome("{\"status\": \"x\"}").is_err());
        assert!(parse_fab_outcome("[1,2]").is_err());
        assert!(parse_fab_outcome("not json").is_err());
        let huge = parse_fab_outcome(&format!(
            "{{\"status\":200,\"body\":\"{}\"}}",
            "x".repeat(WRITE_CONTENT_MAX + 1)
        ));
        assert!(huge.is_err());
    }

    #[test]
    fn ledger_updates_in_place_and_caps() {
        let ledger = Mutex::new(Vec::new());
        let req = |path: &str| FabRequest {
            method: "GET".into(),
            path_and_query: path.into(),
            body: None,
        };
        record_ledger(&ledger, &req("/a"), "{\"x\":1}");
        record_ledger(&ledger, &req("/b"), "{\"y\":2}");
        record_ledger(&ledger, &req("/a"), "{\"x\":2}");
        let snapshot = ledger.lock().expect("lock").clone();
        assert_eq!(snapshot.len(), 2);
        assert_eq!(snapshot[0], ("GET /a".to_string(), "{\"x\":2}".to_string()));

        for i in 0..LEDGER_MAX + 3 {
            record_ledger(&ledger, &req(&format!("/p{i}")), "null");
        }
        let snapshot = ledger.lock().expect("lock");
        assert_eq!(snapshot.len(), LEDGER_MAX);
        assert!(snapshot.iter().all(|(k, _)| !k.contains("/a")));
    }

    #[test]
    fn requires_interface() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = write_tool_file(dir.path(), "widget-api.md");
        let mut t = tool(None);
        t.path = path;
        let backend = Scripted::new(vec![]);
        let mut sink = VecSink::default();
        let (_d, audit) = tmp_audit("iface");
        let err = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
            4,
            Some(dir.path().join("cache")),
            None,
        )
        .expect_err("interface required");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));

        let mut t_empty = tool(Some("   ".to_string()));
        t_empty.path = write_tool_file(dir.path(), "widget-api.md");
        let err = run_with_dirs(
            &t_empty,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
            4,
            Some(dir.path().join("cache")),
            None,
        )
        .expect_err("empty interface");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    }

    #[test]
    fn no_serve_dry_run_generates_without_serving() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool(Some("a CRUD API for widgets".to_string()));
        t.path = write_tool_file(tool_dir.path(), "widget-api.md");
        let (_d, audit) = tmp_audit("dry");
        let backend = Scripted::new(vec![GOOD_SPEC.to_string()]);
        let mut sink = VecSink::default();
        let out = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-dry",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect("dry run");
        assert!(sink.find("--no-serve: not starting the server").is_some());
        let contract_path = cache
            .path()
            .join("virtual-tools")
            .join("widget-api")
            .join("api")
            .join("openapi.json");
        assert_eq!(
            std::fs::read_to_string(&contract_path).expect("contract"),
            GOOD_SPEC
        );
        assert!(contract::read_marker(&contract_path).is_some());
        let text = audit_text(&_d, "dry");
        assert!(text.contains("\"op\":\"contract.generate\""), "{text}");
        assert!(!text.contains("\"op\":\"api.serve\""), "{text}");
        assert_eq!(out.turns, 1);
    }

    #[test]
    fn generation_failure_after_retries_is_loop_overflow() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool(Some("a CRUD API for widgets".to_string()));
        t.path = write_tool_file(tool_dir.path(), "widget-api.md");
        let (_d, audit) = tmp_audit("fail");
        let bad = "{\"openapi\": \"3.1.0\", \"paths\": \"/oops\"}".to_string();
        let backend = Scripted::new(vec![bad.clone(), bad.clone(), bad]);
        let mut sink = VecSink::default();
        let err = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-fail",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect_err("generation failed");
        assert!(
            matches!(err, SmahError::DirectiveLoopOverflow { .. }),
            "{err:?}"
        );
        let text = audit_text(&_d, "fail");
        assert!(text.contains("\"decision\":\"denied\""), "{text}");
        assert!(
            text.contains("validation failed after 3 attempts"),
            "{text}"
        );
    }

    #[test]
    fn oversized_contract_in_cache_is_regenerated() {
        let cache = tempfile::tempdir().expect("cache");
        let contract_path = cache
            .path()
            .join("virtual-tools")
            .join("widget-api")
            .join("api")
            .join("openapi.json");
        std::fs::create_dir_all(contract_path.parent().expect("parent")).expect("dirs");
        std::fs::write(&contract_path, "this is not json").expect("poison");
        // Marker pinned to the REAL tool-file sha: the cache is "current"
        // per the marker, so validate-on-load must reject the corrupt body
        // (marker mismatch would instead be a silent, by-design regen).
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool(Some("a CRUD API for widgets".to_string()));
        t.path = write_tool_file(tool_dir.path(), "widget-api.md");
        contract::write_marker(&contract_path, &pins::tool_sha256(&t.path)).expect("marker");

        let (_d, audit) = tmp_audit("poison");
        let backend = Scripted::new(vec![GOOD_SPEC.to_string()]);
        let mut sink = VecSink::default();
        let out = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-poison",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect("regenerated");
        assert!(sink.find("contract cache rejected").is_some());
        assert_eq!(
            std::fs::read_to_string(&contract_path).expect("read"),
            GOOD_SPEC
        );
        assert_eq!(out.turns, 1);
    }

    // ------------------------------------------------------------------
    // Serve loop: generation + freeze + real loopback HTTP
    // ------------------------------------------------------------------

    /// Run the driver (serve mode) on a worker thread; the test thread
    /// collects notices over a channel, waits for the serving line, and
    /// returns the pieces the assertions need.
    struct Served {
        port: u16,
        sink: VecSink,
        backend: Arc<Scripted>,
        audit_dir: tempfile::TempDir,
        contract_path: PathBuf,
        shutdown: Arc<smah_vtool_http::server::ShutdownSignal>,
    }

    impl Served {
        /// Signal shutdown and wait (deterministically) for the driver to
        /// finish its stop audit + final notices.
        fn stop(&self) {
            self.shutdown.notify_one();
            let audit_file = self.audit_dir.path().join("serve.jsonl");
            let deadline = Instant::now() + Duration::from_secs(5);
            while Instant::now() < deadline {
                if std::fs::read_to_string(&audit_file)
                    .unwrap_or_default()
                    .contains("\"op\":\"api.stop\"")
                {
                    return;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            panic!("driver did not record api.stop within 5s");
        }
    }

    fn serve_with(answers: Vec<String>, cache: &std::path::Path) -> Served {
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool(Some("a CRUD API for widgets".to_string()));
        t.path = write_tool_file(tool_dir.path(), "widget-api.md");
        let t = Arc::new(t);

        let audit_dir = tempfile::tempdir().expect("auditdir");
        let backend = Arc::new(Scripted::new(answers));
        let contract_path = cache
            .join("virtual-tools")
            .join("widget-api")
            .join("api")
            .join("openapi.json");

        let (line_tx, line_rx) = mpsc::channel::<String>();
        let shutdown = Arc::new(smah_vtool_http::server::ShutdownSignal::new());
        let shutdown_for_run = shutdown.clone();

        let cache_root = cache.to_path_buf();
        let sink_for_run = ChanSink(line_tx);
        let t_run = t;
        let backend_run = backend.clone();
        let audit_path = audit_dir.path().join("serve.jsonl");
        std::thread::spawn(move || {
            // Owns everything it touches; the driver joins its own server
            // thread before returning.
            run_with_dirs_inner(
                t_run,
                backend_run,
                sink_for_run,
                cache_root,
                shutdown_for_run,
                audit_path,
            );
        });

        // Collect notices until the serving line arrives; keep them all.
        let deadline = Instant::now() + Duration::from_secs(15);
        let sink = VecSink::default();
        let mut port = None;
        while port.is_none() {
            let line = line_rx
                .recv_timeout(Duration::from_secs(1))
                .expect("notices until serving line");
            sink.lines.borrow_mut().push(line.clone());
            if let Some(rest) = line.split("serving http://127.0.0.1:").nth(1) {
                port = rest
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .and_then(|p| p.parse().ok());
            }
            if Instant::now() > deadline {
                panic!("no serving notice; last line: {line}");
            }
        }
        // Drain any further notices into the sink (non-blocking).
        while let Ok(line) = line_rx.try_recv() {
            sink.lines.borrow_mut().push(line);
        }

        Served {
            port: port.expect("port"),
            sink,
            backend,
            audit_dir,
            contract_path,
            shutdown,
        }
    }

    /// Thin wrapper so the spawned thread owns everything it touches.
    fn run_with_dirs_inner(
        tool: Arc<ToolDef>,
        backend: Arc<Scripted>,
        mut sink: ChanSink,
        cache_root: PathBuf,
        shutdown: Arc<smah_vtool_http::server::ShutdownSignal>,
        audit_path: PathBuf,
    ) {
        let audit = Audit::open_at(&audit_path, "run");
        let _ = run_with_dirs(
            &tool,
            backend.as_ref(),
            &mut sink,
            "m",
            &[],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-serve",
            4,
            Some(cache_root),
            Some(shutdown),
        );
    }

    fn http(port: u16, request: &str) -> String {
        use std::io::{Read, Write};
        let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(10)))
            .expect("timeout");
        stream.write_all(request.as_bytes()).expect("write");
        let mut buf = String::new();
        let _ = stream.read_to_string(&mut buf);
        buf
    }

    #[test]
    fn generates_freezes_and_serves() {
        let cache = tempfile::tempdir().expect("cache");
        // Pops from the END: contract answer last, fabrication answer first.
        let fabrication =
            "{\"status\": 200, \"body\": [{\"id\": 1, \"name\": \"alpha\"}]}".to_string();
        let served = serve_with(vec![fabrication, GOOD_SPEC.to_string()], cache.path());

        // Contract served verbatim.
        let raw = http(
            served.port,
            "GET /openapi.json HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(raw.contains("\"openapi\":\"3.1.0\""), "{raw}");
        assert!(
            raw.to_ascii_lowercase()
                .contains("content-type: application/json"),
            "{raw}"
        );
        // Driver notices reached the sink: frozen contract + serving line.
        assert!(
            served.sink.find("contract frozen").is_some(),
            "{:?}",
            served.sink.notices()
        );
        assert!(
            served
                .sink
                .notices()
                .iter()
                .any(|l| l.contains("serving http://127.0.0.1:")),
            "{:?}",
            served.sink.notices()
        );

        // Emulated endpoint answered from the fabrication turn.
        let raw = http(
            served.port,
            "GET /widgets HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert!(raw.starts_with("HTTP/1.1 200"), "{raw}");
        assert!(raw.contains("alpha"), "{raw}");

        // Fabrication turn: frozen summary in the system prompt, request in
        // the user message.
        assert!(
            served
                .backend
                .last_system()
                .contains("FROZEN CONTRACT SUMMARY"),
            "{}",
            served.backend.last_system()
        );
        let msgs = served.backend.last_messages();
        assert!(
            msgs[0].1.contains("REQUEST: GET /widgets"),
            "{:?}",
            msgs[0].1
        );
        assert!(!msgs[0].1.contains("PRIOR STATE"), "{:?}", msgs[0].1);

        // Audit: contract.generate allowed + api.serve.
        let text = audit_text(&served.audit_dir, "serve");
        assert!(text.contains("\"op\":\"contract.generate\""), "{text}");
        assert!(text.contains("\"decision\":\"allowed\""), "{text}");
        assert!(text.contains("\"op\":\"api.serve\""), "{text}");

        // Contract + marker on disk.
        let on_disk = std::fs::read_to_string(&served.contract_path).expect("contract");
        assert_eq!(contract::validate(&on_disk).expect("valid").raw, GOOD_SPEC);
        assert!(contract::read_marker(&served.contract_path).is_some());

        served.stop();
        // After stop, api.stop is audited.
        let text = audit_text(&served.audit_dir, "serve");
        assert!(text.contains("\"op\":\"api.stop\""), "{text}");
    }

    #[test]
    fn cache_hit_serves_without_model_turn() {
        let cache = tempfile::tempdir().expect("cache");
        {
            let served = serve_with(vec![GOOD_SPEC.to_string()], cache.path());
            served.stop();
        }
        // Second run with an EMPTY scripted backend: any model call would
        // panic the worker — reaching the serving line proves the cache hit.
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool(Some("a CRUD API for widgets".to_string()));
        t.path = write_tool_file(tool_dir.path(), "widget-api.md");
        let (_d, audit) = tmp_audit("hit");
        let backend = Scripted::new(vec![]);
        let mut sink = VecSink::default();
        let out = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-hit",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect("cache-hit dry run");
        assert!(
            sink.notices()
                .iter()
                .any(|l| l.contains("contract cache hit")),
            "{:?}",
            sink.notices()
        );
        assert_eq!(out.turns, 0);
        let text = audit_text(&_d, "hit");
        assert!(!text.contains("\"op\":\"contract.generate\""), "{text}");
    }

    // ------------------------------------------------------------------
    // Real-model e2e (env-gated, #[ignore], never in CI)
    // ------------------------------------------------------------------

    /// ```text
    /// SMAH_ALIBABA_API_KEY=... [ALIBABA_BASE_URL=...] \
    ///   cargo test -p smah-core -- --ignored api_real_model_contract
    /// ```
    #[test]
    #[ignore = "real-model e2e: set SMAH_ALIBABA_API_KEY (+ ALIBABA_BASE_URL) to run"]
    fn api_real_model_contract() {
        let key = std::env::var("SMAH_ALIBABA_API_KEY").unwrap_or_default();
        if key.is_empty() {
            eprintln!("skipping: SMAH_ALIBABA_API_KEY not set");
            return;
        }
        let base_url = std::env::var("ALIBABA_BASE_URL").unwrap_or_default();
        let provider = if base_url.is_empty() {
            smah_providers::AlibabaProvider::new(Some(key))
        } else {
            smah_providers::AlibabaProvider::with_base_url(Some(key), base_url)
        }
        .expect("provider");

        struct Adapter(smah_providers::AlibabaProvider);
        impl ChatBackend for Adapter {
            fn stream_turn(
                &self,
                model: &str,
                system: &str,
                messages: &[ChatMessage],
                on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                use smah_providers::Provider;
                let mut req_messages: Vec<smah_providers::ChatMessage> = messages
                    .iter()
                    .map(|m| match m.role {
                        Role::User => smah_providers::ChatMessage::user(m.content.clone()),
                        Role::Assistant => {
                            smah_providers::ChatMessage::assistant(m.content.clone())
                        }
                    })
                    .collect();
                req_messages.push(smah_providers::ChatMessage::system(system.to_string()));
                let req = smah_providers::CompletionRequest::new(model, req_messages);
                let completion =
                    self.0
                        .complete(&req)
                        .map_err(|err| SmahError::DirectiveProtocol {
                            tool: "api".to_string(),
                            detail: format!("provider error: {err}"),
                        })?;
                on_delta(completion.text.trim());
                Ok(completion.text)
            }
        }

        let dir = tempfile::tempdir().expect("tempdir");
        let mut t = tool(Some(
            "A small REST API for managing widgets: list widgets (GET /widgets), \
create one (POST /widgets with a JSON body), fetch one (GET /widgets/{id}), \
delete one (DELETE /widgets/{id}). JSON only."
                .to_string(),
        ));
        t.path = write_tool_file(dir.path(), "widget-api.md");
        let (_d, audit) = tmp_audit("e2e");
        let backend = Adapter(provider);
        let mut sink = VecSink::default();
        let cache = tempfile::tempdir().expect("cache");
        run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "alibaba.qwen3.8-flash",
            &["--no-serve".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-e2e",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect("real-model contract generation");
        let contract_path = cache
            .path()
            .join("virtual-tools")
            .join("widget-api")
            .join("api")
            .join("openapi.json");
        let raw = std::fs::read_to_string(&contract_path).expect("contract");
        let spec = contract::validate(&raw).expect("contract validates");
        assert!(spec.path_count >= 3, "expected the widget endpoints");
        for line in sink.notices() {
            eprintln!("{line}");
        }
    }
}
