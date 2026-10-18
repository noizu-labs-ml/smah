//! SOAP contract resolution, validation, and the freeze cache (ticket #22).
//!
//! Interface-first is the product: the tool file's `interface:` declares the
//! WSDL (inline XML or a path/`$VAR` reference). The driver never invents a
//! contract — a missing or invalid one is a hard `ToolFileInvalid` error
//! (exit 3). A validated contract is frozen under
//! `~/.cache/smah/virtual-tools/<tool>/soap/` with two pins:
//!
//! - `contract.sha256` — integrity of the frozen WSDL bytes (a tampered or
//!   truncated cache is detected and re-frozen from the tool file);
//! - `tool.sha256` — staleness marker (a changed tool file re-freezes).
//!
//! On every load the frozen WSDL is re-validated ("validate-on-load"), so a
//! cache that passes integrity but not validation is also re-frozen.

use std::collections::BTreeMap;
use std::path::PathBuf;

use sha2::{Digest, Sha256};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{cache_dir, Audit, AuditRecord};
use crate::virtual_tools::loader::ToolDef;

/// Cap on the contract artifact size (inline text or referenced file).
pub(crate) const CONTRACT_MAX: usize = 1024 * 1024;

/// One SOAP operation extracted from the frozen WSDL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SoapOperation {
    /// `operation/@name`.
    pub(crate) name: String,
    /// Local name of the output part's `element` when resolvable from the
    /// WSDL (the response root the driver expects the model to produce).
    /// `None` = structural-only validation for this operation.
    pub(crate) response_element: Option<String>,
}

/// The frozen, validated contract.
#[derive(Debug, Clone)]
pub(crate) struct SoapContract {
    /// The WSDL bytes served verbatim at the discovery URL.
    pub(crate) wsdl: String,
    /// WSDL `definitions/@targetNamespace`.
    pub(crate) target_namespace: String,
    /// Operations declared in the WSDL (portType + binding, deduped).
    pub(crate) operations: Vec<SoapOperation>,
}

impl SoapContract {
    pub(crate) fn operation(&self, name: &str) -> Option<&SoapOperation> {
        self.operations.iter().find(|op| op.name == name)
    }
}

/// Cache paths for one tool's frozen SOAP contract.
pub(crate) struct SoapCache {
    pub(crate) wsdl: PathBuf,
    pub(crate) contract_sha: PathBuf,
    pub(crate) tool_sha: PathBuf,
}

pub(crate) fn cache(tool: &ToolDef) -> SoapCache {
    let base = cache_dir()
        .join("virtual-tools")
        .join(&tool.name)
        .join("soap");
    SoapCache {
        wsdl: base.join("wsdl.xml"),
        contract_sha: base.join("contract.sha256"),
        tool_sha: base.join("tool.sha256"),
    }
}

/// Resolve the `interface:` value to the raw contract bytes.
///
/// Inline WSDL is detected by content (`<definitions` / `<wsdl:definitions`
/// anywhere in the value); anything else is a path — `$VAR` indirection is
/// resolved from the environment, other paths resolve relative to the tool
/// file's directory.
pub(crate) fn resolve_interface(tool: &ToolDef) -> Result<String, SmahError> {
    let raw = tool.interface.as_deref().unwrap_or_default();
    if raw.trim().is_empty() {
        return Err(contract_error(
            tool,
            "`interface:` is required — the soap driver is interface-first; declare the WSDL \
             inline or as a path/$VAR reference",
        ));
    }
    if raw.contains("<definitions") || raw.contains("<wsdl:definitions") {
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
        // `$VAR/tail` splicing.
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
                "`interface:` WSDL is {} bytes; the cap is {CONTRACT_MAX} bytes",
                bytes.len()
            ),
        ));
    }
    String::from_utf8(bytes)
        .map_err(|_| contract_error(tool, "`interface:` WSDL file is not valid UTF-8"))
}

fn contract_error(tool: &ToolDef, detail: &str) -> SmahError {
    SmahError::ToolFileInvalid {
        path: tool.path.display().to_string(),
        detail: detail.to_string(),
    }
}

