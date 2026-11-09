//! Postgres wire-protocol subset for the db virtual tool (ticket #24).
//!
//! The driver serves a **fabrication-only** surface: enough of the Postgres
//! frontend/backend protocol for `psql` and simple ORM SELECTs to talk to the
//! SQLite-backed virtual store over a unix socket or loopback TCP. It is NOT
//! a Postgres implementation:
//!
//! - `SSLRequest`/`GSSENCRequest` are answered with a single `'N'` (no TLS —
//!   the surface is loopback/unix-socket only; T-030);
//! - authentication is always `AuthenticationOk` (trust; local only);
//! - `server_version` is branded `smah-virtual-postgres …` so no client can
//!   mistake it for a real server (T-029);
//! - the **simple query protocol** (`'Q'`) is the supported path; extended
//!   protocol messages (`Parse`/`Bind`/…) get one `feature_not_supported`
//!   error and a clean recovery at `Sync` (documented client contract);
//! - statements are classified lexically: SELECT/WITH run against the
//!   read-only store; everything else is refused `insufficient_privilege`
//!   (the store connection itself is opened read-only, so the lexical gate
//!   is UX, not the security boundary);
//! - message and statement size caps abort abusive clients (T-030).
//!
//! All I/O is blocking `std::net` — no async runtime, no threads per message
//! (one connection at a time is served, sequentially).

use std::io::{Read, Write};

use crate::virtual_tools::audit::{Audit, AuditRecord};

/// Wire protocol 3.0 (`3 << 16 | 0`).
pub(crate) const PROTOCOL_MAJOR: i32 = 196608;
pub(crate) const SSL_REQUEST_CODE: i32 = 80877103;
pub(crate) const GSSENC_REQUEST_CODE: i32 = 80877104;
pub(crate) const CANCEL_REQUEST_CODE: i32 = 80877102;

/// The loud brand every client sees in `server_version` (T-029).
pub const SERVER_BRAND: &str = "smah-virtual-postgres";

/// A startup/query message body larger than this is abuse (T-30 cap).
pub(crate) const MESSAGE_MAX: usize = 256 * 1024;
/// A single statement larger than this is abuse (T-030 cap).
pub const STATEMENT_MAX: usize = 64 * 1024;

/// The statement kinds the subset serves.
pub(crate) const SUPPORTED_STATEMENT_KINDS: [&str; 2] = ["select", "with"];

/// Per-connection serve result.
#[derive(Debug)]
pub(crate) enum ConnEnd {
    /// Client went away / hung up cleanly.
    Closed,
    /// The connection was refused because a cap or protocol bound was hit.
    Rejected(String),
}

/// A wire-level I/O failure mid-connection.
#[derive(Debug)]
pub(crate) struct WireError(pub String);

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// --- byte helpers -----------------------------------------------------------

fn be_i32(buf: &[u8]) -> Option<i32> {
    if buf.len() < 4 {
        return None;
    }
    Some(i32::from_be_bytes([buf[0], buf[1], buf[2], buf[3]]))
}

/// Test-only: decode a big-endian i16 field (the wire builders emit several).
#[cfg(test)]
fn be_i16(buf: &[u8]) -> Option<i16> {
    if buf.len() < 2 {
        return None;
    }
    Some(i16::from_be_bytes([buf[0], buf[1]]))
}

fn push_i32(out: &mut Vec<u8>, v: i32) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn push_i16(out: &mut Vec<u8>, v: i16) {
    out.extend_from_slice(&v.to_be_bytes());
}

fn push_cstr(out: &mut Vec<u8>, s: &str) {
    out.extend_from_slice(s.as_bytes());
    out.push(0);
}

// --- message framing --------------------------------------------------------

