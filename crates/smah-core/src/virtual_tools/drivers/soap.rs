//! SOAP virtual-tool driver (ticket #22) — interface-first, discovery-served.
//!
//! The tool file's `interface:` declares the WSDL (inline or path/`$VAR`).
//! The driver validates it under the strict XML policy (no DTDs/entities),
//! freezes it into the cache with sha pins, serves it byte-for-byte at the
//! discovery URL (`GET /?wsdl`), and answers `POST`ed SOAP envelopes by
//! asking the model to fabricate the operation response — then validates the
//! fabrication against the frozen contract (correct root element and
//! namespace, strict-XML clean, size-capped). One regeneration on drift; a
//! still-drifting fabrication becomes a SOAP fault, never a silent lie.
//!
//! Surface: loopback-only HTTP listener (B7). stderr carries the loud
//! disclosure line; `--no-serve` validates/freezes without binding. The
//! model never executes anything — it fabricates XML, schema-checked against
//! a contract it cannot alter (print-only holds; the only new boundary is
//! the loopback socket).
//!
//! Threading: the `&dyn ChatBackend` borrow is not `Send`, so fabrication
//! never crosses a thread — the tokio runtime runs on a spawned server
//! thread while `run`'s own thread is the fabricator, fed by a std mpsc
//! event channel (`Fabricate` jobs in, `Notice`/`Done` events out; replies
//! ride a tokio oneshot per request). Fabrications are serialized: the
//! service answers one request's model call at a time by design (v1).

#[path = "soap_contract.rs"]
mod contract;
#[path = "soap_xml.rs"]
mod xml;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use axum::extract::{Request, State};
use axum::http::{header, HeaderMap, Response, StatusCode};
use axum::routing::any;
use axum::Router;

