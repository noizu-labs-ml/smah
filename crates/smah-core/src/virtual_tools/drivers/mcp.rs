//! MCP virtual-tool driver (ticket #23): the smah process IS the MCP server.
//!
//! Tool discovery is fabricated (one bounded model turn → validated catalog,
//! frozen under `mcp-tools.json`, reviewable with `--show-catalog`, T-027).
//! The JSON-RPC 2.0 surface rides newline-delimited stdio **batch mode** —
//! the prelude reads stdin to EOF, so every request present at invocation is
//! answered in order to stdout. Interactive clients use the opt-in loopback
//! TCP transport (`--tcp-port <n>`, loud disclosure, sequential connections,
//! T-028) or the standalone `services/mcp-vtool` daemon.
//!
//! No child processes, ever: "serving" is this process answering requests
//! inline (the no-shell gate stays intact). Methods: `initialize`, `ping`,
//! `tools/list`, `tools/call` (memoized per arguments), plus notifications.
//! Siblings (`jsonrpc`, `catalog`) are declared here with `#[path]` so
//! `drivers/mod.rs` stays untouched; the shared `support` sibling is owned
//! by the db driver.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::driver::{invocation_block, ChatMessage};
use crate::virtual_tools::drivers::db::support;
use crate::virtual_tools::drivers::{ChatBackend, FsMode, RunOutcome, Sink};
use crate::virtual_tools::loader::ToolDef;

#[path = "mcp_catalog.rs"]
pub(crate) mod catalog;
#[path = "mcp_jsonrpc.rs"]
pub(crate) mod jsonrpc;

use catalog::CatalogEntry;
use jsonrpc::{Request, RpcError, RpcHandler};

/// Protocol versions this server speaks (negotiates down to the baseline).
const SUPPORTED_PROTOCOL_VERSIONS: [&str; 2] = ["2024-11-05", "2025-06-18"];

#[derive(Debug, Clone, PartialEq, Eq)]
struct McpArgs {
    tcp_port: Option<u16>,
    no_serve: bool,
    show_catalog: bool,
    regen: bool,
}

fn parse_args(tool: &ToolDef, args: &[String]) -> Result<McpArgs, SmahError> {
    let invalid = |detail: String| SmahError::ToolFileInvalid {
        path: tool.path.display().to_string(),
        detail,
    };
    let mut parsed = McpArgs {
        tcp_port: None,
        no_serve: false,
        show_catalog: false,
        regen: false,
    };
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        match arg.as_str() {
            "--stdio" => {}
            "--no-serve" => parsed.no_serve = true,
            "--show-catalog" => parsed.show_catalog = true,
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
                parsed.tcp_port = Some(port);
            }
            other => {
                return Err(invalid(format!(
                    "the mcp driver does not recognize argument '{other}' \
                     (supported: --stdio, --no-serve, --show-catalog, --regen, --tcp-port <n>)"
                )))
            }
        }
    }
    Ok(parsed)
}

/// The method dispatcher: protocol handshake, discovery, and memoized
/// fabricated `tools/call`.
struct McpHandler<'a> {
    tool: &'a ToolDef,
    backend: &'a dyn ChatBackend,
    model: &'a str,
    audit: &'a Audit,
    run_id: &'a str,
    dir: PathBuf,
    catalog: Vec<CatalogEntry>,
}

