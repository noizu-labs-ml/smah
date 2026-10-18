//! DB virtual-tool driver (ticket #24): a virtual postgres that serves the
//! tool's **fabricated** schema over a PG-wire-protocol subset.
//!
//! - The LLM fabricates a structured JSON schema spec (never SQL); this lane
//!   builds `~/.cache/smah/virtual-tools/<tool>/db/store.sqlite` itself and
//!   serves it **read-only** (`--regen` forces a rebuild, a changed tool file
//!   re-keys the cache). No kernel mounts, no FUSE, no real postgres.
//! - Serving surface: a unix socket at
//!   `~/.local/state/smah/vtool-db/<tool>.sock` by default, or loopback TCP
//!   with `--tcp-port <n>` (port `0` = ephemeral). Every bind prints a loud
//!   disclosure naming `smah-virtual-postgres` and `FABRICATED DATA` (T-029).
//! - Flags: `--stdio` (accepted no-op), `--no-serve` (fabricate only),
//!   `--show-schema` (print the fabricated schema and exit), `--regen`,
//!   `--tcp-port <n>`. Any other flag or positional is a Config-class error
//!   (exit 3).
//!
//! Sibling modules (`wire`, `store`, shared `support`) are declared here with
//! `#[path]` so `drivers/mod.rs` stays untouched; the other lanes reach
//! `support` through this module.

use std::net::TcpListener;
use std::os::unix::net::UnixListener;
use std::path::PathBuf;

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::driver::{invocation_block, ChatMessage};
use crate::virtual_tools::drivers::{ChatBackend, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;

#[path = "db_store.rs"]
pub(crate) mod store;
#[path = "vtool_support.rs"]
pub(crate) mod support;
#[path = "db_wire.rs"]
pub(crate) mod wire;

use wire::QueryStore;

/// Serving endpoint chosen from the driver args.
#[derive(Debug, Clone, PartialEq, Eq)]
enum ServeMode {
    /// Default: unix socket under `~/.local/state/smah/vtool-db/`.
    Socket,
    /// `--tcp-port <n>`: loopback TCP (`0` = ephemeral port).
    Tcp(u16),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DbArgs {
    serve: Option<ServeMode>,
    no_serve: bool,
    show_schema: bool,
    regen: bool,
}

fn parse_args(tool: &ToolDef, args: &[String]) -> Result<DbArgs, SmahError> {
    let invalid = |detail: String| SmahError::ToolFileInvalid {
        path: tool.path.display().to_string(),
        detail,
    };
    let mut parsed = DbArgs {
        serve: None,
        no_serve: false,
        show_schema: false,
        regen: false,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--stdio" => {}
            "--no-serve" => parsed.no_serve = true,
            "--show-schema" => parsed.show_schema = true,
            "--regen" => parsed.regen = true,
            "--tcp-port" => {
                let value = iter.next().ok_or_else(|| {
                    invalid(format!(
                        "the {} driver requires --tcp-port <n>",
                        tool.driver
                    ))
                })?;
                let port = value.parse::<u16>().map_err(|_| {
                    invalid(format!(
                        "the {} driver rejects --tcp-port value '{value}' \
                         (not a port number, 0-65535)",
                        tool.driver
                    ))
                })?;
                parsed.serve = Some(ServeMode::Tcp(port));
            }
            other => {
                return Err(invalid(format!(
                    "the db driver does not recognize argument '{other}' \
                     (supported: --stdio, --no-serve, --show-schema, --regen, --tcp-port <n>)"
                )))
            }
        }
    }
    Ok(parsed)
}

/// Run the db driver: fabricate (or reuse) the store, then serve it or print.
#[allow(clippy::too_many_arguments)]
pub fn run(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    _fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    let parsed = parse_args(tool, args)?;
    let dir = support::cache_leaf(&tool.name, "db");

    // Fabrication/reuse — one bounded model turn per attempt (+1 retry).
    let (_store_path, turns) =
        store::ensure_store(tool, backend, model, audit, run_id, &dir, parsed.regen)?;

    let outcome = |messages: Vec<ChatMessage>| RunOutcome {
        turns,
        wrote_files: false,
        messages,
    };
    let invocation = vec![ChatMessage::user(invocation_block(
        args,
        stdin,
        stdin_partial,
    ))];

    if parsed.show_schema {
        let summary = show_schema(tool, &dir)?;
        sink.content(&summary);
        return Ok(outcome(invocation));
    }
    if parsed.no_serve {
        sink.notice(&format!(
            "smah(db): fabricated virtual database for '{}' is ready at {} \
             (FABRICATED DATA; rerun without --no-serve to serve it)",
            tool.name,
            store::store_path(&dir).display()
        ));
        return Ok(outcome(invocation));
    }

    match parsed.serve.unwrap_or(ServeMode::Socket) {
        ServeMode::Socket => serve_socket(tool, sink, audit, run_id, &dir),
        ServeMode::Tcp(port) => serve_tcp(tool, sink, audit, run_id, &dir, port),
    }
}

/// Bind + serve on the default unix socket. Blocks until the process is
/// stopped (connections are served sequentially — one at a time, T-030).
fn serve_socket(
    tool: &ToolDef,
    sink: &mut dyn Sink,
    audit: &Audit,
    run_id: &str,
    dir: &std::path::Path,
) -> Result<RunOutcome, SmahError> {
    let path: PathBuf = crate::virtual_tools::audit::state_dir()
        .join("vtool-db")
        .join(format!("{}.sock", tool.name));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|err| SmahError::ToolFileInvalid {
            path: path.display().to_string(),
            detail: format!("socket dir: {}", support::one_line(&err.to_string())),
        })?;
    }
    // A stale socket file from a previous run would fail bind().
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).map_err(|err| SmahError::ToolFileInvalid {
        path: path.display().to_string(),
        detail: format!(
            "the db driver could not bind its virtual-postgres socket: {}",
            support::one_line(&err.to_string())
        ),
    })?;
    sink.notice(&format!(
        "smah(db): '{}' — smah-virtual-postgres (FABRICATED DATA, read-only) \
         serving on {} (ctrl-c to stop; --no-serve skips serving)",
        tool.name,
        path.display()
    ));
    support::audit_server_op(
        audit,
        run_id,
        &tool.name,
        "db.serve",
        "unix-socket",
        None,
        "allowed",
        "",
    );
    accept_loop(listener.incoming(), tool, sink, audit, run_id, dir)
}

