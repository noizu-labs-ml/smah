//! gRPC contract resolution, validation, and the freeze cache (ticket #21).
//!
//! Interface-first like the SOAP lane: the tool file's `interface:` declares
//! the `.proto` (inline or path/`$VAR`). The driver never invents a contract
//! — a missing or invalid one is a hard `ToolFileInvalid` error (exit 3).
//!
//! Validation runs at load time through `protox` (a pure-Rust protobuf
//! compiler — no `protoc` binary) and `prost-reflect`'s descriptor pool:
//! a contract must parse, declare **exactly one service**, at most
//! [`METHOD_MAX`] methods, and at least one unary method (streaming methods
//! are kept but refused at call time — unary-only v1).
//!
//! A validated contract is frozen under
//! `~/.cache/smah/virtual-tools/<tool>/grpc/` with two pins:
//!
//! - `contract.sha256` — integrity of the frozen `.proto` bytes;
//! - `tool.sha256` — staleness marker (a changed tool file re-freezes).
//!
//! On every load the frozen proto is re-validated ("validate-on-load"), so a
//! cache that passes integrity but not validation is re-frozen from the tool
//! file.

use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{cache_dir, Audit, AuditRecord};
use crate::virtual_tools::loader::ToolDef;

/// Cap on the contract artifact size (inline text or referenced file).
pub(crate) const CONTRACT_MAX: usize = 512 * 1024;

/// Maximum methods per service (parse-DoS bound).
pub(crate) const METHOD_MAX: usize = 64;

/// One declared method (metadata only; the descriptor pool carries types).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct GrpcMethod {
    /// `rpc` name.
    pub(crate) name: String,
    /// Fully-qualified input message type.
    pub(crate) input: String,
    /// Fully-qualified output message type.
    pub(crate) output: String,
    pub(crate) client_streaming: bool,
    pub(crate) server_streaming: bool,
}

impl GrpcMethod {
    pub(crate) fn unary(&self) -> bool {
        !self.client_streaming && !self.server_streaming
    }
}

/// The frozen, validated contract.
#[derive(Debug, Clone)]
pub(crate) struct GrpcContract {
    /// The `.proto` source served to the compiler on every load.
    pub(crate) proto: String,
    /// Fully-qualified service name (`package.Service`).
    pub(crate) service_name: String,
    /// Methods declared in the service (document order).
    pub(crate) methods: Vec<GrpcMethod>,
}

impl GrpcContract {
    pub(crate) fn method(&self, name: &str) -> Option<&GrpcMethod> {
        self.methods.iter().find(|m| m.name == name)
    }

    pub(crate) fn unary_methods(&self) -> Vec<&GrpcMethod> {
        self.methods.iter().filter(|m| m.unary()).collect()
    }
}

/// Compile the proto source and validate its shape into a [`GrpcContract`].
pub(crate) fn validate(tool: &ToolDef, proto: &str) -> Result<GrpcContract, SmahError> {
    if proto.len() > CONTRACT_MAX {
        return Err(contract_error(
            tool,
            &format!("`interface:` proto exceeds the {CONTRACT_MAX}-byte cap"),
        ));
    }

    let pool = compile_pool(tool, proto)?;
    let services: Vec<_> = pool.services().collect();
    if services.len() != 1 {
        return Err(contract_error(
            tool,
            &format!(
                "proto must declare exactly one service; found {}",
                services.len()
            ),
        ));
    }
    let service = &services[0];
    let service_name = service.full_name().to_string();

    let mut methods = Vec::new();
    for method in service.methods() {
        methods.push(GrpcMethod {
            name: method.name().to_string(),
            input: method.input().full_name().to_string(),
            output: method.output().full_name().to_string(),
            client_streaming: method.is_client_streaming(),
            server_streaming: method.is_server_streaming(),
        });
        if methods.len() > METHOD_MAX {
            return Err(contract_error(
                tool,
                &format!("service {service_name} exceeds the {METHOD_MAX}-method cap"),
            ));
        }
    }
    if methods.iter().all(|m| !m.unary()) {
        return Err(contract_error(
            tool,
            &format!(
                "service {service_name} declares no unary methods — the grpc virtual \
                 tool is unary-only in v1"
            ),
        ));
    }

    Ok(GrpcContract {
        proto: proto.to_string(),
        service_name,
        methods,
    })
}

/// Compile the source into a descriptor pool; a one-line summary of parse
/// failures (protox errors can be verbose).
pub(crate) fn compile_pool(
    tool: &ToolDef,
    proto: &str,
) -> Result<prost_reflect::DescriptorPool, SmahError> {
    let scratch = scratch_path();
    let dir = scratch
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| PathBuf::from("."));
    std::fs::write(&scratch, proto)
        .map_err(|e| contract_error(tool, &format!("proto scratch write failed: {e}")))?;
    let compiled = protox::compile([&scratch], [dir]);
    let _ = std::fs::remove_file(&scratch);
    let fds = compiled
        .map_err(|e| contract_error(tool, &format!("`interface:` proto failed to compile: {e}")))?;
    prost_reflect::DescriptorPool::from_file_descriptor_set(fds).map_err(|e| {
        contract_error(
            tool,
            &format!("`interface:` proto descriptors rejected: {e}"),
        )
    })
}

