//! gRPC virtual-tool driver (ticket #21) — interface-first, unary-only v1.
//!
//! The tool file's `interface:` declares the `.proto` (inline or
//! path/`$VAR`). The driver compiles it at load time with `protox` (pure
//! Rust — no `protoc` binary), validates the shape through
//! `prost-reflect`'s descriptor pool (exactly one service, capped methods,
//! at least one unary method), and freezes it with sha pins. Each incoming
//! unary call is decoded dynamically against the declared input message and
//! answered by asking the model to fabricate the declared output message in
//! protojson form — parsed back through the typed descriptor before it goes
//! on the wire. One regeneration on drift; a still-drifting fabrication
//! becomes a sanitized `INTERNAL` status, never a silent lie. Streaming
//! methods get a clear `UNIMPLEMENTED` refusal (v1 scope).
//!
//! Surface: loopback-only listener (B7). stderr carries the loud disclosure
//! line; `--no-serve` validates/freezes without binding. Dispatch runs on an
//! axum fallback handler that routes `/package.Service/Method` to tonic's
//! server-side `Grpc::unary` with a dynamic codec — no codegen, and tonic's
//! framing/compression/limits stay stock.
//!
//! Threading: identical to the SOAP lane — the `&dyn ChatBackend` borrow is
//! not `Send`, so the tokio runtime runs on a spawned server thread while
//! `run`'s own thread is the fabricator, fed by a std mpsc event channel;
//! replies ride a tokio oneshot per call. Fabrications are serialized (v1).

