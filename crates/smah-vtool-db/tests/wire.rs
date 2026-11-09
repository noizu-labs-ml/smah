//! Black-box wire-level tests for the db driver (ticket #24).
//!
//! `db::run` is started on a thread in its default unix-socket mode; a
//! hand-rolled PG client then drives the wire: SSL refusal, startup +
//! trust auth + branding, SELECT round-trip (`T`/`D`/`C`/`Z`), read-only
//! refusal (42501), extended-protocol refusal (0A000) with recovery on
//! Sync, and clean termination. All env-pointed state is hermetic
//! (`XDG_CACHE_HOME` + `XDG_STATE_HOME` under tempdirs, env-lock guarded).

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard};

use smah_core::errors::SmahError;
use smah_core::virtual_tools::audit::Audit;
use smah_core::virtual_tools::drivers::db;
use smah_core::virtual_tools::drivers::{ChatBackend, ChatMessage, FsMode, Sink};
use smah_core::virtual_tools::loader::ToolDef;

/// Process-wide lock: tests repoint process-global env vars.
static ENV: Mutex<()> = Mutex::new(());

/// Backend whose replies are consumed in order.
#[derive(Default)]
struct Scripted {
    replies: Mutex<Vec<String>>,
}

impl Scripted {
    fn new(replies: Vec<String>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies),
        })
    }
}

impl ChatBackend for Scripted {
    fn stream_turn(
        &self,
        _model: &str,
        _system: &str,
        _messages: &[ChatMessage],
        _on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
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

/// Thread-shareable sink: notices land in an Arc mutex for assertions.
#[derive(Clone, Default)]
struct Shared {
    notices: Arc<Mutex<Vec<String>>>,
}

impl Sink for Shared {
    fn content(&mut self, _piece: &str) {}
    fn notice(&mut self, line: &str) {
        self.notices.lock().expect("notices").push(line.to_string());
    }
}

/// Hermetic env + tool file. Sets `XDG_CACHE_HOME` and `XDG_STATE_HOME`.
fn hermetic_tool(
    name: &str,
) -> (
    tempfile::TempDir,
    tempfile::TempDir,
    ToolDef,
    MutexGuard<'static, ()>,
) {
    let guard = ENV.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    let cache = tempfile::tempdir().expect("cache tempdir");
    let state = tempfile::tempdir().expect("state tempdir");
    std::env::set_var("XDG_CACHE_HOME", cache.path());
    std::env::set_var("XDG_STATE_HOME", state.path());
    let tool_path = cache.path().join(format!("{name}.md"));
    std::fs::write(&tool_path, "# crmdb\nFabricate a small CRM schema.\n").expect("tool file");
    let tool = ToolDef {
        name: name.to_string(),
        description: "a small CRM database".to_string(),
        model: None,
        driver: "db".to_string(),
        interface: None,
        constraints: Vec::new(),
        allow_roots: Vec::new(),
        body: "You fabricate a two-table CRM.".to_string(),
        path: tool_path,
    };
    (cache, state, tool, guard)
}

fn args(list: &[&str]) -> Vec<String> {
    list.iter().map(|a| a.to_string()).collect()
}

// --- minimal PG wire client -------------------------------------------------

fn be_i32(buf: &[u8]) -> i32 {
    i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]])
}

/// StartupMessage (protocol 3.0) with user param.
fn startup_message() -> Vec<u8> {
    let mut body = Vec::new();
    body.extend_from_slice(&196_608_i32.to_be_bytes());
    for (key, value) in [("user", "tester"), ("database", "tester")] {
        body.extend_from_slice(key.as_bytes());
        body.push(0);
        body.extend_from_slice(value.as_bytes());
        body.push(0);
    }
    body.push(0);
    let mut out = ((body.len() as i32) + 4).to_be_bytes().to_vec();
    out.extend_from_slice(&body);
    out
}

/// SSLRequest (no tag, length-only framing).
fn ssl_request() -> Vec<u8> {
    let mut out = 8_i32.to_be_bytes().to_vec();
    out.extend_from_slice(&80_877_103_i32.to_be_bytes());
    out
}