/// Read one length-prefixed message body (no type byte — used for startup).
fn read_body<S: Read>(stream: &mut S, cap: usize) -> Result<Vec<u8>, WireError> {
    let mut len_buf = [0u8; 4];
    stream
        .read_exact(&mut len_buf)
        .map_err(|_| WireError("client closed during length".to_string()))?;
    let len = be_i32(&len_buf).ok_or_else(|| WireError("short length".into()))?;
    if !(4..=cap as i32).contains(&len) {
        return Err(WireError(format!("message length {len} outside 4..{cap}")));
    }
    let mut body = vec![0u8; (len - 4) as usize];
    stream
        .read_exact(&mut body)
        .map_err(|_| WireError("client closed during body".to_string()))?;
    Ok(body)
}

/// Read one typed message: the type byte, then the length-prefixed body.
fn read_message<S: Read>(stream: &mut S) -> Result<(u8, Vec<u8>), WireError> {
    let mut kind = [0u8; 1];
    stream
        .read_exact(&mut kind)
        .map_err(|_| WireError("client closed".to_string()))?;
    let body = read_body(stream, MESSAGE_MAX)?;
    Ok((kind[0], body))
}

/// Write one typed message (`len` covers itself + body, not the type byte).
pub(crate) fn write_message<S: Write>(
    stream: &mut S,
    kind: u8,
    body: &[u8],
) -> Result<(), WireError> {
    let mut len = [0u8; 4];
    len.copy_from_slice(&((body.len() as i32 + 4).to_be_bytes()));
    stream
        .write_all(&[kind])
        .and_then(|_| stream.write_all(&len))
        .and_then(|_| stream.write_all(body))
        .and_then(|_| stream.flush())
        .map_err(|err| WireError(format!("write failed: {err}")))
}

// --- server → client messages ----------------------------------------------

pub(crate) fn msg_authentication_ok() -> Vec<u8> {
    let mut out = Vec::new();
    push_i32(&mut out, 0); // AuthenticationOk
    out
}

pub(crate) fn msg_parameter_status(name: &str, value: &str) -> Vec<u8> {
    let mut out = Vec::new();
    push_cstr(&mut out, name);
    push_cstr(&mut out, value);
    out
}

pub(crate) fn msg_backend_key_data() -> Vec<u8> {
    let mut out = Vec::new();
    // pid + secret; both arbitrary but non-zero — clients only echo them on
    // cancel, and cancel requests are ignored by this subset.
    push_i32(&mut out, 0x534D_4148); // "SMAH"
    push_i32(&mut out, 0x5649_5254); // "VIRT"
    out
}

pub(crate) fn msg_ready_for_query() -> Vec<u8> {
    vec![b'I'] // idle
}

pub(crate) fn msg_empty_query() -> Vec<u8> {
    Vec::new()
}

/// RowDescription: one entry per column (text format).
pub(crate) fn msg_row_description(columns: &[(String, i32, i16)]) -> Vec<u8> {
    let mut out = Vec::new();
    push_i16(&mut out, columns.len() as i16);
    for (name, oid, typlen) in columns {
        push_cstr(&mut out, name);
        push_i32(&mut out, 0); // table oid
        push_i16(&mut out, 0); // column attnum
        push_i32(&mut out, *oid);
        push_i16(&mut out, *typlen);
        push_i32(&mut out, -1); // typmod
        push_i16(&mut out, 0); // format: text
    }
    out
}

/// DataRow in text format; `None` = NULL.
pub(crate) fn msg_data_row(values: &[Option<String>]) -> Vec<u8> {
    let mut out = Vec::new();
    push_i16(&mut out, values.len() as i16);
    for value in values {
        match value {
            None => push_i32(&mut out, -1),
            Some(text) => {
                push_i32(&mut out, text.len() as i32);
                out.extend_from_slice(text.as_bytes());
            }
        }
    }
    out
}

pub(crate) fn msg_command_complete(tag: &str) -> Vec<u8> {
    let mut out = Vec::new();
    push_cstr(&mut out, tag);
    out
}