/// Validate WSDL text into a [`SoapContract`] (strict XML, no DTD/entities —
/// a contract carrying them is rejected like any inbound request).
pub(crate) fn validate(tool: &ToolDef, wsdl: &str) -> Result<SoapContract, SmahError> {
    if wsdl.len() > CONTRACT_MAX {
        return Err(contract_error(
            tool,
            &format!("WSDL exceeds the {CONTRACT_MAX}-byte cap"),
        ));
    }
    let doc = super::xml::strict_scan(wsdl).map_err(|e| {
        contract_error(
            tool,
            &format!("`interface:` WSDL rejected by strict XML scan: {e}"),
        )
    })?;
    if !doc.has_element("definitions", "http://schemas.xmlsoap.org/wsdl/") {
        return Err(contract_error(
            tool,
            "WSDL must contain a <definitions> root in the WSDL namespace",
        ));
    }
    let target_namespace = doc.root_attribute("targetNamespace").unwrap_or_default();
    if target_namespace.trim().is_empty() {
        return Err(contract_error(
            tool,
            "WSDL <definitions> must declare a non-empty targetNamespace",
        ));
    }
    if !doc.has_element("service", "http://schemas.xmlsoap.org/wsdl/") {
        return Err(contract_error(
            tool,
            "WSDL must declare a <service> element",
        ));
    }

    // Operation names from portType + binding levels, deduped, order-stable.
    let mut names: Vec<String> = Vec::new();
    for el in &doc.elements {
        if el.local != "operation" || el.ns.as_deref() != Some("http://schemas.xmlsoap.org/wsdl/") {
            continue;
        }
        if let Some(name) = el.attr("name") {
            if !names.iter().any(|n| n == name) {
                names.push(name.to_string());
            }
        }
    }
    if names.is_empty() {
        return Err(contract_error(
            tool,
            "WSDL declares no <operation> elements",
        ));
    }

    // Response-element resolution: operation (portType) -> output message ->
    // part/@element local name. Best-effort: unresolvable = None.
    type PartElements = BTreeMap<String, Vec<(Option<String>, Option<String>)>>;
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    let mut output_message: BTreeMap<String, String> = BTreeMap::new(); // op -> message QName
    let mut messages: PartElements = BTreeMap::new(); // msg name -> part element QNames
    let mut current_port_type_op: Option<String> = None;
    let mut current_message: Option<String> = None;
    for el in &doc.elements {
        match el.local.as_str() {
            "operation" => {
                current_port_type_op = el.attr("name").map(str::to_string);
            }
            "output" => {
                if let (Some(op), Some(msg)) = (current_port_type_op.as_ref(), el.attr("message")) {
                    output_message.insert(op.clone(), msg.to_string());
                }
            }
            "message" => {
                current_message = el.attr("name").map(str::to_string);
            }
            "part" => {
                if let Some(msg) = current_message.as_ref() {
                    messages
                        .entry(msg.clone())
                        .or_default()
                        .push((el.attr("element").map(str::to_string), None));
                }
            }
            _ => {}
        }
    }
    for (op, msg_qname) in &output_message {
        let local = local_qname(msg_qname);
        if let Some(parts) = messages.get(local) {
            if let Some((Some(element_qname), _)) = parts.first() {
                map.insert(op.clone(), local_qname(element_qname).to_string());
            }
        }
    }

    Ok(SoapContract {
        wsdl: wsdl.to_string(),
        target_namespace,
        operations: names
            .into_iter()
            .map(|name| SoapOperation {
                response_element: map.get(&name).cloned(),
                name,
            })
            .collect(),
    })
}

fn local_qname(qname: &str) -> &str {
    qname.rsplit(':').next().unwrap_or(qname)
}