/// Typed frontend message: 1 tag byte + i32 length + body.
fn typed(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend_from_slice(&((body.len() as i32) + 4).to_be_bytes());
    out.extend_from_slice(body);
    out
}

fn simple_query(sql: &str) -> Vec<u8> {
    let mut body = sql.as_bytes().to_vec();
    body.push(0);
    typed(b'Q', &body)
}

/// Read one backend message; returns (tag, body).
fn read_message(stream: &mut UnixStream) -> (u8, Vec<u8>) {
    let mut tag = [0u8; 1];
    stream.read_exact(&mut tag).expect("message tag");
    let mut len = [0u8; 4];
    stream.read_exact(&mut len).expect("message length");
    let body_len = be_i32(&len) as usize - 4;
    let mut body = vec![0u8; body_len];
    stream.read_exact(&mut body).expect("message body");
    (tag[0], body)
}

/// Extract a NUL-terminated string starting at `offset`; returns (text, next offset).
fn cstring_at(body: &[u8], offset: usize) -> (String, usize) {
    let end = body[offset..]
        .iter()
        .position(|b| *b == 0)
        .map(|p| offset + p)
        .unwrap_or(body.len());
    (
        String::from_utf8_lossy(&body[offset..end]).into_owned(),
        end + 1,
    )
}

/// ErrorResponse's 'C' (SQLSTATE) and 'M' (message) fields.
fn error_fields(body: &[u8]) -> (String, String) {
    let mut code = String::new();
    let mut message = String::new();
    let mut offset = 0;
    while offset < body.len() && body[offset] != 0 {
        let field = body[offset] as char;
        let (value, next) = cstring_at(body, offset + 1);
        match field {
            'C' => code = value,
            'M' => message = value,
            _ => {}
        }
        offset = next;
    }
    (code, message)
}

/// DataRow: i16 field count then (i32 len + bytes) per field.
fn data_row_fields(body: &[u8]) -> Vec<Option<String>> {
    let count = be_i32(&[body[0], body[1], 0, 0]) as usize >> 16; // i16 prefix
    let mut offset = 2;
    let mut fields = Vec::new();
    for _ in 0..count {
        let len = be_i32(&body[offset..offset + 4]);
        offset += 4;
        if len < 0 {
            fields.push(None);
        } else {
            fields.push(Some(
                String::from_utf8_lossy(&body[offset..offset + len as usize]).into_owned(),
            ));
            offset += len as usize;
        }
    }
    fields
}

/// RowDescription: i16 count then per-column name + fixed metadata.
fn row_description_columns(body: &[u8]) -> Vec<(String, i32, i16)> {
    let count = be_i32(&[body[0], body[1], 0, 0]) as usize >> 16;
    let mut offset = 2;
    let mut columns = Vec::new();
    for _ in 0..count {
        let (name, next) = cstring_at(body, offset);
        offset = next;
        let table_oid = be_i32(&body[offset..offset + 4]);
        let attnum = i16::from_be_bytes([body[offset + 4], body[offset + 5]]);
        let type_oid = be_i32(&body[offset + 6..offset + 10]);
        let typlen = i16::from_be_bytes([body[offset + 10], body[offset + 11]]);
        columns.push((name, type_oid, typlen));
        let _ = (table_oid, attnum);
        offset += 18; // 4 table oid + 2 attnum + 4 oid + 2 typlen + 4 typmod + 2 format
    }
    columns
}