#[path = "grpc_codec.rs"]
mod codec;
#[path = "grpc_contract.rs"]
mod contract;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use prost_reflect::DynamicMessage;
use tonic::server::Grpc;
use tonic::{Request as TonicRequest, Response as TonicResponse, Status};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{Audit, AuditRecord};
use crate::virtual_tools::drivers::{ChatBackend, ChatMessage, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::{TURNS_MAX, WRITE_CONTENT_MAX};

use contract::{load_or_freeze, GrpcContract, GrpcMethod};

/// Per-call fabricated-response cap (mirrors the family write cap).
const RESPONSE_MAX: usize = WRITE_CONTENT_MAX;
/// Inbound message cap (tonic decode limit).
const REQUEST_MAX: usize = 1024 * 1024;
/// Fabrication turns per call (initial attempt + one bounded regen).
const TURNS_PER_REQUEST: usize = 2;
/// Prior request/response pairs kept in the fabrication context.
const LEDGER_PAIRS: usize = 6;
/// Bytes of each ledger side kept after trimming.
const LEDGER_SNIPPET: usize = 2 * 1024;
/// Bytes of the request JSON shown to the model.
const REQUEST_SNIPPET: usize = 16 * 1024;

#[derive(Debug)]
struct GrpcArgs {
    port: u16,
    no_serve: bool,
    max_serve_seconds: Option<u64>,
}

fn parse_args(tool: &ToolDef, args: &[String], sink: &mut dyn Sink) -> Result<GrpcArgs, SmahError> {
    let mut out = GrpcArgs {
        port: 0,
        no_serve: false,
        max_serve_seconds: None,
    };
    let mut i = 0;
    while i < args.len() {
        let (flag, inline) = split_flag(&args[i]);
        match flag {
            "--port" => {
                let value = flag_value(args, &mut i, "port", inline)?;
                out.port = value
                    .parse()
                    .map_err(|_| usage(&format!("--port expects a port number, got {value:?}")))?;
            }
            "--max-serve-seconds" => {
                let value = flag_value(args, &mut i, "max-serve-seconds", inline)?;
                out.max_serve_seconds = Some(value.parse().map_err(|_| {
                    usage(&format!(
                        "--max-serve-seconds expects seconds, got {value:?}"
                    ))
                })?);
            }
            "--no-serve" => {
                if inline.is_some() {
                    return Err(usage("--no-serve takes no value"));
                }
                out.no_serve = true;
            }
            other => {
                sink.notice(&format!(
                    "smah(virtual): {} flag {other:?} is not a grpc-driver flag \
                     (--port, --no-serve, --max-serve-seconds)",
                    tool.name
                ));
                return Err(usage(&format!(
                    "unknown grpc-driver flag {other:?} (supported: --port, --no-serve, \
                     --max-serve-seconds)"
                )));
            }
        }
        i += 1;
    }
    Ok(out)
}

fn split_flag(arg: &str) -> (&str, Option<&str>) {
    match arg.split_once('=') {
        Some((flag, value)) => (flag, Some(value)),
        None => (arg, None),
    }
}

/// Consume the value for a value-taking flag (inline `--flag=v` or the next
/// argument), advancing the cursor.
fn flag_value(
    args: &[String],
    i: &mut usize,
    flag: &str,
    inline: Option<&str>,
) -> Result<String, SmahError> {
    if let Some(v) = inline {
        return Ok(v.to_string());
    }
    *i += 1;
    args.get(*i)
        .cloned()
        .ok_or_else(|| usage(&format!("--{flag} requires a value")))
}

fn usage(detail: &str) -> SmahError {
    SmahError::Usage {
        message: format!("smah --emulate <grpc-tool>: {detail}"),
    }
}

/// Run the grpc driver: validate/freeze the contract, then (unless
/// `--no-serve`) serve the declared unary methods on a loopback listener
/// until ctrl-c / the serve deadline.
#[allow(clippy::too_many_arguments)]
pub fn run(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    _stdin: Option<&str>,
    _stdin_partial: bool,
    _fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    let parsed = parse_args(tool, args, sink)?;

    // Interface-first: no contract, no service.
    let contract = load_or_freeze(tool, audit)?;
    // The descriptor pool for the frozen proto (typed dispatch below).
    let pool = contract::compile_pool(tool, &contract.proto)?;
    let unary = contract.unary_methods().len();
    sink.notice(&format!(
        "smah(virtual): {} contract frozen (service {}; {unary} unary method(s); \
         streaming methods refused in v1)",
        tool.name, contract.service_name
    ));

    if parsed.no_serve {
        sink.notice(&format!(
            "smah(virtual): {} dry run (--no-serve): contract valid, nothing bound",
            tool.name
        ));
        return Ok(RunOutcome {
            turns: 0,
            wrote_files: false,
            messages: Vec::new(),
        });
    }

    serve(
        tool, backend, sink, model, contract, pool, parsed, audit, run_id,
    )
}

/// Events flowing from the server thread (and its handlers) back to the
/// fabricator thread that owns the backend borrow.
enum Event {
    Fabricate(FabricateJob),
    Notice(String),
    Done(Result<(), SmahError>),
}

struct FabricateJob {
    method: String,
    request_json: String,
    reply: tokio::sync::oneshot::Sender<Result<String, String>>,
}

// Mirrors the soap driver's serve(): the driver seam fixes this shape.
#[allow(clippy::too_many_arguments)]
fn serve(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    contract: GrpcContract,
    pool: prost_reflect::DescriptorPool,
    parsed: GrpcArgs,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| SmahError::Internal {
            detail: format!("grpc driver: tokio runtime: {e}"),
        })?;

    let (event_tx, event_rx) = std::sync::mpsc::channel::<Event>();
    let turns = Arc::new(AtomicUsize::new(0));

    let state = Arc::new(ServeState {
        tool_name: tool.name.clone(),
        contract,
        pool,
        event_tx: event_tx.clone(),
        turns: Arc::clone(&turns),
        audit: audit.clone(),
        run_id: run_id.to_string(),
    });

    // The server thread owns the runtime; every backend-touching step stays
    // on this thread.
    let server_thread = {
        let event_tx = event_tx.clone();
        let server_state = Arc::clone(&state);
        let max_serve_seconds = parsed.max_serve_seconds;
        std::thread::spawn(move || {
            let notice_tx = event_tx.clone();
            let result = runtime.block_on(async move {
                let app = axum::Router::new()
                    .fallback(dispatch)
                    .with_state(server_state);
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", parsed.port))
                    .await
                    .map_err(|e| SmahError::Internal {
                        detail: format!("grpc driver: loopback bind failed: {e}"),
                    })?;
                let bound = listener
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "127.0.0.1".to_string());
                if !bound.starts_with("127.0.0.1") {
                    return Err(SmahError::Internal {
                        detail: format!("grpc driver: refusing non-loopback bind {bound}"),
                    });
                }
                let _ = notice_tx.send(Event::Notice(format!(
                    "serving 127.0.0.1 (grpc, http/2 prior knowledge) on {bound} (ctrl-c to \
                     stop) — VIRTUAL grpc service: all responses are model fabrications, \
                     never real upstream data"
                )));

                // 'static required by with_graceful_shutdown — own the deadline.
                let shutdown = async move {
                    let ctrl = async {
                        let _ = tokio::signal::ctrl_c().await;
                    };
                    let deadline = async move {
                        match max_serve_seconds {
                            Some(secs) => tokio::time::sleep(Duration::from_secs(secs)).await,
                            None => std::future::pending::<()>().await,
                        }
                    };
                    tokio::select! {
                        _ = ctrl => {},
                        _ = deadline => {},
                    }
                };

                axum::serve(listener, app)
                    .with_graceful_shutdown(shutdown)
                    .await
                    .map_err(|e| SmahError::Internal {
                        detail: format!("grpc driver: serve failed: {e}"),
                    })
            });
            let _ = event_tx.send(Event::Done(result));
        })
    };

    // Fabricator loop: owns the backend borrow, the exchange ledger, and the
    // first fatal provider error seen mid-serve.
    let mut ledger: Vec<(String, String)> = Vec::new();
    let mut fatal: Option<SmahError> = None;
    let mut serve_result: Option<Result<(), SmahError>> = None;
    while let Ok(event) = event_rx.recv() {
        match event {
            Event::Fabricate(job) => {
                turns.fetch_add(1, Ordering::SeqCst);
                match fabricate(
                    tool,
                    backend,
                    model,
                    &state.contract,
                    &state.pool,
                    &job.method,
                    &job.request_json,
                    &ledger,
                ) {
                    Ok(reply_json) => {
                        ledger.push((job.request_json.clone(), reply_json.clone()));
                        let excess = ledger.len().saturating_sub(LEDGER_PAIRS);
                        ledger.drain(..excess);
                        let _ = job.reply.send(Ok(reply_json));
                    }
                    Err(err) => {
                        if fatal.is_none() {
                            fatal = Some(err.clone());
                        }
                        let _ = job.reply.send(Err(err.to_string()));
                    }
                }
            }
            Event::Notice(text) => sink.notice(&text),
            Event::Done(result) => {
                serve_result = Some(result);
                break;
            }
        }
    }
    let _ = server_thread.join();

    let turns_used = turns.load(Ordering::SeqCst);
    sink.notice(&format!(
        "smah(virtual): {} grpc service stopped ({turns_used} fabrication turns)",
        tool.name
    ));

    let served = serve_result.ok_or_else(|| SmahError::Internal {
        detail: "grpc driver: server thread ended unexpectedly".to_string(),
    })?;
    served?;

    // A fatal provider problem observed mid-serve becomes the run's exit.
    if let Some(err) = fatal {
        return Err(err);
    }
    Ok(RunOutcome {
        turns: turns_used,
        wrote_files: false,
        messages: Vec::new(),
    })
}