/// ErrorResponse from severity/code/message fields.
pub(crate) fn msg_error_response(code: &str, message: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for (field, value) in [("S", "ERROR"), ("V", "ERROR"), ("C", code), ("M", message)] {
        out.push(field.as_bytes()[0]);
        push_cstr(&mut out, value);
    }
    out.push(0); // terminator
    out
}

// --- statement classification (lexical, quote-aware) ------------------------

/// Strip `--` and `/* */` comments without touching quoted literals.
pub(crate) fn strip_comments(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    let mut in_single = false;
    let mut in_double = false;
    while let Some(c) = chars.next() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                out.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                out.push(c);
            }
            '-' if !in_single && !in_double && chars.peek() == Some(&'-') => {
                chars.next();
                for skipped in chars.by_ref() {
                    if skipped == '\n' {
                        out.push('\n');
                        break;
                    }
                }
            }
            '/' if !in_single && !in_double && chars.peek() == Some(&'*') => {
                chars.next();
                while let Some(skipped) = chars.next() {
                    if skipped == '*' && chars.peek() == Some(&'/') {
                        chars.next();
                        break;
                    }
                }
            }
            _ => out.push(c),
        }
    }
    out
}

/// Split on top-level semicolons (quote-aware). Returns the statements.
pub(crate) fn split_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut in_single = false;
    let mut in_double = false;
    for c in sql.chars() {
        match c {
            '\'' if !in_double => {
                in_single = !in_single;
                current.push(c);
            }
            '"' if !in_single => {
                in_double = !in_double;
                current.push(c);
            }
            ';' if !in_single && !in_double => {
                statements.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    if !current.trim().is_empty() {
        statements.push(current);
    }
    statements
}

/// The first keyword of a statement, lowercased.
pub(crate) fn first_keyword(statement: &str) -> String {
    statement
        .split_whitespace()
        .next()
        .unwrap_or("")
        .to_ascii_lowercase()
}

// --- the serve loop ---------------------------------------------------------

/// Outcome of a finished simple query (for audit: kind + bytes + ok).
pub(crate) struct QueryAudit<'a> {
    pub audit: &'a Audit,
    pub run_id: &'a str,
    pub tool: &'a str,
}

fn audit_query(qa: &QueryAudit, kind: &str, bytes: usize, allowed: bool, reason: &str) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: qa.run_id.to_string(),
        tool: qa.tool.to_string(),
        op: "db.query".to_string(),
        path: kind.to_string(),
        resolved: None,
        bytes: Some(bytes),
        decision: if allowed { "allowed" } else { "denied" },
        reason: reason.to_string(),
        turn: 0,
    };
    if let Err(err) = qa.audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {err}");
    }
}

fn send_error<S: Write>(stream: &mut S, sql_code: &str, message: &str) -> Result<(), WireError> {
    write_message(stream, b'E', &msg_error_response(sql_code, message))
}

/// Columns (name, oid, typlen) plus text-encoded rows.
pub(crate) type QueryResult = (Vec<(String, i32, i16)>, Vec<Vec<Option<String>>>);

/// What the store gives the wire layer per statement.
pub(crate) trait QueryStore {
    /// Run a SELECT-shaped statement; returns columns (name, oid, typlen) and
    /// text-encoded rows.
    fn select(&self, sql: &str) -> Result<QueryResult, String>;
}