fn wait_for_socket(path: &std::path::Path) {
    for _ in 0..500 {
        if path.exists() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("virtual-postgres socket never appeared: {}", path.display());
}

const VALID_SPEC: &str = r#"{"tables":[{"name":"contacts","columns":[{"name":"id","type":"INTEGER"},{"name":"email","type":"TEXT"}],"rows":[[1,"ada@example.com"],[2,"grace@example.com"]]}]}"#;

#[test]
fn wire_serves_fabricated_rows_and_refuses_abuse() {
    let (_cache, state, tool, _env) = hermetic_tool("crmdb");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit_path = audit_dir.path().join("audit.jsonl");

    let backend = Scripted::new(vec![VALID_SPEC.to_string()]);
    let sink = Shared::default();
    let notices = sink.notices.clone();

    // Serve on the default unix socket (blocks; test process tears it down).
    let tool_for_thread = tool.clone();
    let socket_path: PathBuf = state
        .path()
        .join("smah")
        .join("vtool-db")
        .join(format!("{}.sock", tool.name));
    let server = std::thread::spawn(move || {
        let audit = Audit::open_at(&audit_path, "run");
        let _ = db::run(
            &tool_for_thread,
            backend.as_ref(),
            &mut sink.clone(),
            "m",
            &args(&[]),
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
        );
    });

    wait_for_socket(&socket_path);
    let mut stream = UnixStream::connect(&socket_path).expect("connect");

    // SSL probe: refused with a single 'N'.
    stream.write_all(&ssl_request()).expect("ssl probe");
    let mut ssl_answer = [0u8; 1];
    stream.read_exact(&mut ssl_answer).expect("ssl answer");
    assert_eq!(ssl_answer[0], b'N', "no SSL — plain wire");

    // Startup: trust auth, branded ParameterStatus, BackendKeyData, ready.
    stream.write_all(&startup_message()).expect("startup");
    let mut server_version = String::new();
    let mut saw_auth_ok = false;
    let mut saw_key = false;
    loop {
        let (tag, body) = read_message(&mut stream);
        match tag {
            b'R' => {
                saw_auth_ok = true;
                assert_eq!(be_i32(&body), 0, "AuthenticationOk (trust)");
            }
            b'S' => {
                let (name, next) = cstring_at(&body, 0);
                let (value, _) = cstring_at(&body, next);
                if name == "server_version" {
                    server_version = value;
                }
            }
            b'K' => {
                saw_key = true;
                assert_eq!(body.len(), 8, "BackendKeyData pid+secret");
            }
            b'Z' => {
                assert_eq!(body, vec![b'I'], "idle");
                break;
            }
            other => panic!("unexpected handshake message {other}"),
        }
    }
    assert!(saw_auth_ok && saw_key);
    assert!(
        server_version.contains("smah-virtual-postgres"),
        "branded server_version: {server_version}"
    );
    assert!(
        server_version.contains("fabricated data"),
        "server_version discloses fabrication: {server_version}"
    );

    // SELECT round-trip: RowDescription (oids 23/25), DataRows, tag, ready.
    stream
        .write_all(&simple_query("SELECT id, email FROM contacts ORDER BY id;"))
        .expect("select");
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'T');
    let columns = row_description_columns(&body);
    assert_eq!(
        columns,
        vec![("id".to_string(), 23, 4), ("email".to_string(), 25, -1),],
        "oids + typlens from the fabricated decls"
    );
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'D');
    assert_eq!(
        data_row_fields(&body),
        vec![Some("1".to_string()), Some("ada@example.com".to_string())]
    );
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'D');
    assert_eq!(
        data_row_fields(&body),
        vec![Some("2".to_string()), Some("grace@example.com".to_string())]
    );
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'C');
    let (command_tag, _) = cstring_at(&body, 0);
    assert_eq!(command_tag, "SELECT 2");
    let (tag, body) = read_message(&mut stream);
    assert_eq!((tag, body.as_slice()), (b'Z', b"I" as &[u8]));

    // Write attempt: refused read-only (42501), connection stays usable.
    stream
        .write_all(&simple_query("DELETE FROM contacts"))
        .expect("delete");
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'E', "DELETE refused");
    let (code, message) = error_fields(&body);
    assert_eq!(code, "42501");
    assert!(message.contains("read-only"), "{message}");
    let (tag, _) = read_message(&mut stream);
    assert_eq!(tag, b'Z', "ready after refusal");

    // Empty query: EmptyQueryResponse then ready.
    stream.write_all(&simple_query(" ; ")).expect("empty");
    let (tag, _) = read_message(&mut stream);
    assert_eq!(tag, b'I', "EmptyQueryResponse");
    let (tag, _) = read_message(&mut stream);
    assert_eq!(tag, b'Z');

    // Extended protocol: one 0A000 refusal, then Sync recovers to idle.
    let parse_body: Vec<u8> = {
        let mut body = b"stmt1\0SELECT 1\0".to_vec();
        body.extend_from_slice(&0_i16.to_be_bytes());
        body
    };
    stream.write_all(&typed(b'P', &parse_body)).expect("parse");
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'E', "extended protocol refused");
    let (code, _) = error_fields(&body);
    assert_eq!(code, "0A000");
    stream.write_all(&typed(b'S', &[])).expect("sync");
    let (tag, body) = read_message(&mut stream);
    assert_eq!((tag, body.as_slice()), (b'Z', b"I" as &[u8]), "recovered");

    // Terminate: the server closes the connection.
    stream.write_all(&typed(b'X', &[])).expect("terminate");
    let mut probe = [0u8; 1];
    let closed = matches!(stream.read(&mut probe), Ok(0) | Err(_));
    assert!(closed, "connection closed after Terminate");

    // The disclosure line named the virtual postgres before serving.
    let notices = notices.lock().expect("notices");
    assert!(
        notices
            .iter()
            .any(|line| line.contains("smah-virtual-postgres") && line.contains("FABRICATED DATA")),
        "bind disclosure present: {notices:?}"
    );

    drop(server); // detached; the accept loop dies with the test process
}