/// Bind + serve on loopback TCP (`--tcp-port`; `0` = ephemeral).
fn serve_tcp(
    tool: &ToolDef,
    sink: &mut dyn Sink,
    audit: &Audit,
    run_id: &str,
    dir: &std::path::Path,
    port: u16,
) -> Result<RunOutcome, SmahError> {
    let listener =
        TcpListener::bind(("127.0.0.1", port)).map_err(|err| SmahError::ToolFileInvalid {
            path: format!("127.0.0.1:{port}"),
            detail: format!(
                "the db driver could not bind its virtual-postgres TCP listener: {}",
                support::one_line(&err.to_string())
            ),
        })?;
    let bound = listener
        .local_addr()
        .map(|addr| addr.port())
        .unwrap_or(port);
    sink.notice(&format!(
        "smah(db): '{}' — smah-virtual-postgres (FABRICATED DATA, read-only) \
         serving on 127.0.0.1:{bound} — loopback only (ctrl-c to stop; --no-serve skips serving)",
        tool.name
    ));
    support::audit_server_op(
        audit,
        run_id,
        &tool.name,
        "db.serve",
        "tcp-loopback",
        Some(bound as usize),
        "allowed",
        "",
    );
    accept_loop(listener.incoming(), tool, sink, audit, run_id, dir)
}

/// Sequential accept loop shared by both transports. Store opened once,
/// read-only; every connection audited; abusive connections rejected with an
/// audit line and the loop keeps serving.
fn accept_loop<L, S>(
    listener: L,
    tool: &ToolDef,
    _sink: &mut dyn Sink,
    audit: &Audit,
    run_id: &str,
    dir: &std::path::Path,
) -> Result<RunOutcome, SmahError>
where
    L: Iterator<Item = std::io::Result<S>>,
    S: std::io::Read + std::io::Write,
{
    let store_path = store::store_path(dir);
    let db =
        store::DbStore::open_readonly(&store_path).map_err(|err| SmahError::ToolFileInvalid {
            path: store_path.display().to_string(),
            detail: err,
        })?;
    let qa = wire::QueryAudit {
        audit,
        run_id,
        tool: &tool.name,
    };
    for incoming in listener {
        match incoming {
            Ok(mut stream) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "db.serve",
                    "connection",
                    None,
                    "allowed",
                    "",
                );
                match wire::serve_connection(&mut stream, &db, &qa) {
                    Ok(wire::ConnEnd::Closed) => {}
                    Ok(wire::ConnEnd::Rejected(reason)) => {
                        support::audit_server_op(
                            audit, run_id, &tool.name, "db.serve", "rejected", None, "denied",
                            &reason,
                        );
                        let _ = stream.flush();
                    }
                    Err(err) => {
                        support::audit_server_op(
                            audit,
                            run_id,
                            &tool.name,
                            "db.serve",
                            "wire-error",
                            None,
                            "denied",
                            &err.0,
                        );
                    }
                }
            }
            Err(err) => {
                eprintln!(
                    "smah(db): accept failed: {}",
                    support::one_line(&err.to_string())
                );
            }
        }
    }
    Ok(RunOutcome {
        turns: 0,
        wrote_files: false,
        messages: Vec::new(),
    })
}