impl McpHandler<'_> {
    fn initialize(&self, params: &Value) -> Result<Value, RpcError> {
        let requested = params
            .get("protocolVersion")
            .and_then(Value::as_str)
            .unwrap_or("");
        let version = if SUPPORTED_PROTOCOL_VERSIONS.contains(&requested) {
            requested
        } else {
            SUPPORTED_PROTOCOL_VERSIONS[0]
        };
        Ok(json!({
            "protocolVersion": version,
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {
                "name": format!("smah-vtool-{}", self.tool.name),
                "version": env!("CARGO_PKG_VERSION"),
            },
            "instructions": self.tool.description,
        }))
    }

    fn tools_call(&mut self, params: &Value) -> Result<Value, RpcError> {
        let name = params
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| RpcError::new(jsonrpc::INVALID_PARAMS, "'name' must be a string"))?;
        let arguments = match params.get("arguments") {
            None | Some(Value::Null) => json!({}),
            Some(a @ Value::Object(_)) => a.clone(),
            Some(_) => {
                return Err(RpcError::new(
                    jsonrpc::INVALID_PARAMS,
                    "'arguments' must be an object",
                ))
            }
        };
        let entry = self
            .catalog
            .iter()
            .find(|e| e.name == name)
            .ok_or_else(|| {
                let available: Vec<String> = self.catalog.iter().map(|e| e.name.clone()).collect();
                RpcError::new(
                    jsonrpc::INVALID_PARAMS,
                    format!(
                        "no such tool '{name}' (available: {})",
                        available.join(", ")
                    ),
                )
            })?
            .clone();
        let args_bytes = arguments.to_string().len();
        support::audit_server_op(
            self.audit,
            self.run_id,
            &self.tool.name,
            "mcp.call",
            name,
            Some(args_bytes),
            "allowed",
            "",
        );

        // Memo first: identical calls never re-fabricate.
        let key = catalog::memo_key(name, &arguments);
        let text = match catalog::memo_get(&self.dir, &key) {
            Some(text) => text,
            None => {
                let fabricated = self.fabricate_call(&entry, &arguments)?;
                let _ = catalog::memo_put(&self.dir, &key, name, &fabricated);
                fabricated
            }
        };
        Ok(json!({
            "content": [{"type": "text", "text": text}],
            "isError": false,
        }))
    }

    /// One bounded fabrication turn for a single tool call.
    fn fabricate_call(&self, entry: &CatalogEntry, arguments: &Value) -> Result<String, RpcError> {
        let mut user = String::new();
        user.push_str("VIRTUAL MCP SERVER PERSONA:\n");
        user.push_str(&self.tool.body);
        if !self.tool.constraints.is_empty() {
            user.push_str("\n\nHard constraints:\n");
            for constraint in &self.tool.constraints {
                user.push_str(&format!("- {constraint}\n"));
            }
        }
        user.push_str("\n\nCalled tool:\n");
        user.push_str(&entry.to_wire().to_string());
        user.push_str("\n\nArguments:\n");
        user.push_str(&arguments.to_string());
        user.push_str(
            "\n\nProduce only the result text for this call now (it becomes content[0].text).",
        );
        let mut text = self
            .backend
            .stream_turn(
                self.model,
                MCP_CALL_SYSTEM,
                &[ChatMessage::user(&user)],
                &mut |_| {},
            )
            .map_err(|err| RpcError::new(jsonrpc::INTERNAL, err.to_string()))?;
        if text.len() > catalog::CALL_TEXT_MAX {
            text.truncate(catalog::CALL_TEXT_MAX);
            text.push_str("\n[smah: truncated at the 64 KiB tool-result cap]");
        }
        Ok(text)
    }
}

const MCP_CALL_SYSTEM: &str = concat!(
    "You are answering one tool call on a VIRTUAL MCP server.\n",
    "Produce ONLY the result text the MCP client should see (it becomes content[0].text):\n",
    "no preamble, no JSON-RPC framing, no restating the arguments.\n",
    "Be coherent with the tool persona, the tool's schema, and the given arguments;\n",
    "keep the same facts stable across calls to the same tool."
);

impl RpcHandler for McpHandler<'_> {
    fn call(&mut self, request: &Request) -> Result<Value, RpcError> {
        match request.method.as_str() {
            "initialize" => self.initialize(&request.params),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({
                "tools": self.catalog.iter().map(CatalogEntry::to_wire).collect::<Vec<_>>()
            })),
            "tools/call" => self.tools_call(&request.params),
            method if method.starts_with("notifications/") => Err(RpcError::new(
                jsonrpc::INVALID_REQUEST,
                "notifications must not carry an id",
            )),
            _ => Err(RpcError::new(
                jsonrpc::METHOD_NOT_FOUND,
                format!(
                    "unknown method '{}' (supported: initialize, ping, tools/list, tools/call)",
                    request.method
                ),
            )),
        }
    }
}

