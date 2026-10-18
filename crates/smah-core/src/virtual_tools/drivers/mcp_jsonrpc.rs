//! JSON-RPC 2.0 framing for the MCP virtual tool (ticket #23).
//!
//! One request per line (newline-delimited). MCP does not use JSON-RPC
//! batch arrays — an array payload is answered `Invalid Request` rather
//! than silently batched. Notifications (no `id`) are consumed silently;
//! requests carry an `id` and always get exactly one response line.
//!
//! The generic batch/TCP serve helpers take any [`RpcHandler`], so the vfs
//! driver reuses the same framing for its file tools (ticket #25).

use std::io::{BufRead, Write};
use std::net::TcpListener;

use serde_json::{Map, Value};

use crate::virtual_tools::drivers::Sink;

// JSON-RPC 2.0 reserved error codes.
pub(crate) const PARSE_ERROR: i64 = -32700;
pub(crate) const INVALID_REQUEST: i64 = -32600;
pub(crate) const METHOD_NOT_FOUND: i64 = -32601;
pub(crate) const INVALID_PARAMS: i64 = -32602;
pub(crate) const INTERNAL: i64 = -32603;

/// A method-level refusal (mapped 1:1 onto a JSON-RPC error response).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RpcError {
    pub code: i64,
    pub message: String,
}

impl RpcError {
    pub(crate) fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError {
            code,
            message: message.into(),
        }
    }
}

/// One parsed request. `id: None` = notification (no response owed).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Request {
    pub method: String,
    pub params: Value,
    pub id: Option<Value>,
}

/// A line that parsed: a call (response owed) or a notification.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    Call(Request),
    Notification(Request),
}

/// Parse one newline-delimited request. `Err((code, message))` becomes an
/// error response with a `null` id (the spec's shape for pre-id failures).
pub(crate) fn parse_line(line: &str) -> Result<Incoming, (i64, String)> {
    let value: Value =
        serde_json::from_str(line).map_err(|err| (PARSE_ERROR, format!("invalid JSON: {err}")))?;
    let Some(obj) = value.as_object() else {
        return Err((
            INVALID_REQUEST,
            "each request must be a single JSON object (MCP does not batch)".to_string(),
        ));
    };
    match obj.get("jsonrpc") {
        Some(Value::String(v)) if v == "2.0" => {}
        _ => {
            return Err((
                INVALID_REQUEST,
                "missing or wrong jsonrpc field (expected \"2.0\")".to_string(),
            ))
        }
    }
    let method = match obj.get("method") {
        Some(Value::String(m)) => m.clone(),
        _ => return Err((INVALID_REQUEST, "missing string 'method'".to_string())),
    };
    let params = match obj.get("params") {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(p @ Value::Object(_)) | Some(p @ Value::Array(_)) => p.clone(),
        Some(_) => {
            return Err((
                INVALID_REQUEST,
                "'params' must be an object or array".to_string(),
            ))
        }
    };
    let id = match obj.get("id") {
        None | Some(Value::Null) => None,
        Some(id) => Some(id.clone()),
    };
    let request = Request { method, params, id };
    Ok(match request.id {
        None => Incoming::Notification(request),
        Some(_) => Incoming::Call(request),
    })
}

fn base_response(id: &Value) -> Map<String, Value> {
    let mut out = Map::new();
    out.insert("jsonrpc".to_string(), Value::String("2.0".to_string()));
    out.insert("id".to_string(), id.clone());
    out
}

/// Serialize a success response line.
pub(crate) fn response(id: &Value, result: Value) -> String {
    let mut out = base_response(id);
    out.insert("result".to_string(), result);
    Value::Object(out).to_string()
}

/// Serialize an error response line (spec shape: error.code/message/data).
pub(crate) fn error_response(id: &Value, code: i64, message: &str) -> String {
    let mut err = Map::new();
    err.insert("code".to_string(), Value::from(code));
    err.insert("message".to_string(), Value::from(message));
    let mut out = base_response(id);
    out.insert("error".to_string(), Value::Object(err));
    Value::Object(out).to_string()
}

/// The method dispatcher a serve loop drives.
pub(crate) trait RpcHandler {
    fn call(&mut self, request: &Request) -> Result<Value, RpcError>;
}

