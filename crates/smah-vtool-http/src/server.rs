//! The loopback HTTP host for the api driver (#20): serves the frozen
//! OpenAPI contract byte-for-byte at `/openapi.json` and routes everything
//! else to a caller-supplied fabrication closure (the LLM seam stays in
//! smah-core; this crate is plumbing only).
//!
//! Binds `127.0.0.1` exclusively (T-020); no TLS, no non-loopback bind, no
//! process execution.

use std::net::SocketAddr;
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, Method, StatusCode, Uri};
use axum::response::IntoResponse;
use axum::routing::get;
use axum::Router;
use tokio::net::TcpListener;
use tokio::sync::Notify;

/// Re-exported so driver crates (which stay tokio-free) can arm a test
/// shutdown signal without a direct tokio dependency.
pub use tokio::sync::Notify as ShutdownSignal;

/// One emulated request handed to the fabrication closure.
#[derive(Debug, Clone)]
pub struct FabRequest {
    /// HTTP method (`GET`, `POST`, ...).
    pub method: String,
    /// Path + query string as received (`/widgets?page=2`).
    pub path_and_query: String,
    /// Raw request body, when present (the host caps body size first).
    pub body: Option<String>,
}

/// The closure's verdict for one emulated request.
#[derive(Debug, Clone)]
pub struct FabOutcome {
    /// HTTP status to answer with (the host trusts the closure's range
    /// check; out-of-range falls back to 500).
    pub status: u16,
    /// Response body (JSON text; served with `application/json`).
    pub body: String,
}

/// The fabrication seam: blocking, callable from multiple threads.
pub type Fabricate = Arc<dyn Fn(FabRequest) -> Result<FabOutcome, String> + Send + Sync>;

/// Request-body cap: oversized bodies are refused with 413 before any model
/// call (bounded provider spend per request).
pub const BODY_MAX: usize = 256 * 1024;

struct ServerState {
    spec: Arc<str>,
    fabricate: Fabricate,
}

/// Serve until ctrl-c (or `shutdown` fires, for tests).
///
/// `on_bound` runs once with the actual (ephemeral-capable) port after the
/// listener is up — the driver uses it for its stderr line + audit record.
/// Returns when the server shuts down gracefully.
pub async fn serve(
    port: u16,
    spec: Arc<str>,
    fabricate: Fabricate,
    shutdown: Option<Arc<Notify>>,
    on_bound: Option<Box<dyn FnOnce(u16) + Send>>,
) -> std::io::Result<()> {
    let state = Arc::new(ServerState { spec, fabricate });
    let app = Router::new()
        .route("/openapi.json", get(serve_spec))
        .fallback(fabricate_route)
        .layer(DefaultBodyLimit::max(BODY_MAX))
        .with_state(state);
    let listener = TcpListener::bind(("127.0.0.1", port)).await?;
    let addr: SocketAddr = listener.local_addr()?;
    if let Some(cb) = on_bound {
        cb(addr.port());
    }
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_future(shutdown))
        .await
}

/// Blocking flavor of [`serve`]: builds its own runtime and returns after
/// graceful shutdown (ctrl-c or the test signal). Driver crates call this —
/// they stay tokio-free.
pub fn serve_blocking(
    port: u16,
    spec: Arc<str>,
    fabricate: Fabricate,
    shutdown: Option<Arc<Notify>>,
    on_bound: Option<Box<dyn FnOnce(u16) + Send>>,
) -> std::io::Result<()> {
    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(serve(port, spec, fabricate, shutdown, on_bound))
}

async fn shutdown_future(shutdown: Option<Arc<Notify>>) {
    match shutdown {
        Some(notify) => {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = notify.notified() => {}
            }
        }
        None => {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

async fn serve_spec(State(state): State<Arc<ServerState>>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "application/json")],
        state.spec.to_string(),
    )
}

/// Everything that is not `/openapi.json` is an emulated endpoint: hand the
/// request to the fabrication closure.
async fn fabricate_route(
    State(state): State<Arc<ServerState>>,
    method: Method,
    uri: Uri,
    body: Bytes,
) -> impl IntoResponse {
    let body = if body.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&body).to_string())
    };
    let request = FabRequest {
        method: method.as_str().to_string(),
        path_and_query: uri
            .path_and_query()
            .map(|pq| pq.to_string())
            .unwrap_or_else(|| uri.path().to_string()),
        body,
    };

    // The closure blocks on the provider; keep it off the runtime workers.
    let fabricate = state.fabricate.clone();
    let result = tokio::task::spawn_blocking(move || fabricate(request))
        .await
        .unwrap_or_else(|join| Err(format!("fabrication task failed: {join}")));

    match result {
        Ok(outcome) => {
            let status =
                StatusCode::from_u16(outcome.status).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
            (
                status,
                [(header::CONTENT_TYPE, "application/json")],
                outcome.body,
            )
        }
        Err(reason) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            [(header::CONTENT_TYPE, "application/json")],
            format!(
                "{{\"error\":\"fabrication failed\",\"detail\":{}}}",
                json_escape(&reason)
            ),
        ),
    }
}