struct ServeState {
    tool_name: String,
    contract: GrpcContract,
    pool: prost_reflect::DescriptorPool,
    event_tx: std::sync::mpsc::Sender<Event>,
    turns: Arc<AtomicUsize>,
    audit: Audit,
    run_id: String,
}

/// Every path lands here: `/package.Service/Method` dispatches a unary call
/// through tonic; everything else is refused trailers-only.
async fn dispatch(
    axum::extract::State(state): axum::extract::State<Arc<ServeState>>,
    request: axum::extract::Request,
) -> axum::response::Response {
    let path = request.uri().path().to_string();
    let (service, method_name) = match parse_grpc_path(&path) {
        Ok(parts) => parts,
        Err(reason) => {
            audit_call(&state, &path, 0, "denied", &reason);
            return grpc_refusal(&Status::unimplemented(reason));
        }
    };

    if service != state.contract.service_name {
        let reason = format!(
            "unknown service {service:?}; this virtual tool declares {:?}",
            state.contract.service_name
        );
        audit_call(&state, &path, 0, "denied", &reason);
        eprintln!("smah(virtual): {} grpc refusal: {reason}", state.tool_name);
        return grpc_refusal(&Status::unimplemented(reason));
    }

    let Some(method) = state.contract.method(&method_name).cloned() else {
        let reason = format!(
            "unknown method {method_name:?}; declared: {}",
            state
                .contract
                .methods
                .iter()
                .map(|m| m.name.clone())
                .collect::<Vec<_>>()
                .join(", ")
        );
        audit_call(&state, &path, 0, "denied", &reason);
        eprintln!("smah(virtual): {} grpc refusal: {reason}", state.tool_name);
        return grpc_refusal(&Status::unimplemented(reason));
    };

    if !method.unary() {
        let reason = format!(
            "method {:?} streams — the smah grpc virtual tool is unary-only in v1",
            method.name
        );
        audit_call(&state, &path, 0, "denied", &reason);
        eprintln!("smah(virtual): {} grpc refusal: {reason}", state.tool_name);
        return grpc_refusal(&Status::unimplemented(reason));
    }

    let (input, output) = match (
        state.pool.get_message_by_name(&method.input),
        state.pool.get_message_by_name(&method.output),
    ) {
        (Some(input), Some(output)) => (input, output),
        _ => {
            let reason = format!(
                "descriptor missing for {:?} (contract pool out of sync)",
                method.name
            );
            audit_call(&state, &path, 0, "denied", &reason);
            return grpc_refusal(&Status::internal(reason));
        }
    };

    // Stock tonic unary machinery: framing, compression headers, decode and
    // encode limits all apply as in generated servers.
    let mut grpc = Grpc::new(codec::DynamicCodec::new(input, output))
        .max_decoding_message_size(REQUEST_MAX)
        .max_encoding_message_size(RESPONSE_MAX);
    let response = grpc
        .unary(
            DynamicUnary {
                state: Arc::clone(&state),
                method,
            },
            request,
        )
        .await;

    if response
        .headers()
        .get("grpc-status")
        .is_none_or(|v| v.as_bytes() == b"0")
    {
        audit_call(&state, &path, 0, "allowed", "");
    } else {
        audit_call(
            &state,
            &path,
            0,
            "denied",
            "call returned a grpc error status",
        );
    }

    // tonic returned an http::Response<tonic::body::Body>; axum takes any
    // http_body body.
    response.map(axum::body::Body::new)
}