/// Serve one connection to completion (blocking). `qa` carries the audit
/// context; every statement (allowed or refused) is audited as `db.query`
/// with the statement KIND + byte length — never the text (T-029/T-030).
pub(crate) fn serve_connection<S: Read + Write, Q: QueryStore>(
    stream: &mut S,
    store: &Q,
    qa: &QueryAudit<'_>,
) -> Result<ConnEnd, WireError> {
    // --- startup (possibly after an SSL/GSS refusal) ---
    loop {
        let body = read_body(stream, MESSAGE_MAX)?;
        let code = be_i32(&body).ok_or_else(|| WireError("startup shorter than 4 bytes".into()))?;
        match code {
            SSL_REQUEST_CODE | GSSENC_REQUEST_CODE => {
                // Single byte 'N': no SSL, no GSS. Loop for the real startup.
                stream
                    .write_all(b"N")
                    .and_then(|_| stream.flush())
                    .map_err(|err| WireError(format!("ssl refusal write failed: {err}")))?;
            }
            CANCEL_REQUEST_CODE => return Ok(ConnEnd::Closed),
            PROTOCOL_MAJOR => break, // StartupMessage: params follow
            other => {
                return Ok(ConnEnd::Rejected(format!(
                    "unsupported startup code {other}"
                )))
            }
        }
    }

    // --- authentication (trust) + branding + ready ---
    write_message(stream, b'R', &msg_authentication_ok())?;
    let server_version = format!(
        "16.0 {} ({SERVER_BRAND}; fabricated data — not a real database)",
        env!("CARGO_PKG_VERSION")
    );
    for (name, value) in [
        ("server_version", server_version.as_str()),
        ("server_encoding", "UTF8"),
        ("client_encoding", "UTF8"),
        ("DateStyle", "ISO, MDY"),
        ("integer_datetimes", "on"),
        ("standard_conforming_strings", "on"),
        ("application_name", SERVER_BRAND),
        ("session_authorization", "smah_virtual"),
    ] {
        write_message(stream, b'S', &msg_parameter_status(name, value))?;
    }
    write_message(stream, b'K', &msg_backend_key_data())?;
    write_message(stream, b'Z', &msg_ready_for_query())?;

    // --- message loop (sequential; one connection at a time) ---
    let mut extended_refused = false;
    loop {
        let (kind, body) = read_message(stream)?;
        match kind {
            b'Q' => {
                let sql = cstring(&body);
                let bytes = sql.len();
                if bytes > STATEMENT_MAX {
                    audit_query(qa, "rejected:oversize", bytes, false, "statement cap");
                    send_error(
                        stream,
                        "54000",
                        "statement exceeds the smah virtual postgres statement cap",
                    )?;
                    write_message(stream, b'Z', &msg_ready_for_query())?;
                    continue;
                }
                let statements = split_statements(&strip_comments(&sql));
                let statements: Vec<&str> = statements
                    .iter()
                    .map(|s| s.trim())
                    .filter(|s| !s.is_empty())
                    .collect();
                if statements.len() > 1 {
                    audit_query(
                        qa,
                        "rejected:multiple",
                        bytes,
                        false,
                        "one statement per query",
                    );
                    send_error(
                        stream,
                        "0A000",
                        "multiple statements in one query are not supported by smah-virtual-postgres",
                    )?;
                    write_message(stream, b'Z', &msg_ready_for_query())?;
                    continue;
                }
                let Some(statement) = statements.first() else {
                    // Empty query (whitespace/';' only).
                    write_message(stream, b'I', &msg_empty_query())?;
                    write_message(stream, b'Z', &msg_ready_for_query())?;
                    continue;
                };
                let keyword = first_keyword(statement);
                if !SUPPORTED_STATEMENT_KINDS.contains(&keyword.as_str()) {
                    audit_query(
                        qa,
                        &format!("refused:{keyword}"),
                        bytes,
                        false,
                        "fabricated store is read-only; SELECT-only",
                    );
                    send_error(
                        stream,
                        "42501",
                        &format!(
                            "smah-virtual-postgres serves fabricated data and is read-only: \
                             '{keyword}' statements are refused (SELECT-only surface)"
                        ),
                    )?;
                    write_message(stream, b'Z', &msg_ready_for_query())?;
                    continue;
                }
                audit_query(qa, &keyword, bytes, true, "");
                match store.select(statement) {
                    Ok((columns, rows)) => {
                        write_message(stream, b'T', &msg_row_description(&columns))?;
                        for row in &rows {
                            write_message(stream, b'D', &msg_data_row(row))?;
                        }
                        write_message(
                            stream,
                            b'C',
                            &msg_command_complete(&format!("SELECT {}", rows.len())),
                        )?;
                    }
                    Err(detail) => {
                        send_error(stream, "42601", &detail)?;
                    }
                }
                write_message(stream, b'Z', &msg_ready_for_query())?;
            }
            b'X' => return Ok(ConnEnd::Closed),
            // Extended query protocol: one refusal, clean recovery at Sync.
            b'P' | b'B' | b'D' | b'E' | b'C' | b'F' => {
                if !extended_refused {
                    extended_refused = true;
                    audit_query(
                        qa,
                        "refused:extended",
                        body.len(),
                        false,
                        "extended query protocol unsupported",
                    );
                    send_error(
                        stream,
                        "0A000",
                        "extended query protocol is not supported by smah-virtual-postgres; \
                         use the simple query protocol (psql and SELECT-only clients)",
                    )?;
                }
                // 'H' (Flush) with a pending refusal: nothing more to say —
                // the flag keeps later extended messages silent until Sync.
            }
            b'S' => {
                extended_refused = false;
                write_message(stream, b'Z', &msg_ready_for_query())?;
            }
            b'p' => {
                send_error(
                    stream,
                    "28000",
                    "smah-virtual-postgres requires no authentication",
                )?;
            }
            other => {
                send_error(
                    stream,
                    "08P01",
                    &format!("unsupported message type '{}'", other as char),
                )?;
                return Ok(ConnEnd::Rejected(format!("bad message type {other}")));
            }
        }
    }
}