use crate::errors::SmahError;
use crate::virtual_tools::audit::{Audit, AuditRecord};
use crate::virtual_tools::drivers::{ChatBackend, ChatMessage, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::{TURNS_MAX, WRITE_CONTENT_MAX};

use contract::{load_or_freeze, SoapContract};

/// Per-request fabricated-response cap (mirrors the family write cap).
const RESPONSE_MAX: usize = WRITE_CONTENT_MAX;
/// Inbound envelope cap.
const REQUEST_MAX: usize = 1024 * 1024;
/// Fabrication turns per request (initial attempt + one bounded regen).
const TURNS_PER_REQUEST: usize = 2;
/// Prior request/response pairs kept in the fabrication context.
const LEDGER_PAIRS: usize = 6;
/// Bytes of each ledger side kept after trimming.
const LEDGER_SNIPPET: usize = 2 * 1024;
/// Bytes of the request payload shown to the model.
const REQUEST_SNIPPET: usize = 16 * 1024;

#[derive(Debug)]
struct SoapArgs {
    port: u16,
    no_serve: bool,
    max_serve_seconds: Option<u64>,
}

fn parse_args(tool: &ToolDef, args: &[String], sink: &mut dyn Sink) -> Result<SoapArgs, SmahError> {
    let mut out = SoapArgs {
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
                    "smah(virtual): {} flag {other:?} is not a soap-driver flag \
                     (--port, --no-serve, --max-serve-seconds)",
                    tool.name
                ));
                return Err(usage(&format!(
                    "unknown soap-driver flag {other:?} (supported: --port, --no-serve, \
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
        message: format!("smah --emulate <soap-tool>: {detail}"),
    }
}

/// Run the soap driver: validate/freeze the contract, then (unless
/// `--no-serve`) serve the discovery document and emulated operations on a
/// loopback HTTP listener until ctrl-c / the serve deadline.
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
    sink.notice(&format!(
        "smah(virtual): {} contract frozen ({} operations; discovery: GET /?wsdl)",
        tool.name,
        contract.operations.len()
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

    serve(tool, backend, sink, model, contract, parsed, run_id)
}

/// Events flowing from the server thread (and its handlers) back to the
/// fabricator thread that owns the backend borrow.
enum Event {
    Fabricate(FabricateJob),
    Notice(String),
    Done(Result<(), SmahError>),
}

struct FabricateJob {
    operation: String,
    request_inner: String,
    reply: tokio::sync::oneshot::Sender<Result<String, String>>,
}

fn serve(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    contract: SoapContract,
    parsed: SoapArgs,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(|e| SmahError::Internal {
            detail: format!("soap driver: tokio runtime: {e}"),
        })?;

    let (event_tx, event_rx) = std::sync::mpsc::channel::<Event>();
    let turns = Arc::new(AtomicUsize::new(0));

    let state = Arc::new(ServeState {
        tool_name: tool.name.clone(),
        contract,
        event_tx: event_tx.clone(),
        turns: Arc::clone(&turns),
        audit: Audit::open(run_id),
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
                let app = Router::new()
                    .route("/", any(discover_or_invoke))
                    .fallback(unknown_endpoint)
                    .with_state(server_state);
                let listener = tokio::net::TcpListener::bind(("127.0.0.1", parsed.port))
                    .await
                    .map_err(|e| SmahError::Internal {
                        detail: format!("soap driver: loopback bind failed: {e}"),
                    })?;
                let bound = listener
                    .local_addr()
                    .map(|a| a.to_string())
                    .unwrap_or_else(|_| "127.0.0.1".to_string());
                if !bound.starts_with("127.0.0.1") {
                    return Err(SmahError::Internal {
                        detail: format!("soap driver: refusing non-loopback bind {bound}"),
                    });
                }
                let _ = notice_tx.send(Event::Notice(format!(
                    "serving http://{bound}/ (ctrl-c to stop) — VIRTUAL soap service: all \
                     responses are model fabrications, never real upstream data"
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
                        detail: format!("soap driver: serve failed: {e}"),
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
                    &job.operation,
                    &job.request_inner,
                    &ledger,
                ) {
                    Ok(fragment) => {
                        ledger.push((job.request_inner.clone(), fragment.clone()));
                        let excess = ledger.len().saturating_sub(LEDGER_PAIRS);
                        ledger.drain(..excess);
                        let _ = job.reply.send(Ok(fragment));
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
        "smah(virtual): {} soap service stopped ({turns_used} fabrication turns)",
        tool.name
    ));

    let served = serve_result.ok_or_else(|| SmahError::Internal {
        detail: "soap driver: server thread ended unexpectedly".to_string(),
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
    contract: SoapContract,
    event_tx: std::sync::mpsc::Sender<Event>,
    turns: Arc<AtomicUsize>,
    audit: Audit,
    run_id: String,
}

/// GET ?wsdl serves the frozen discovery document; POST invokes an operation.
async fn discover_or_invoke(
    State(state): State<Arc<ServeState>>,
    headers: HeaderMap,
    request: Request,
) -> Response<axum::body::Body> {
    let is_wsdl_query = request
        .uri()
        .query()
        .map(|q| q.to_ascii_lowercase().contains("wsdl"))
        .unwrap_or(false);
    if request.method() == axum::http::Method::GET && is_wsdl_query {
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "text/xml; charset=utf-8")
            .body(axum::body::Body::from(state.contract.wsdl.clone()))
            .expect("static response");
    }
    if request.method() != axum::http::Method::POST {
        return plain(StatusCode::METHOD_NOT_ALLOWED, "use GET /?wsdl or POST /\n");
    }

    let body = match BodyLimit::read(request, REQUEST_MAX).await {
        Ok(body) => body,
        Err(reason) => {
            audit_request(&state, "(oversized)", REQUEST_MAX, "denied", &reason);
            return soap_fault("Client", &reason);
        }
    };

    match invoke(&state, &headers, &body).await {
        Ok(response) => {
            audit_request(&state, &response.operation, body.len(), "allowed", "");
            xml_response(&response.body, &response.content_type)
        }
        Err(inv_fault) => {
            audit_request(
                &state,
                &inv_fault.operation,
                body.len(),
                "denied",
                &inv_fault.reason,
            );
            eprintln!(
                "smah(virtual): {} soap fault ({} {}): {}",
                state.tool_name, inv_fault.code, inv_fault.operation, inv_fault.reason
            );
            soap_fault(inv_fault.code, &inv_fault.reason)
        }
    }
}

async fn unknown_endpoint() -> Response<axum::body::Body> {
    plain(
        StatusCode::NOT_FOUND,
        "the endpoint is POST / (discovery: GET /?wsdl)\n",
    )
}

struct InvokeOk {
    operation: String,
    body: String,
    content_type: String,
}

struct InvokeFault {
    operation: String,
    code: &'static str,
    reason: String,
}

/// Parse + dispatch + fabricate, all errors surfaced as typed faults.
async fn invoke(
    state: &ServeState,
    headers: &HeaderMap,
    body: &str,
) -> Result<InvokeOk, InvokeFault> {
    let refuse = |code: &'static str, operation: &str, reason: String| InvokeFault {
        operation: operation.to_string(),
        code,
        reason,
    };

    let doc = xml::strict_scan(body).map_err(|e| {
        refuse(
            "Client",
            "(unparsable)",
            format!("request refused by strict XML policy: {e}"),
        )
    })?;
    if doc.root_local != "Envelope"
        || doc
            .root_ns
            .as_deref()
            .map(|ns| !xml::ENVELOPE_NS.contains(&ns))
            .unwrap_or(true)
    {
        return Err(refuse(
            "Client",
            "(no envelope)",
            "request root must be a SOAP Envelope in a SOAP envelope namespace".to_string(),
        ));
    }

    // First element child of the Body is the request payload.
    let body_index = doc
        .elements
        .iter()
        .position(|el| el.local == "Body" && el.ns == doc.root_ns)
        .ok_or_else(|| {
            refuse(
                "Client",
                "(no body)",
                "Envelope carries no Body".to_string(),
            )
        })?;
    let payload = doc.elements.get(body_index + 1).ok_or_else(|| {
        refuse(
            "Client",
            "(no operation)",
            "Body carries no operation element".to_string(),
        )
    })?;

    let operation_name = soap_action(headers).unwrap_or_else(|| payload.local.clone());
    let operation = state
        .contract
        .operation(&operation_name)
        .ok_or_else(|| {
            refuse(
                "Client",
                &operation_name,
                format!(
                    "unknown operation {operation_name:?}; declared: {}",
                    state
                        .contract
                        .operations
                        .iter()
                        .map(|op| op.name.clone())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            )
        })?
        .clone();

    let request_inner = inner_payload(body, &payload.local)
        .unwrap_or_else(|| format!("<{}>{}</{}>", payload.local, payload.local, payload.local));
    let request_inner = truncate_chars(&request_inner, REQUEST_SNIPPET);

    // Hand the fabrication to the thread that owns the backend borrow.
    let (reply_tx, reply_rx) = tokio::sync::oneshot::channel();
    state
        .event_tx
        .send(Event::Fabricate(FabricateJob {
            operation: operation.name.clone(),
            request_inner: request_inner.clone(),
            reply: reply_tx,
        }))
        .map_err(|_| {
            refuse(
                "Server",
                &operation.name,
                "fabrication worker is unavailable".to_string(),
            )
        })?;
    let fabricated = reply_rx.await.map_err(|_| {
        refuse(
            "Server",
            &operation.name,
            "fabrication worker dropped the reply".to_string(),
        )
    })?;

    let fragment = match fabricated {
        Ok(fragment) => fragment,
        Err(reason) => {
            return Err(refuse(
                "Server",
                &operation.name,
                format!("fabrication failed: {reason}"),
            ))
        }
    };

    // Validate the fabricated fragment against the frozen contract, then wrap.
    let (envelope_ns, content_type) = envelope_for(&doc.root_ns);
    match validate_fragment(
        &fragment,
        operation.response_element.as_ref(),
        &state.contract,
    ) {
        Ok(()) => Ok(InvokeOk {
            operation: operation.name,
            body: wrap_envelope(&fragment, envelope_ns),
            content_type: content_type.to_string(),
        }),
        Err(reason) => Err(refuse(
            "Server",
            &operation.name,
            format!("fabricated response rejected by the frozen contract: {reason}"),
        )),
    }
}

fn soap_action(headers: &HeaderMap) -> Option<String> {
    headers
        .get("SOAPAction")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim_matches('"').trim().to_string())
        .filter(|v| !v.is_empty())
        .map(|v| {
            // SOAPAction often carries a URI; keep its last path segment.
            v.rsplit('/').next().unwrap_or(&v).to_string()
        })
}

fn envelope_for(request_ns: &Option<String>) -> (&'static str, &'static str) {
    match request_ns.as_deref() {
        Some("http://www.w3.org/2003/05/soap-envelope") => (
            "http://www.w3.org/2003/05/soap-envelope",
            "application/soap+xml; charset=utf-8",
        ),
        _ => (
            "http://schemas.xmlsoap.org/soap/envelope/",
            "text/xml; charset=utf-8",
        ),
    }
}

/// Extract the raw payload element (scan from its start tag to the matching
/// close) so the model sees real XML rather than a re-serialization. Nested
/// same-name elements match greedily from the inside — acceptable for
/// fabrication context, since the response is validated separately.
fn inner_payload(body: &str, payload_local: &str) -> Option<String> {
    let open = format!("<{payload_local}");
    let start = body.find(&open)?;
    let bytes = body.as_bytes();
    let mut i = start;
    let mut depth = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'<' => depth += 1,
            b'>' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    let head = &body[start..=i];
                    if head.ends_with("/>") {
                        return Some(head.to_string());
                    }
                    let close = format!("</{payload_local}>");
                    let end_rel = body[i + 1..].find(&close)?;
                    let end = i + 1 + end_rel + close.len();
                    return Some(body[start..end].to_string());
                }
            }
            _ => {}
        }
        i += 1;
    }
    None
}

fn validate_fragment(
    fragment: &str,
    expected_root: Option<&String>,
    contract: &SoapContract,
) -> Result<(), String> {
    if fragment.len() > RESPONSE_MAX {
        return Err(format!("fragment exceeds the {RESPONSE_MAX}-byte cap"));
    }
    let doc = xml::strict_scan(fragment).map_err(|e| format!("not strict-clean XML: {e}"))?;
    if let Some(expected) = expected_root {
        if doc.root_local != *expected {
            return Err(format!(
                "expected root element {expected:?}, got {:?}",
                doc.root_local
            ));
        }
    }
    // Namespace: the fabricated root must sit in the contract's target
    // namespace when it declares one.
    if let Some(ns) = &doc.root_ns {
        if *ns != contract.target_namespace {
            return Err(format!(
                "root namespace {ns:?} is not the contract targetNamespace {:?}",
                contract.target_namespace
            ));
        }
    }
    Ok(())
}

fn wrap_envelope(fragment: &str, envelope_ns: &str) -> String {
    format!(
        "<soap:Envelope xmlns:soap=\"{envelope_ns}\"><soap:Body>{fragment}</soap:Body></soap:Envelope>"
    )
}

/// SOAP 1.1 fault envelope (also acceptable to most 1.2 clients in v1).
fn soap_fault(code: &'static str, reason: &str) -> Response<axum::body::Body> {
    let envelope_ns = "http://schemas.xmlsoap.org/soap/envelope/";
    let body = format!(
        "<soap:Envelope xmlns:soap=\"{envelope_ns}\"><soap:Body><soap:Fault>\
         <faultcode>soap:{code}</faultcode><faultstring>{}</faultstring>\
         </soap:Fault></soap:Body></soap:Envelope>",
        escaped(reason)
    );
    xml_response(&body, "text/xml; charset=utf-8")
}

fn xml_response(body: &str, content_type: &str) -> Response<axum::body::Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, content_type)
        .body(axum::body::Body::from(body.to_string()))
        .expect("static response")
}

fn plain(status: StatusCode, text: &str) -> Response<axum::body::Body> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .body(axum::body::Body::from(text.to_string()))
        .expect("static response")
}