/// Split `/package.Service/Method` into its two segments.
fn parse_grpc_path(path: &str) -> Result<(String, String), String> {
    let segments: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    match segments.as_slice() {
        [service, method] if !service.is_empty() && !method.is_empty() => {
            Ok(((*service).to_string(), (*method).to_string()))
        }
        _ => Err(format!(
            "expected /package.Service/Method, got {path:?} — this is a gRPC endpoint, \
             not a browser page"
        )),
    }
}

/// The per-call unary service: hands the decoded request to the fabricator
/// thread and parses the fabricated protojson back into the typed output
/// message. Implementing `UnaryService` directly (no `tower::Service`
/// blanket impl on this type) keeps the fabricator hop explicit.
struct DynamicUnary {
    state: Arc<ServeState>,
    method: GrpcMethod,
}

impl tonic::server::UnaryService<DynamicMessage> for DynamicUnary {
    type Response = DynamicMessage;
    type Future = std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<TonicResponse<DynamicMessage>, Status>> + Send>,
    >;

    fn call(&mut self, request: TonicRequest<DynamicMessage>) -> Self::Future {
        let state = Arc::clone(&self.state);
        let method = self.method.clone();
        Box::pin(async move { handle_unary(&state, &method, request).await })
    }
}

async fn handle_unary(
    state: &ServeState,
    method: &GrpcMethod,
    request: TonicRequest<DynamicMessage>,
) -> Result<TonicResponse<DynamicMessage>, Status> {
    // The declared output message the model must fabricate.
    let output = state
        .pool
        .get_message_by_name(&method.output)
        .ok_or_else(|| Status::internal("output descriptor missing"))?;

    // protojson of the request, for the fabrication context.
    let request_json = serde_json::to_string_pretty(request.get_ref()).map_err(|e| {
        Status::internal(format!(
            "smah grpc virtual tool: request render failed: {e}"
        ))
    })?;
    let request_json = truncate_chars(&request_json, REQUEST_SNIPPET);

    // Hand the fabrication to the thread that owns the backend borrow.
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    state
        .event_tx
        .send(Event::Fabricate(FabricateJob {
            method: method.name.clone(),
            request_json: request_json.clone(),
            reply: reply_tx,
        }))
        .map_err(|_| {
            Status::unavailable("smah grpc virtual tool: fabrication worker is unavailable")
        })?;
    let fabricated = reply_rx.await.map_err(|_| {
        Status::unavailable("smah grpc virtual tool: fabrication worker dropped the reply")
    })?;

    let reply_json = match fabricated {
        Ok(json) => json,
        Err(reason) => {
            // Keep details off the wire; they go to stderr via the run loop.
            eprintln!(
                "smah(virtual): {} grpc fabrication failed for {}: {reason}",
                state.tool_name, method.name
            );
            return Err(Status::internal(format!(
                "smah grpc virtual tool: fabrication failed for method {:?}",
                method.name
            )));
        }
    };

    // Typed validation: the reply must parse as the declared output message,
    // unknown fields refused (prost-reflect's protojson default).
    let mut deserializer = serde_json::Deserializer::from_str(&reply_json);
    let message = DynamicMessage::deserialize(output.clone(), &mut deserializer).map_err(|e| {
        eprintln!(
            "smah(virtual): {} grpc fabricated reply rejected for {}: {e}",
            state.tool_name, method.name
        );
        Status::internal(format!(
            "smah grpc virtual tool: fabricated reply rejected by the frozen contract \
                 for method {:?}",
            method.name
        ))
    })?;
    deserializer.end().map_err(|_| {
        Status::internal("smah grpc virtual tool: trailing bytes after fabricated reply")
    })?;
    if reply_json.len() > RESPONSE_MAX {
        return Err(Status::internal(
            "smah grpc virtual tool: fabricated reply exceeds the response cap",
        ));
    }

    Ok(TonicResponse::new(message))
}