/// `--show-schema`: print the fabricated schema (code reads `sqlite_master`;
/// the output is deterministic, never model text).
fn show_schema(tool: &ToolDef, dir: &std::path::Path) -> Result<String, SmahError> {
    let store_path = store::store_path(dir);
    let db =
        store::DbStore::open_readonly(&store_path).map_err(|err| SmahError::ToolFileInvalid {
            path: store_path.display().to_string(),
            detail: err,
        })?;
    let (columns, rows) = db
        .select(
            "SELECT name FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%' ORDER BY name",
        )
        .map_err(|err| SmahError::ToolFileInvalid {
            path: store_path.display().to_string(),
            detail: err,
        })?;
    let mut out = format!(
        "virtual schema '{}' — smah-virtual-postgres (FABRICATED DATA, read-only)\n",
        tool.name
    );
    for row in &rows {
        let Some(table) = row.first().and_then(Option::as_ref) else {
            continue;
        };
        let summary_row = db
            .select(&format!(
                "SELECT (SELECT group_concat(name || ' ' || lower(type), ', ') FROM pragma_table_info('{table}')), \
                 (SELECT count(*) FROM \"{table}\")"
            ))
            .map_err(|err| SmahError::ToolFileInvalid {
                path: store_path.display().to_string(),
                detail: err,
            })?
            .1
            .into_iter()
            .next()
            .unwrap_or_default();
        let cols = summary_row
            .first()
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_default();
        let nrows = summary_row
            .get(1)
            .and_then(Option::as_ref)
            .cloned()
            .unwrap_or_else(|| "0".to_string());
        out.push_str(&format!("  {table}({cols})  rows={nrows}\n"));
    }
    let _ = columns;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolDef {
        ToolDef {
            name: "crmdb".to_string(),
            description: "fixture".to_string(),
            model: None,
            driver: "db".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "persona".to_string(),
            path: std::path::PathBuf::from("crmdb.md"),
        }
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn args_parse_the_contract_flags() {
        let t = tool();
        let parsed = parse_args(&t, &[]).expect("empty");
        assert_eq!(parsed.serve, None);
        assert!(!parsed.no_serve && !parsed.show_schema && !parsed.regen);

        let parsed = parse_args(&t, &s(&["--stdio"])).expect("stdio");
        assert_eq!(parsed.serve, None);

        let parsed =
            parse_args(&t, &s(&["--no-serve", "--show-schema", "--regen"])).expect("flags");
        assert!(parsed.no_serve && parsed.show_schema && parsed.regen);

        let parsed = parse_args(&t, &s(&["--tcp-port", "0"])).expect("ephemeral");
        assert_eq!(parsed.serve, Some(ServeMode::Tcp(0)));

        let parsed = parse_args(&t, &s(&["--tcp-port", "55432"])).expect("tcp");
        assert_eq!(parsed.serve, Some(ServeMode::Tcp(55432)));
    }

    #[test]
    fn args_reject_unknown_flags_and_bad_ports() {
        let t = tool();
        for bad in [
            vec!["--fusion"],
            vec!["extra"],
            vec!["--tcp-port"],
            vec!["--tcp-port", "http"],
            vec!["--tcp-port", "99999"],
        ] {
            let err = parse_args(&t, &s(&bad)).expect_err(bad.join(" ").as_str());
            assert_eq!(
                err.exit_code(),
                crate::exit_codes::ExitCode::Config,
                "{bad:?}: driver arg errors are Config class"
            );
            assert!(
                err.to_string().contains("db driver"),
                "{bad:?}: message names the driver: {err}"
            );
        }
    }

    #[test]
    fn run_refuses_unknown_flags_before_any_model_call() {
        use crate::virtual_tools::drivers::stub_test_support::{NullBackend, NullSink};
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let t = tool();
        let mut sink = NullSink;
        let err = run(
            &t,
            &NullBackend,
            &mut sink,
            "m",
            &s(&["--fusion"]),
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
        )
        .expect_err("unknown flag");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    }
}