/// First NUL-terminated string of the body (the simple-query payload).
fn cstring(body: &[u8]) -> String {
    let end = body.iter().position(|b| *b == 0).unwrap_or(body.len());
    String::from_utf8_lossy(&body[..end]).into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_comments_is_quote_aware() {
        assert_eq!(
            strip_comments("SELECT 1 -- trailing\nFROM t"),
            "SELECT 1 \nFROM t"
        );
        assert_eq!(strip_comments("SELECT /* x */ 1"), "SELECT  1");
        assert_eq!(
            strip_comments("SELECT '--not a comment' AS v"),
            "SELECT '--not a comment' AS v"
        );
        assert_eq!(
            strip_comments("SELECT 'a/*b' , /* real */ 1"),
            "SELECT 'a/*b' ,  1"
        );
    }

    #[test]
    fn split_statements_is_quote_aware() {
        assert_eq!(
            split_statements("SELECT 1; SELECT 2;"),
            vec!["SELECT 1", " SELECT 2"]
        );
        assert_eq!(split_statements("SELECT 'a;b'"), vec!["SELECT 'a;b'"]);
        assert!(split_statements("  ").is_empty());
        assert_eq!(split_statements("SELECT 1"), vec!["SELECT 1"]);
    }

    #[test]
    fn first_keyword_is_case_insensitive() {
        assert_eq!(first_keyword("  SeLeCT 1"), "select");
        assert_eq!(first_keyword("DROP TABLE x"), "drop");
        assert_eq!(first_keyword(""), "");
    }

    #[test]
    fn error_response_carries_fields_and_terminator() {
        let body = msg_error_response("42501", "refused");
        let text = String::from_utf8_lossy(&body).into_owned();
        assert!(text.contains('S'));
        assert!(text.contains("42501"));
        assert!(text.contains("refused"));
        assert_eq!(body.last(), Some(&0));
    }

    #[test]
    fn row_description_encodes_columns() {
        let body = msg_row_description(&[("id".into(), 23, 4), ("email".into(), 25, -1)]);
        // 2 columns then per-column 1 name+NUL + 4 + 2 + 4 + 2 + 4 + 2 bytes.
        assert_eq!(be_i16(&body), Some(2));
    }
}