#[test]
fn bad_specs_exhaust_the_retry_budget_and_abort() {
    let (_cache, _state, tool, _env) = hermetic_tool("crmdb2");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit = Audit::open_at(&audit_dir.path().join("audit.jsonl"), "run");

    let backend = Scripted::new(vec!["not json at all".to_string(), "{}".to_string()]);
    let err = db::run(
        &tool,
        backend.as_ref(),
        &mut Shared::default(),
        "m",
        &args(&["--show-schema"]),
        None,
        false,
        &FsMode::NoneDeclared,
        &audit,
        "run",
    )
    .expect_err("two bad specs");

    assert!(
        matches!(err, SmahError::DirectiveLoopOverflow { .. }),
        "loop budget exhausted: {err}"
    );
    assert_eq!(err.exit_code(), smah_core::exit_codes::ExitCode::Provider);
}

/// Real-model e2e: DashScope fabricates the spec, the wire serves it.
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

    let (_cache, state, tool, _env_guard) = hermetic_tool("crmdb-real");
    let audit_dir = tempfile::tempdir().expect("audit tempdir");
    let audit_path = audit_dir.path().join("audit.jsonl");
    let backend = RealBackend(smah_providers::AlibabaProvider::new(api_key).expect("provider"));
    let mut sink = Shared::default();

    let tool_for_thread = tool.clone();
    let socket_path: PathBuf = state
        .path()
        .join("smah")
        .join("vtool-db")
        .join(format!("{}.sock", tool.name));
    let server = std::thread::spawn(move || {
        let audit = Audit::open_at(&audit_path, "run");
        let _ = db::run(
            &tool_for_thread,
            &backend,
            &mut sink,
            "qwen3.8-flash",
            &args(&[]),
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
        );
    });

    wait_for_socket(&socket_path);
    let mut stream = UnixStream::connect(&socket_path).expect("connect");
    stream.write_all(&startup_message()).expect("startup");
    loop {
        let (tag, _) = read_message(&mut stream);
        if tag == b'Z' {
            break;
        }
    }
    stream
        .write_all(&simple_query(
            "SELECT name FROM sqlite_master WHERE type='table' ORDER BY name;",
        ))
        .expect("catalog query");
    let (tag, body) = read_message(&mut stream);
    assert_eq!(tag, b'T', "RowDescription for the catalog query");
    let columns = row_description_columns(&body);
    assert!(!columns.is_empty());
    // At least one fabricated table row must come back before the tag.
    let (tag, _) = read_message(&mut stream);
    assert!(
        matches!(tag, b'D' | b'C'),
        "rows or immediate completion, got {tag}"
    );

    drop(server);
}