/// Unique scratch name for the compiler input (temp dir; removed after use).
fn scratch_path() -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    std::env::temp_dir().join(format!("smah-grpc-{}-{}.proto", std::process::id(), nanos))
}

/// Cache paths for one tool's frozen gRPC contract.
pub(crate) struct GrpcCache {
    pub(crate) proto: PathBuf,
    pub(crate) contract_sha: PathBuf,
    pub(crate) tool_sha: PathBuf,
}

pub(crate) fn cache(tool: &ToolDef) -> GrpcCache {
    let base = cache_dir()
        .join("virtual-tools")
        .join(&tool.name)
        .join("grpc");
    GrpcCache {
        proto: base.join("grpc.proto"),
        contract_sha: base.join("contract.sha256"),
        tool_sha: base.join("tool.sha256"),
    }
}

/// Resolve the `interface:` value to the raw contract bytes.
///
/// Inline proto is detected by content (`syntax =` together with a
/// `message`/`service` declaration); anything else is a path — `$VAR`
/// indirection resolves from the environment, other paths resolve relative
/// to the tool file's directory.
pub(crate) fn resolve_interface(tool: &ToolDef) -> Result<String, SmahError> {
    let raw = tool.interface.as_deref().unwrap_or_default();
    if raw.trim().is_empty() {
        return Err(contract_error(
            tool,
            "`interface:` is required — the grpc driver is interface-first; declare the \
             proto inline or as a path/$VAR reference",
        ));
    }
    let looks_inline =
        raw.contains("syntax") && (raw.contains("service ") || raw.contains("message "));
    if looks_inline {
        return Ok(raw.to_string());
    }
    // Path form.
    let path_text = if let Some(var) = raw.strip_prefix('$') {
        let name = var.split('/').next().unwrap_or(var);
        let val = std::env::var(name).map_err(|_| {
            contract_error(
                tool,
                &format!("`interface:` references undefined environment variable ${name}"),
            )
        })?;
        let tail = &raw[1 + name.len()..];
        format!("{val}{tail}")
    } else {
        raw.to_string()
    };
    let path = std::path::Path::new(&path_text);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        tool.path
            .parent()
            .map(|p| p.join(path))
            .unwrap_or_else(|| path.to_path_buf())
    };
    let bytes = std::fs::read(&path).map_err(|e| {
        contract_error(
            tool,
            &format!("`interface:` path {} unreadable: {e}", path.display()),
        )
    })?;
    if bytes.len() > CONTRACT_MAX {
        return Err(contract_error(
            tool,
            &format!(
                "`interface:` proto is {} bytes; the cap is {CONTRACT_MAX} bytes",
                bytes.len()
            ),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| contract_error(tool, "`interface:` proto file is not valid UTF-8"))
}

fn contract_error(tool: &ToolDef, detail: &str) -> SmahError {
    SmahError::ToolFileInvalid {
        path: tool.path.display().to_string(),
        detail: detail.to_string(),
    }
}

/// Load the frozen contract, re-freezing when stale/tampered/invalid.
/// Metadata-only audit: `grpc.contract`.
pub(crate) fn load_or_freeze(tool: &ToolDef, audit: &Audit) -> Result<GrpcContract, SmahError> {
    let cache = cache(tool);
    let tool_hash = sha256_hex(&std::fs::read(&tool.path).unwrap_or_default());

    if let (Ok(proto), Ok(stored_contract), Ok(stored_tool)) = (
        std::fs::read_to_string(&cache.proto),
        std::fs::read_to_string(&cache.contract_sha),
        std::fs::read_to_string(&cache.tool_sha),
    ) {
        let integrity_ok = stored_contract.trim() == sha256_hex(proto.as_bytes());
        let stale = stored_tool.trim() != tool_hash;
        if integrity_ok && !stale {
            match validate(tool, &proto) {
                Ok(contract) => {
                    audit_contract(audit, tool, "load", proto.len(), "allowed", "cache hit");
                    return Ok(contract);
                }
                Err(reason) => {
                    audit_contract(
                        audit,
                        tool,
                        "load",
                        proto.len(),
                        "denied",
                        &format!("cache failed validation: {}", err_line(&reason)),
                    );
                }
            }
        } else if !integrity_ok {
            audit_contract(
                audit,
                tool,
                "load",
                proto.len(),
                "denied",
                "cache integrity mismatch (contract.sha256)",
            );
        }
    }

    // (Re)freeze from the tool file's interface.
    let proto = resolve_interface(tool)?;
    let contract = validate(tool, &proto)?;
    std::fs::create_dir_all(cache.proto.parent().expect("cache path has a parent")).map_err(
        |e| {
            contract_error(
                tool,
                &format!(
                    "cache dir {} unusable: {e}",
                    cache
                        .proto
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
            )
        },
    )?;
    std::fs::write(&cache.proto, &contract.proto)
        .map_err(|e| contract_error(tool, &format!("freeze write failed: {e}")))?;
    std::fs::write(&cache.contract_sha, sha256_hex(contract.proto.as_bytes()))
        .map_err(|e| contract_error(tool, &format!("freeze sha write failed: {e}")))?;
    std::fs::write(&cache.tool_sha, tool_hash)
        .map_err(|e| contract_error(tool, &format!("freeze tool-sha write failed: {e}")))?;
    audit_contract(
        audit,
        tool,
        "freeze",
        contract.proto.len(),
        "allowed",
        "validated through protox compile",
    );
    Ok(contract)
}

fn err_line(err: &SmahError) -> String {
    // One line without the log suffix for audit reasons.
    err.to_string()
        .split(". Full log")
        .next()
        .unwrap_or_default()
        .to_string()
}

fn audit_contract(
    audit: &Audit,
    tool: &ToolDef,
    action: &str,
    bytes: usize,
    decision: &'static str,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: String::new(),
        tool: tool.name.clone(),
        op: format!("grpc.contract.{action}"),
        path: cache(tool).proto.display().to_string(),
        resolved: None,
        bytes: Some(bytes),
        decision,
        reason: reason.to_string(),
        turn: 0,
    };
    if let Err(reason) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

pub(crate) fn sha256_hex(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    format!("{:x}", h.finalize())
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn tool(interface: Option<&str>) -> ToolDef {
        ToolDef {
            name: "grpctest".to_string(),
            description: "t".to_string(),
            model: None,
            driver: "grpc".to_string(),
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            interface: interface.map(str::to_string),
            body: "persona".to_string(),
            path: PathBuf::from("grpctest.md"),
        }
    }

    const MINIMAL_PROTO: &str = r#"syntax = "proto3";
package demo.v1;
message PingRequest { string name = 1; }
message PingReply { string message = 1; }
service Greeter {
  rpc Ping(PingRequest) returns (PingReply);
  rpc Observe(PingRequest) returns (stream PingReply);
}
"#;

    #[test]
    fn missing_interface_is_a_hard_error() {
        let err = resolve_interface(&tool(None)).expect_err("must refuse");
        assert!(err.to_string().contains("interface-first"), "{err}");
        assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Config);
    }

    #[test]
    fn inline_proto_is_detected_by_content() {
        assert_eq!(
            resolve_interface(&tool(Some(MINIMAL_PROTO))).unwrap(),
            MINIMAL_PROTO
        );
        assert!(resolve_interface(&tool(Some("./svc.proto"))).is_err()); // file missing → path form
    }

    #[test]
    fn validate_accepts_minimal_proto_and_reports_methods() {
        let c = validate(&tool(Some(MINIMAL_PROTO)), MINIMAL_PROTO).expect("valid");
        assert_eq!(c.service_name, "demo.v1.Greeter");
        assert_eq!(c.methods.len(), 2);
        assert_eq!(c.methods[0].name, "Ping");
        assert!(c.methods[0].unary());
        assert_eq!(c.methods[0].input, "demo.v1.PingRequest");
        assert_eq!(c.methods[1].name, "Observe");
        assert!(!c.methods[1].unary());
        assert_eq!(c.unary_methods().len(), 1);
        assert!(c.method("Ping").is_some());
        assert!(c.method("Nope").is_none());
    }

    #[test]
    fn validate_rejects_unparsable_and_multi_service_protos() {
        let err =
            validate(&tool(Some("not proto at all")), "not proto at all").expect_err("unparsable");
        assert!(err.to_string().contains("compile"), "{err}");

        let two = format!("{MINIMAL_PROTO}\nservice Second {{ rpc P(demo.v1.PingRequest) returns (demo.v1.PingReply); }}");
        let err = validate(&tool(Some(&two)), &two).expect_err("two services");
        assert!(err.to_string().contains("exactly one service"), "{err}");
    }

    #[test]
    fn validate_rejects_streaming_only_services() {
        let only_stream = r#"syntax = "proto3";
package demo.v1;
message M { string a = 1; }
service S {
  rpc Watch(M) returns (stream M);
}
"#;
        let err = validate(&tool(Some(only_stream)), only_stream).expect_err("streaming only");
        assert!(err.to_string().contains("no unary methods"), "{err}");
    }

    #[test]
    fn validate_rejects_oversize() {
        let big = format!("syntax = \"proto3\";\n// {}", "x".repeat(CONTRACT_MAX + 1));
        assert!(validate(&tool(Some(&big)), &big).is_err());
    }
}