/// Answer every newline-delimited request in the invocation's stdin (batch
/// mode — the prelude reads stdin to EOF, so all requests are already
/// present). Responses go to stdout in order. Returns the response count.
pub(crate) fn serve_stdio_batch(
    stdin_text: &str,
    stdin_partial: bool,
    sink: &mut dyn Sink,
    handler: &mut dyn RpcHandler,
) -> usize {
    if stdin_partial {
        sink.notice("smah: input truncated at the byte cap; trailing requests may be malformed");
    }
    let mut answered = 0usize;
    for line in stdin_text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let outgoing = match parse_line(line) {
            Err((code, message)) => error_response(&Value::Null, code, &message),
            Ok(Incoming::Notification(_)) => continue,
            Ok(Incoming::Call(request)) => match handler.call(&request) {
                Ok(result) => response(request.id.as_ref().unwrap_or(&Value::Null), result),
                Err(err) => error_response(
                    request.id.as_ref().unwrap_or(&Value::Null),
                    err.code,
                    &err.message,
                ),
            },
        };
        sink.content(&format!("{outgoing}\n"));
        answered += 1;
    }
    answered
}

/// Serve newline-delimited JSON-RPC on an already-bound loopback TCP
/// listener, sequentially (one connection at a time). Blocks until the
/// process ends; each client disconnect just returns to the accept loop.
pub(crate) fn serve_tcp<H: RpcHandler>(
    listener: TcpListener,
    mut handler: H,
    notice: &mut dyn FnMut(&str),
) -> std::io::Result<()> {
    for incoming in listener.incoming() {
        let mut stream = match incoming {
            Ok(stream) => stream,
            Err(err) => {
                notice(&format!("smah: accept failed: {err}"));
                continue;
            }
        };
        let mut reader = std::io::BufReader::new(match stream.try_clone() {
            Ok(reader) => reader,
            Err(err) => {
                notice(&format!("smah: stream setup failed: {err}"));
                continue;
            }
        });
        let mut line = String::new();
        loop {
            line.clear();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break, // client went away
                Ok(_) => {}
            }
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let outgoing = match parse_line(trimmed) {
                Err((code, message)) => error_response(&Value::Null, code, &message),
                Ok(Incoming::Notification(_)) => continue,
                Ok(Incoming::Call(request)) => match handler.call(&request) {
                    Ok(result) => response(request.id.as_ref().unwrap_or(&Value::Null), result),
                    Err(err) => error_response(
                        request.id.as_ref().unwrap_or(&Value::Null),
                        err.code,
                        &err.message,
                    ),
                },
            };
            if stream
                .write_all(outgoing.as_bytes())
                .and_then(|_| stream.write_all(b"\n"))
                .and_then(|_| stream.flush())
                .is_err()
            {
                break;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn call(method: &str, id: i64) -> String {
        json!({"jsonrpc":"2.0","id":id,"method":method}).to_string()
    }

    #[test]
    fn parse_line_accepts_calls_and_notifications() {
        let incoming = parse_line(&call("ping", 7)).expect("call");
        match incoming {
            Incoming::Call(request) => {
                assert_eq!(request.method, "ping");
                assert_eq!(request.id, Some(json!(7)));
                assert_eq!(request.params, json!({}));
            }
            _ => panic!("expected a call"),
        }
        let incoming = parse_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .expect("notification");
        assert!(matches!(incoming, Incoming::Notification(_)));
    }

    #[test]
    fn parse_line_rejects_bad_frames_with_wire_codes() {
        assert_eq!(parse_line("not json").unwrap_err().0, PARSE_ERROR);
        assert_eq!(parse_line("[1,2]").unwrap_err().0, INVALID_REQUEST);
        assert_eq!(
            parse_line(r#"{"id":1,"method":"x"}"#).unwrap_err().0,
            INVALID_REQUEST
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"1.0","id":1,"method":"x"}"#)
                .unwrap_err()
                .0,
            INVALID_REQUEST
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","id":1}"#).unwrap_err().0,
            INVALID_REQUEST
        );
        assert_eq!(
            parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"x","params":3}"#)
                .unwrap_err()
                .0,
            INVALID_REQUEST
        );
    }

    #[test]
    fn responses_carry_result_and_errors_carry_code_message() {
        let ok = response(&json!(3), json!({"pong":true}));
        let v: Value = serde_json::from_str(&ok).expect("json");
        assert_eq!(v["id"], json!(3));
        assert_eq!(v["result"]["pong"], json!(true));
        let err = error_response(&Value::Null, METHOD_NOT_FOUND, "nope");
        let v: Value = serde_json::from_str(&err).expect("json");
        assert_eq!(v["error"]["code"], json!(METHOD_NOT_FOUND));
        assert_eq!(v["error"]["message"], json!("nope"));
        assert!(v.get("result").is_none());
    }

    /// A scripted handler for serve-loop tests.
    struct Echo;
    impl RpcHandler for Echo {
        fn call(&mut self, request: &Request) -> Result<Value, RpcError> {
            if request.method == "boom" {
                return Err(RpcError::new(INTERNAL, "boom"));
            }
            Ok(json!({"echo": request.method}))
        }
    }

    #[test]
    fn batch_mode_answers_in_order_and_survives_bad_lines() {
        let stdin_text = format!(
            "{}\n\ngarbage\n{}\n{}\n",
            call("a", 1),
            call("b", 2),
            call("boom", 3)
        );
        struct Collect(Vec<String>);
        impl Sink for Collect {
            fn content(&mut self, piece: &str) {
                self.0.push(piece.to_string());
            }
            fn notice(&mut self, _line: &str) {}
        }
        let mut sink = Collect(Vec::new());
        let count = serve_stdio_batch(&stdin_text, false, &mut sink, &mut Echo);
        // 3 calls + 1 parse-error answer for the garbage line.
        assert_eq!(count, 4);
        let lines: Vec<Value> = sink
            .0
            .iter()
            .map(|l| serde_json::from_str(l.trim()).expect("json line"))
            .collect();
        assert_eq!(lines[0]["id"], json!(1));
        assert_eq!(lines[0]["result"]["echo"], json!("a"));
        assert_eq!(lines[1]["error"]["code"], json!(PARSE_ERROR));
        assert!(lines[1]["id"].is_null());
        assert_eq!(lines[2]["id"], json!(2));
        assert_eq!(lines[2]["result"]["echo"], json!("b"));
        assert_eq!(lines[3]["error"]["code"], json!(INTERNAL));
        assert_eq!(lines[3]["id"], json!(3));
    }

    #[test]
    fn batch_mode_flags_truncated_input_once() {
        struct Notices(Vec<String>, usize);
        impl Sink for Notices {
            fn content(&mut self, _piece: &str) {
                self.1 += 1;
            }
            fn notice(&mut self, line: &str) {
                self.0.push(line.to_string());
            }
        }
        let mut sink = Notices(Vec::new(), 0);
        serve_stdio_batch(&call("a", 1), true, &mut sink, &mut Echo);
        assert_eq!(sink.0.len(), 1);
        assert!(sink.0[0].contains("truncated"));
    }

    #[test]
    fn tcp_serve_round_trips_one_connection() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind");
        let port = listener.local_addr().expect("addr").port();
        // serve_tcp blocks forever by design; the server thread is leaked
        // after the round trip (the test harness exits the process).
        fn noop_notice(_line: &str) {}
        let server = std::thread::spawn(move || {
            let _ = serve_tcp(listener, Echo, &mut noop_notice);
        });
        use std::io::{BufRead, BufReader, Write};
        let mut client = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
        client
            .write_all(format!("{}\n{}\n", call("a", 1), call("boom", 2)).as_bytes())
            .expect("send");
        client.flush().expect("flush");
        let mut reader = BufReader::new(client);
        let mut first = String::new();
        let mut second = String::new();
        reader.read_line(&mut first).expect("read1");
        reader.read_line(&mut second).expect("read2");
        let v: Value = serde_json::from_str(first.trim()).expect("json1");
        assert_eq!(v["result"]["echo"], json!("a"));
        let v: Value = serde_json::from_str(second.trim()).expect("json2");
        assert_eq!(v["error"]["code"], json!(INTERNAL));
        drop(reader);
        let _ = server; // intentionally not joined: serve_tcp never returns
    }
}