/// Read the request body with a hard byte cap; oversize is an error, not a
/// silent truncation.
struct BodyLimit;

impl BodyLimit {
    async fn read(request: Request, cap: usize) -> Result<String, String> {
        let bytes = axum::body::to_bytes(request.into_body(), cap)
            .await
            .map_err(|e| format!("body exceeds the {cap}-byte cap ({e})"))?;
        String::from_utf8(bytes.to_vec()).map_err(|_| "body is not valid UTF-8".to_string())
    }
}

fn escaped(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(c),
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

/// Metadata-only audit for one served request (name/size/decision — never
/// contents).
fn audit_request(
    state: &ServeState,
    operation: &str,
    bytes: usize,
    decision: &'static str,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: state.run_id.clone(),
        tool: state.tool_name.clone(),
        op: "soap.request".to_string(),
        path: operation.to_string(),
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
fn fabricate(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    model: &str,
    contract: &SoapContract,
    operation: &str,
    request_inner: &str,
    ledger: &[(String, String)],
) -> Result<String, SmahError> {
    let system = system_prompt(tool, contract);
    let mut user = format!(
        "SOAP request received.\nOperation: {operation}{}\nRequest payload:\n{request_inner}\n",
        contract
            .operation(operation)
            .and_then(|op| op.response_element.as_ref())
            .map(|el| format!(" (expected response element: <{el}>)"))
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
        "\nProduce ONLY the XML element for this operation's response, consistent with \
         the contract and the prior exchanges. No envelope, no code fences, no commentary.\n",
    );
    let messages = vec![ChatMessage::user(user)];

    let mut feedback: Option<String> = None;
    for _attempt in 0..TURNS_PER_REQUEST {
        let mut full_messages = messages.clone();
        if let Some(reason) = &feedback {
            full_messages.push(ChatMessage::assistant(format!(
                "<{operation}Response>…</{operation}Response>"
            )));
            full_messages.push(ChatMessage::user(format!(
                "Your previous reply was refused by the frozen contract: {reason}\n\
                 Reply again with ONLY the corrected response element XML."
            )));
        }
        let reply = backend.stream_turn(model, &system, &full_messages, &mut |_| {})?;
        let fragment = strip_fences(reply.trim());
        match validate_fragment(&fragment, expected_root(contract, operation), contract) {
            Ok(()) => return Ok(fragment),
            Err(reason) => feedback = Some(reason),
        }
    }
    Err(SmahError::DirectiveLoopOverflow {
        tool: tool.name.clone(),
        turns: TURNS_PER_REQUEST.min(TURNS_MAX),
    })
}

fn expected_root<'a>(contract: &'a SoapContract, operation: &str) -> Option<&'a String> {
    contract
        .operation(operation)
        .and_then(|op| op.response_element.as_ref())
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

fn system_prompt(tool: &ToolDef, contract: &SoapContract) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "RESERVED TOKEN: the line prefix \"@@smah \" is reserved by the host program. \
Never emit it.\n\n",
    );
    prompt.push_str(
        "You are the implementation behind a VIRTUAL SOAP service used for contract \
testing and local development. The service contract below is FROZEN — it defines \
exactly which operations exist and what their responses look like. Fabricate \
responses that a real service implementing this contract could truthfully return. \
Never claim to have contacted a real upstream.\n\n",
    );
    prompt.push_str(&format!(
        "CONTRACT (frozen): targetNamespace {}; operations: {}\n",
        contract.target_namespace,
        contract
            .operations
            .iter()
            .map(|op| match &op.response_element {
                Some(el) => format!("{} -> <{}>", op.name, el),
                None => op.name.clone(),
            })
            .collect::<Vec<_>>()
            .join(", ")
    ));
    prompt.push_str(
        "\nOUTPUT RULES:\n- Reply with ONLY the response XML element for the \
requested operation (the inner payload, no SOAP envelope, no code fences, no \
commentary).\n- The reply is validated against the frozen contract; a reply with \
the wrong root element or namespace is refused and you get one bounded retry.\n",
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
            name: "soaptest".to_string(),
            description: "d".to_string(),
            model: None,
            driver: "soap".to_string(),
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            interface: None,
            body: String::new(),
            path: std::path::PathBuf::from("soaptest.md"),
        }
    }

    #[test]
    fn args_parse_flags_and_reject_unknowns() {
        let mut sink = Sink;
        let args: Vec<String> = ["--port", "8080", "--no-serve"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let tool = test_tool();
        let parsed = parse_args(&tool, &args, &mut sink).expect("parse");
        assert_eq!(parsed.port, 8080);
        assert!(parsed.no_serve);

        let inline: Vec<String> = ["--port=9090", "--max-serve-seconds=5"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let parsed = parse_args(&tool, &inline, &mut sink).expect("inline parse");
        assert_eq!(parsed.port, 9090);
        assert_eq!(parsed.max_serve_seconds, Some(5));

        let bad: Vec<String> = ["--prot", "8080"].iter().map(|s| s.to_string()).collect();
        let err = parse_args(&tool, &bad, &mut sink).expect_err("unknown flag");
        assert!(matches!(err, SmahError::Usage { .. }));
        assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Usage);
    }

    #[test]
    fn envelope_wrap_and_fault_shape() {
        let wrapped = wrap_envelope(
            "<x:pingResponse/>",
            "http://schemas.xmlsoap.org/soap/envelope/",
        );
        assert!(wrapped.starts_with("<soap:Envelope"));
        assert!(wrapped.contains("<soap:Body><x:pingResponse/></soap:Body>"));
        let body = soap_fault("Client", "bad <stuff>");
        assert_eq!(body.status(), StatusCode::OK);
    }

    #[test]
    fn fragment_validation_enforces_root_and_namespace() {
        let contract = SoapContract {
            wsdl: String::new(),
            target_namespace: "urn:x".to_string(),
            operations: Vec::new(),
        };
        assert!(validate_fragment(
            "<pingResponse xmlns=\"urn:x\"/>",
            Some(&"pingResponse".to_string()),
            &contract
        )
        .is_ok());
        assert!(validate_fragment(
            "<wrong xmlns=\"urn:x\"/>",
            Some(&"pingResponse".to_string()),
            &contract
        )
        .is_err());
        assert!(validate_fragment("<pingResponse xmlns=\"urn:other\"/>", None, &contract).is_err());
        assert!(validate_fragment("<a>&custom;</a>", None, &contract).is_err());
    }

    #[test]
    fn fences_are_stripped_and_payload_extracted() {
        assert_eq!(strip_fences("```xml\n<a/>\n```"), "<a/>");
        assert_eq!(strip_fences("<a/>"), "<a/>");
        let inner = inner_payload(
            "<Env><Body><ping xmlns=\"urn:x\">hi</ping></Body></Env>",
            "ping",
        )
        .expect("payload");
        assert_eq!(inner, "<ping xmlns=\"urn:x\">hi</ping>");
        assert_eq!(escaped("<a&b>"), "&lt;a&amp;b&gt;");
    }

    #[test]
    fn envelope_version_matches_the_request() {
        let (ns, ct) = envelope_for(&Some("http://www.w3.org/2003/05/soap-envelope".to_string()));
        assert_eq!(ns, "http://www.w3.org/2003/05/soap-envelope");
        assert!(ct.starts_with("application/soap+xml"));
        let (ns, ct) = envelope_for(&None);
        assert_eq!(ns, "http://schemas.xmlsoap.org/soap/envelope/");
        assert!(ct.starts_with("text/xml"));
    }
}