/// Load the frozen contract, re-freezing when stale/tampered/invalid.
/// Metadata-only audit: `soap.contract`.
pub(crate) fn load_or_freeze(tool: &ToolDef, audit: &Audit) -> Result<SoapContract, SmahError> {
    let cache = cache(tool);
    let tool_hash = sha256_hex(&std::fs::read(&tool.path).unwrap_or_default());

    if let (Ok(wsdl), Ok(stored_contract), Ok(stored_tool)) = (
        std::fs::read_to_string(&cache.wsdl),
        std::fs::read_to_string(&cache.contract_sha),
        std::fs::read_to_string(&cache.tool_sha),
    ) {
        let integrity_ok = stored_contract.trim() == sha256_hex(wsdl.as_bytes());
        let stale = stored_tool.trim() != tool_hash;
        if integrity_ok && !stale {
            match validate(tool, &wsdl) {
                Ok(contract) => {
                    audit_contract(audit, tool, "load", wsdl.len(), "allowed", "cache hit");
                    return Ok(contract);
                }
                Err(reason) => {
                    audit_contract(
                        audit,
                        tool,
                        "load",
                        wsdl.len(),
                        "denied",
                        &format!("cache failed validation: {reason}"),
                    );
                }
            }
        } else if !integrity_ok {
            audit_contract(
                audit,
                tool,
                "load",
                wsdl.len(),
                "denied",
                "cache integrity mismatch (contract.sha256)",
            );
        }
    }

    // (Re)freeze from the tool file's interface.
    let wsdl = resolve_interface(tool)?;
    let contract = validate(tool, &wsdl)?;
    std::fs::create_dir_all(cache.wsdl.parent().expect("cache path has a parent")).map_err(
        |e| {
            contract_error(
                tool,
                &format!(
                    "cache dir {} unusable: {e}",
                    cache
                        .wsdl
                        .parent()
                        .map(|p| p.display().to_string())
                        .unwrap_or_default()
                ),
            )
        },
    )?;
    std::fs::write(&cache.wsdl, &contract.wsdl)
        .map_err(|e| contract_error(tool, &format!("freeze write failed: {e}")))?;
    std::fs::write(&cache.contract_sha, sha256_hex(contract.wsdl.as_bytes()))
        .map_err(|e| contract_error(tool, &format!("freeze sha write failed: {e}")))?;
    std::fs::write(&cache.tool_sha, tool_hash)
        .map_err(|e| contract_error(tool, &format!("freeze tool-sha write failed: {e}")))?;
    audit_contract(
        audit,
        tool,
        "freeze",
        contract.wsdl.len(),
        "allowed",
        "validated against strict XML scan",
    );
    Ok(contract)
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
        op: format!("soap.contract.{action}"),
        path: cache(tool).wsdl.display().to_string(),
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
            name: "soaptest".to_string(),
            description: "t".to_string(),
            model: None,
            driver: "soap".to_string(),
            constraints: Vec::new(),
            allow_roots: Vec::new(),
            interface: interface.map(str::to_string),
            body: "persona".to_string(),
            path: PathBuf::from("soaptest.md"),
        }
    }

    const MINIMAL_WSDL: &str = r#"<?xml version="1.0"?>
<wsdl:definitions xmlns:wsdl="http://schemas.xmlsoap.org/wsdl/"
    xmlns:tns="urn:soaptest"
    targetNamespace="urn:soaptest">
  <wsdl:message name="pingIn"><wsdl:part name="body" element="tns:ping"/></wsdl:message>
  <wsdl:message name="pingOut"><wsdl:part name="body" element="tns:pingResponse"/></wsdl:message>
  <wsdl:portType name="pt">
    <wsdl:operation name="Ping">
      <wsdl:input message="tns:pingIn"/>
      <wsdl:output message="tns:pingOut"/>
    </wsdl:operation>
  </wsdl:portType>
  <wsdl:binding name="b" type="tns:pt">
    <wsdl:operation name="Ping"/>
  </wsdl:binding>
  <wsdl:service name="svc"><wsdl:port name="p" binding="tns:b"/></wsdl:service>
</wsdl:definitions>"#;

    #[test]
    fn missing_interface_is_a_hard_error() {
        let err = resolve_interface(&tool(None)).expect_err("must refuse");
        assert!(err.to_string().contains("interface-first"), "{err}");
        assert_eq!(err.exit_code(), crate::exit_codes::ExitCode::Config);
    }

    #[test]
    fn inline_wsdl_is_detected_by_content() {
        assert_eq!(
            resolve_interface(&tool(Some(MINIMAL_WSDL))).unwrap(),
            MINIMAL_WSDL
        );
    }

    #[test]
    fn validate_accepts_minimal_wsdl_and_resolves_response_element() {
        let c = validate(&tool(Some(MINIMAL_WSDL)), MINIMAL_WSDL).expect("valid");
        assert_eq!(c.target_namespace, "urn:soaptest");
        assert_eq!(c.operations.len(), 1);
        assert_eq!(c.operations[0].name, "Ping");
        assert_eq!(
            c.operations[0].response_element.as_deref(),
            Some("pingResponse")
        );
    }

    #[test]
    fn validate_rejects_non_wsdl_and_missing_service() {
        let err = validate(&tool(Some("<html/>")), "<html/>").expect_err("not wsdl");
        assert!(err.to_string().contains("definitions"), "{err}");
        let no_service = MINIMAL_WSDL.replace(
            "<wsdl:service name=\"svc\"><wsdl:port name=\"p\" binding=\"tns:b\"/></wsdl:service>",
            "",
        );
        assert!(validate(&tool(Some(&no_service)), &no_service).is_err());
    }

    #[test]
    fn validate_rejects_wsdl_carrying_a_dtd() {
        let xxe = "<?xml version=\"1.0\"?><!DOCTYPE definitions [<!ENTITY xxe SYSTEM \"file:///etc/passwd\">]><wsdl:definitions xmlns:wsdl=\"http://schemas.xmlsoap.org/wsdl/\" targetNamespace=\"urn:x\"/>".to_string();
        assert!(validate(&tool(Some(&xxe)), &xxe).is_err());
    }
}