/// A trailers-only gRPC response: status headers on an empty 200. The
/// message is percent-encoded per the gRPC HTTP/2 spec.
fn grpc_refusal(status: &Status) -> axum::response::Response {
    let mut builder = axum::http::Response::builder()
        .status(axum::http::StatusCode::OK)
        .header("content-type", "application/grpc")
        .header("grpc-status", (status.code() as i32).to_string());
    if !status.message().is_empty() {
        builder = builder.header("grpc-message", percent_encode(status.message()));
    }
    builder
        .body(axum::body::Body::empty())
        .expect("static response")
}

fn percent_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b' ' => out.push_str("%20"),
            b'%' => out.push_str("%25"),
            0x21..=0x7E => out.push(byte as char),
            other => out.push_str(&format!("%{other:02X}")),
        }
    }
    out
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let cut: String = text.chars().take(max).collect();
    format!("{cut}\n(… truncated at {max} chars)")
}

/// Metadata-only audit for one served call (method/size/decision — never
/// request or response contents).
fn audit_call(state: &ServeState, path: &str, bytes: usize, decision: &'static str, reason: &str) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: state.run_id.clone(),
        tool: state.tool_name.clone(),
        op: "grpc.call".to_string(),
        path: path.to_string(),
        resolved: None,
        bytes: Some(bytes),
        decision,
        reason: reason.to_string(),
        turn: state.turns.load(Ordering::SeqCst),
    };
    if let Err(reason) = state.audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

/// One fabrication chain on the fabricator thread (owns the backend):
/// initial attempt + one bounded regeneration on contract drift — the loop
/// is capped by `TURNS_PER_REQUEST`, itself bounded by `TURNS_MAX`.
#[allow(clippy::too_many_arguments)]
fn fabricate(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    model: &str,
    contract: &GrpcContract,
    pool: &prost_reflect::DescriptorPool,
    method: &str,
    request_json: &str,
    ledger: &[(String, String)],
) -> Result<String, SmahError> {
    let system = system_prompt(tool, contract);
    let mut user = format!(
        "gRPC request received.\nMethod: {method}{}\nRequest message (protojson):\n{request_json}\n",
        contract
            .method(method)
            .map(|m| format!(
                " (declared input: {}; declared output: {})",
                m.input, m.output
            ))
            .unwrap_or_default()
    );
    if !ledger.is_empty() {
        user.push_str("\nPrior exchanges on this service (oldest first):\n");
        for (req, resp) in ledger {
            user.push_str(&format!(
                "<exchange>\n{}</exchange>\n<exchange>\n{}</exchange>\n",
                truncate_chars(req, LEDGER_SNIPPET),
                truncate_chars(resp, LEDGER_SNIPPET)
            ));
        }
    }
    user.push_str(
        "\nProduce ONLY the protojson response message for this method, consistent with \
         the contract and the prior exchanges. No code fences, no commentary.\n",
    );
    let messages = vec![ChatMessage::user(user)];

    let mut feedback: Option<String> = None;
    for _attempt in 0..TURNS_PER_REQUEST {
        let mut full_messages = messages.clone();
        if let Some(reason) = &feedback {
            full_messages.push(ChatMessage::assistant(
                "{\"fabricated\": \"…\"}".to_string(),
            ));
            full_messages.push(ChatMessage::user(format!(
                "Your previous reply was refused by the frozen contract: {reason}\n\
                 Reply again with ONLY the corrected protojson response message."
            )));
        }
        let reply = backend.stream_turn(model, &system, &full_messages, &mut |_| {})?;
        let candidate = strip_fences(reply.trim());
        match validate_reply(&candidate, pool, contract, method) {
            Ok(()) => return Ok(candidate),
            Err(reason) => feedback = Some(reason),
        }
    }
    Err(SmahError::DirectiveLoopOverflow {
        tool: tool.name.clone(),
        turns: TURNS_PER_REQUEST.min(TURNS_MAX),
    })
}