fn json_escape(text: &str) -> String {
    serde_json::to_string(text).unwrap_or_else(|_| "\"fabrication failed\"".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpStream;
    use std::sync::Mutex;
    use std::time::Duration;

    const SPEC: &str = r#"{"openapi":"3.1.0","paths":{"/widgets":{"get":{}}}}"#;

    fn http_get(port: u16, path: &str) -> String {
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        write!(
            stream,
            "GET {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
        )
        .expect("write");
        let mut buf = String::new();
        let _ = stream.read_to_string(&mut buf);
        buf
    }

    /// Spawn the server on an ephemeral port; returns (port, shutdown).
    fn spawn_server(fabricate: Fabricate) -> (u16, Arc<Notify>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let notify = Arc::new(Notify::new());
        let shutdown = notify.clone();
        std::thread::spawn(move || {
            let _ = serve_blocking(
                0,
                Arc::from(SPEC),
                fabricate,
                Some(shutdown),
                Some(Box::new(move |port| {
                    let _ = tx.send(port);
                })),
            );
        });
        let port = rx
            .recv_timeout(Duration::from_secs(5))
            .expect("server bound in time");
        (port, notify)
    }

    #[test]
    fn serves_contract_verbatim_and_routes_fallback() {
        let calls: Arc<Mutex<Vec<FabRequest>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_for_fb = calls.clone();
        let fabricate: Fabricate = Arc::new(move |req: FabRequest| {
            calls_for_fb.lock().expect("lock").push(req);
            Ok(FabOutcome {
                status: 201,
                body: "{\"ok\":true,\"echo\":[1,2]}".to_string(),
            })
        });

        let (port, shutdown) = spawn_server(fabricate);

        // Contract is byte-for-byte.
        let raw = http_get(port, "/openapi.json");
        let body = raw.split("\r\n\r\n").nth(1).expect("body");
        assert_eq!(body, SPEC);

        // Fallback reaches the fabrication closure with the request shape.
        let raw = http_get(port, "/widgets?page=2");
        assert!(raw.starts_with("HTTP/1.1 201"), "{raw}");
        assert!(raw.contains("{\"ok\":true,\"echo\":[1,2]}"), "{raw}");
        let seen = calls.lock().expect("lock");
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].method, "GET");
        assert_eq!(seen[0].path_and_query, "/widgets?page=2");
        assert!(seen[0].body.is_none());

        shutdown.notify_one();
    }

    #[test]
    fn fabrication_error_is_500_json() {
        let fabricate: Fabricate = Arc::new(|_req| Err("provider 502".to_string()));
        let (port, shutdown) = spawn_server(fabricate);
        let raw = http_get(port, "/anything");
        assert!(raw.starts_with("HTTP/1.1 500"), "{raw}");
        assert!(raw.contains("fabrication failed"), "{raw}");
        shutdown.notify_one();
    }

    #[test]
    fn oversized_body_is_refused_413() {
        use std::sync::atomic::{AtomicU16, Ordering};
        // A body over BODY_MAX must be refused before the closure runs —
        // assert the closure is never called via a panicking... (panics in a
        // spawn_blocking thread poison, so instead count calls).
        let count = Arc::new(AtomicU16::new(0));
        let count_for_fb = count.clone();
        let fabricate: Fabricate = Arc::new(move |_req| {
            count_for_fb.fetch_add(1, Ordering::SeqCst);
            Ok(FabOutcome {
                status: 200,
                body: "{}".to_string(),
            })
        });
        let (port, shutdown) = spawn_server(fabricate);

        let big = "x".repeat(BODY_MAX + 1);
        let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .expect("timeout");
        write!(
            stream,
            "POST /widgets HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{big}",
            big.len()
        )
        .expect("write");
        let mut buf = String::new();
        let _ = stream.read_to_string(&mut buf);
        assert!(buf.starts_with("HTTP/1.1 413"), "expected 413, got: {buf}");
        assert_eq!(count.load(Ordering::SeqCst), 0, "closure must not run");
        shutdown.notify_one();
    }
}