/// Run the mcp driver: fabricate (or reuse) the catalog, then serve or print.
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
    let dir = support::cache_leaf(&tool.name, "mcp");

    let (catalog_entries, turns) =
        catalog::ensure_catalog(tool, backend, model, audit, run_id, &dir, parsed.regen)?;

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

    if parsed.show_catalog {
        let wire: Vec<Value> = catalog_entries.iter().map(CatalogEntry::to_wire).collect();
        let pretty = serde_json::to_string_pretty(&wire).map_err(|err| SmahError::Internal {
            detail: err.to_string(),
        })?;
        sink.content(&format!("{pretty}\n"));
        return Ok(outcome(invocation));
    }
    if parsed.no_serve {
        sink.notice(&format!(
            "smah(mcp): fabricated tool catalog for '{}' is ready at {} ({} tools; \
             rerun without --no-serve to serve it)",
            tool.name,
            catalog::catalog_path(&dir).display(),
            catalog_entries.len()
        ));
        return Ok(outcome(invocation));
    }

    let mut handler = McpHandler {
        tool,
        backend,
        model,
        audit,
        run_id,
        dir,
        catalog: catalog_entries,
    };

    if let Some(port) = parsed.tcp_port {
        let listener = std::net::TcpListener::bind(("127.0.0.1", port)).map_err(|err| {
            SmahError::ToolFileInvalid {
                path: format!("127.0.0.1:{port}"),
                detail: format!(
                    "the mcp driver could not bind its loopback TCP listener: {}",
                    support::one_line(&err.to_string())
                ),
            }
        })?;
        let bound = listener
            .local_addr()
            .map(|addr| addr.port())
            .unwrap_or(port);
        sink.notice(&format!(
            "smah(mcp): '{}' virtual MCP server on 127.0.0.1:{bound} — loopback only, \
             newline-delimited JSON-RPC 2.0, sequential connections (ctrl-c to stop)",
            tool.name
        ));
        support::audit_server_op(
            audit,
            run_id,
            &tool.name,
            "mcp.serve",
            "tcp-loopback",
            Some(bound as usize),
            "allowed",
            "",
        );
        jsonrpc::serve_tcp(listener, handler, &mut |line: &str| sink.notice(line)).map_err(
            |err| SmahError::Internal {
                detail: format!("mcp tcp serve: {}", support::one_line(&err.to_string())),
            },
        )?;
        return Ok(outcome(invocation)); // unreachable in practice
    }

    // Default: newline-delimited batch over the invocation's stdin.
    let stdin_text = stdin.unwrap_or("");
    if stdin_text.trim().is_empty() {
        sink.notice(
            "smah(mcp): no requests on stdin (batch mode reads stdin to EOF); \
             start an interactive client with --tcp-port <n> or the mcp-vtool service",
        );
        return Ok(outcome(invocation));
    }
    let answered = jsonrpc::serve_stdio_batch(stdin_text, stdin_partial, sink, &mut handler);
    support::audit_server_op(
        audit,
        run_id,
        &tool.name,
        "mcp.serve",
        "stdio-batch",
        Some(answered),
        "allowed",
        "",
    );
    Ok(outcome(invocation))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_tools::drivers::stub_test_support::{NullBackend, NullSink};

    fn tool() -> ToolDef {
        ToolDef {
            name: "crm".to_string(),
            description: "fixture CRM".to_string(),
            model: None,
            driver: "mcp".to_string(),
            interface: None,
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            body: "persona".to_string(),
            path: std::path::PathBuf::from("crm.md"),
        }
    }

    fn s(args: &[&str]) -> Vec<String> {
        args.iter().map(|a| a.to_string()).collect()
    }

    #[test]
    fn args_parse_the_contract_flags() {
        let t = tool();
        let parsed = parse_args(&t, &[]).expect("empty");
        assert_eq!(
            parsed,
            McpArgs {
                tcp_port: None,
                no_serve: false,
                show_catalog: false,
                regen: false
            }
        );
        let parsed = parse_args(
            &t,
            &s(&["--stdio", "--no-serve", "--show-catalog", "--regen"]),
        )
        .expect("flags");
        assert!(parsed.no_serve && parsed.show_catalog && parsed.regen);
        let parsed = parse_args(&t, &s(&["--tcp-port", "4749"])).expect("tcp");
        assert_eq!(parsed.tcp_port, Some(4749));
    }

    #[test]
    fn args_reject_unknown_flags_and_bad_ports() {
        let t = tool();
        for bad in [vec!["--tools"], vec!["positional"], vec!["--tcp-port", "x"]] {
            let err = parse_args(&t, &s(&bad)).expect_err(bad.join(" ").as_str());
            assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Config);
            assert!(err.to_string().contains("mcp driver"), "{err}");
        }
    }

    fn handler<'a>(
        t: &'a ToolDef,
        dir: &'a std::path::Path,
        backend: &'a dyn ChatBackend,
        audit: &'a Audit,
    ) -> McpHandler<'a> {
        McpHandler {
            tool: t,
            backend,
            model: "m",
            audit,
            run_id: "run",
            dir: dir.to_path_buf(),
            catalog: vec![CatalogEntry {
                name: "search".to_string(),
                description: "searches".to_string(),
                input_schema: json!({"type": "object"}),
            }],
        }
    }

    #[test]
    fn handler_answers_initialize_ping_list_and_unknown() {
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let backend = NullBackend;
        let mut h = handler(&t, dir.path(), &backend, &audit);

        let init = h
            .call(&Request {
                method: "initialize".to_string(),
                params: json!({"protocolVersion": "2024-11-05"}),
                id: Some(json!(1)),
            })
            .expect("initialize");
        assert_eq!(init["protocolVersion"], json!("2024-11-05"));
        assert_eq!(init["serverInfo"]["name"], json!("smah-vtool-crm"));
        // Unsupported version negotiates down to the baseline.
        let init = h
            .call(&Request {
                method: "initialize".to_string(),
                params: json!({"protocolVersion": "1999-01-01"}),
                id: Some(json!(2)),
            })
            .expect("initialize");
        assert_eq!(init["protocolVersion"], json!("2024-11-05"));

        let ping = h
            .call(&Request {
                method: "ping".to_string(),
                params: json!({}),
                id: Some(json!(3)),
            })
            .expect("ping");
        assert_eq!(ping, json!({}));

        let list = h
            .call(&Request {
                method: "tools/list".to_string(),
                params: json!({}),
                id: Some(json!(4)),
            })
            .expect("list");
        assert_eq!(list["tools"][0]["name"], json!("search"));
        assert_eq!(list["tools"][0]["inputSchema"]["type"], json!("object"));

        let unknown = h
            .call(&Request {
                method: "resources/list".to_string(),
                params: json!({}),
                id: Some(json!(5)),
            })
            .expect_err("unknown method");
        assert_eq!(unknown.code, jsonrpc::METHOD_NOT_FOUND);

        let notif_with_id = h
            .call(&Request {
                method: "notifications/initialized".to_string(),
                params: json!({}),
                id: Some(json!(6)),
            })
            .expect_err("notification with id");
        assert_eq!(notif_with_id.code, jsonrpc::INVALID_REQUEST);
    }

    #[test]
    fn tools_call_refuses_unknown_tools_and_requires_object_arguments() {
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let backend = NullBackend;
        let mut h = handler(&t, dir.path(), &backend, &audit);

        let err = h
            .call(&Request {
                method: "tools/call".to_string(),
                params: json!({"name": "nope", "arguments": {}}),
                id: Some(json!(1)),
            })
            .expect_err("unknown tool");
        assert_eq!(err.code, jsonrpc::INVALID_PARAMS);
        assert!(err.message.contains("available: search"));

        let err = h
            .call(&Request {
                method: "tools/call".to_string(),
                params: json!({"name": "search", "arguments": [1]}),
                id: Some(json!(2)),
            })
            .expect_err("array arguments");
        assert_eq!(err.code, jsonrpc::INVALID_PARAMS);

        let err = h
            .call(&Request {
                method: "tools/call".to_string(),
                params: json!({"arguments": {}}),
                id: Some(json!(3)),
            })
            .expect_err("missing name");
        assert_eq!(err.code, jsonrpc::INVALID_PARAMS);
    }

    #[test]
    fn tools_call_fabricates_once_then_memoizes() {
        /// Returns the same fabricated text on every turn; the memo makes the
        /// second identical call backend-free (the script would panic if it
        /// were consulted twice through mutable state, but a constant is
        /// sufficient to prove the memo answered: the file gains an entry).
        struct Constant;
        impl ChatBackend for Constant {
            fn stream_turn(
                &self,
                _model: &str,
                _system: &str,
                _messages: &[ChatMessage],
                _on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                Ok("the fabricated search result".to_string())
            }
        }
        let t = tool();
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let backend = Constant;
        let mut h = handler(&t, dir.path(), &backend, &audit);
        let params = json!({"name": "search", "arguments": {"q": "ada"}});
        let first = h
            .call(&Request {
                method: "tools/call".to_string(),
                params: params.clone(),
                id: Some(json!(1)),
            })
            .expect("first call");
        assert_eq!(
            first["content"][0]["text"],
            json!("the fabricated search result")
        );
        assert_eq!(first["isError"], json!(false));
        // The memo file now holds the fabricated result keyed by the call.
        let memo = std::fs::read_to_string(catalog::memo_path(dir.path())).expect("memo written");
        assert!(memo.contains("the fabricated search result"));
        let second = h
            .call(&Request {
                method: "tools/call".to_string(),
                params,
                id: Some(json!(2)),
            })
            .expect("memoized call");
        assert_eq!(
            second["content"][0]["text"],
            json!("the fabricated search result")
        );
        let _ = NullSink;
    }

    #[test]
    fn run_refuses_unknown_flags_before_any_model_call() {
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join("audit.jsonl"), "run");
        let t = tool();
        let mut sink = NullSink;
        let err = run(
            &t,
            &NullBackend,
            &mut sink,
            "m",
            &s(&["--tools"]),
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