/// Typed validation of a fabricated reply on the fabricator side: it must
/// parse as the declared output message (strict protojson — unknown fields
/// refused — and size-capped). Returns the refusal reason on drift so the
/// regen attempt can quote it.
fn validate_reply(
    candidate: &str,
    pool: &prost_reflect::DescriptorPool,
    contract: &GrpcContract,
    method: &str,
) -> Result<(), String> {
    if candidate.len() > RESPONSE_MAX {
        return Err(format!("reply exceeds the {RESPONSE_MAX}-byte cap"));
    }
    let declared = contract
        .method(method)
        .ok_or_else(|| format!("method {method:?} is not declared in the contract"))?;
    let output = pool
        .get_message_by_name(&declared.output)
        .ok_or_else(|| format!("output type {} missing from the pool", declared.output))?;
    let mut de = serde_json::Deserializer::from_str(candidate);
    DynamicMessage::deserialize(output, &mut de).map_err(|e| e.to_string())?;
    de.end()
        .map_err(|e| format!("trailing bytes after JSON: {e}"))
}

fn strip_fences(text: &str) -> String {
    if !text.contains("```") {
        return text.to_string();
    }
    // Drop the fence lines themselves; the fenced content is the payload.
    let mut out = String::new();
    for line in text.lines() {
        if line.trim_start().starts_with("```") {
            continue;
        }
        out.push_str(line);
        out.push('\n');
    }
    out.trim().to_string()
}

fn system_prompt(tool: &ToolDef, contract: &GrpcContract) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "RESERVED TOKEN: the line prefix \"@@smah \" is reserved by the host program. \
Never emit it.\n\n",
    );
    prompt.push_str(
        "You are the implementation behind a VIRTUAL gRPC service used for contract \
testing and local development. The service contract below is FROZEN — it defines \
exactly which methods exist and what their request/response messages look like. \
Fabricate responses that a real service implementing this contract could truthfully \
return. Never claim to have contacted a real upstream.\n\n",
    );
    prompt.push_str(&format!(
        "CONTRACT (frozen): service {}; methods: {}\n",
        contract.service_name,
        contract
            .methods
            .iter()
            .map(|m| {
                let kind = if m.unary() {
                    "unary"
                } else {
                    "streaming (refused in v1)"
                };
                format!("{}({}) -> {} [{kind}]", m.name, m.input, m.output)
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    prompt.push_str(
        "\nOUTPUT RULES:\n- Reply with ONLY the protojson response message for \
the requested method (field names exactly as declared; no code fences, no \
commentary).\n- The reply is parsed against the frozen contract's typed descriptor; \
unknown fields or a wrong shape are refused and you get one bounded retry.\n",
    );

    prompt.push_str("\n# Tool persona\n\n");
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Sink;
    impl crate::virtual_tools::drivers::Sink for Sink {
        fn content(&mut self, _: &str) {}
        fn notice(&mut self, _: &str) {}
    }

    fn test_tool() -> ToolDef {
        ToolDef {
            name: "grpctest".to_string(),
            description: "d".to_string(),
            model: None,
            driver: "grpc".to_string(),
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            interface: None,
            body: String::new(),
            path: std::path::PathBuf::from("grpctest.md"),
        }
    }

    #[test]
    fn args_parse_flags_and_reject_unknowns() {
        let mut sink = Sink;
        let args: Vec<String> = ["--port", "8080", "--no-serve"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = parse_args(&test_tool(), &args, &mut sink).expect("parse");
        assert_eq!(parsed.port, 8080);
        assert!(parsed.no_serve);
        assert_eq!(parsed.max_serve_seconds, None);

        let args: Vec<String> = ["--port=9090", "--max-serve-seconds=5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = parse_args(&test_tool(), &args, &mut sink).expect("parse");
        assert_eq!(parsed.port, 9090);
        assert_eq!(parsed.max_serve_seconds, Some(5));

        let err = parse_args(&test_tool(), &["--bogus".to_string()], &mut sink)
            .expect_err("unknown flag");
        assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Usage);
        assert!(err.to_string().contains("--bogus"), "{err}");
    }

    #[test]
    fn grpc_paths_split_and_refuse_garbage() {
        assert_eq!(
            parse_grpc_path("/demo.v1.Greeter/Ping").ok(),
            Some(("demo.v1.Greeter".to_string(), "Ping".to_string()))
        );
        assert_eq!(
            parse_grpc_path("demo.v1.Greeter/Ping").ok(),
            Some(("demo.v1.Greeter".to_string(), "Ping".to_string()))
        );
        assert!(parse_grpc_path("/").is_err());
        assert!(parse_grpc_path("/only-service").is_err());
        assert!(parse_grpc_path("/a/b/c").is_err());
    }

    #[test]
    fn percent_encoding_matches_grpc_spec() {
        assert_eq!(percent_encode("ok"), "ok");
        assert_eq!(percent_encode("a b"), "a%20b");
        assert_eq!(percent_encode("100%"), "100%25");
        assert_eq!(percent_encode("café"), "caf%C3%A9");
    }

    #[test]
    fn refusal_carries_grpc_status_headers() {
        let response = grpc_refusal(&Status::unimplemented("unary-only v1"));
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("grpc-status")
                .and_then(|v| v.to_str().ok()),
            Some("12")
        );
        assert_eq!(
            response
                .headers()
                .get("grpc-message")
                .and_then(|v| v.to_str().ok()),
            Some("unary-only%20v1")
        );
    }

    #[test]
    fn fabricated_replies_validate_against_the_declared_output() {
        let tool = test_tool();
        let contract = contract::validate(&tool, PROTO).expect("contract");
        let pool = contract::compile_pool(&tool, PROTO).expect("pool");
        // Valid reply for the declared output type.
        validate_reply(r#"{"message": "hi"}"#, &pool, &contract, "Ping").expect("valid reply");
        // Unknown field → refused (strict protojson).
        let err = validate_reply(r#"{"message": "hi", "extra": 1}"#, &pool, &contract, "Ping")
            .expect_err("unknown field");
        assert!(err.contains("unrecognized field"), "{err}");
        // Wrong type → refused.
        assert!(validate_reply(r#"{"message": 5}"#, &pool, &contract, "Ping").is_err());
        // Unknown METHOD → refused without touching the compiler.
        assert!(validate_reply("{}", &pool, &contract, "Nope").is_err());
        // Oversize → refused before parsing.
        let big = format!("\"{}\"", "x".repeat(RESPONSE_MAX + 1));
        assert!(validate_reply(&big, &pool, &contract, "Ping").is_err());
    }

    #[test]
    fn fences_are_stripped_but_content_kept() {
        assert_eq!(strip_fences("```json\n{\"a\":1}\n```"), "{\"a\":1}");
        assert_eq!(strip_fences("plain"), "plain");
        assert_eq!(strip_fences("```\nX\n```"), "X");
    }

    const PROTO: &str = r#"syntax = "proto3";
package demo.v1;
message PingRequest { string name = 1; }
message PingReply { string message = 1; }
service Greeter {
  rpc Ping(PingRequest) returns (PingReply);
}
"#;

    #[test]
    fn system_prompt_frames_the_virtual_service_and_contract() {
        let tool = test_tool();
        let contract = contract::validate(&tool, PROTO).expect("contract");
        let prompt = system_prompt(&tool, &contract);
        assert!(prompt.starts_with("RESERVED TOKEN"), "reserved token first");
        assert!(prompt.contains("VIRTUAL gRPC service"), "{prompt}");
        assert!(prompt.contains("demo.v1.Greeter"), "{prompt}");
        assert!(
            prompt.contains("Ping(demo.v1.PingRequest) -> demo.v1.PingReply"),
            "{prompt}"
        );
        assert!(prompt.contains("protojson"), "{prompt}");
    }
}
