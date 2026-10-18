//! Strict XML scanning for the SOAP driver (ticket #22).
//!
//! One job, security-first: accept only plain, well-formed, entity-free XML.
//! The scanner **rejects** (never resolves):
//!
//! - document type declarations — `Event::DocType` is an immediate refusal,
//!   which kills every XXE / billion-laughs vector at the parse step;
//! - general entity references other than the five predefined (`amp`, `lt`,
//!   `gt`, `quot`, `apos`) and numeric character references — without a DTD
//!   they are malformed anyway, and refusing keeps custom-entity smuggling
//!   out of the fabrication loop;
//! - processing instructions other than the XML declaration;
//! - documents above the caller-enforced size caps.
//!
//! Namespace handling is real but minimal: prefix bindings are collected
//! into a stack so element namespaces can be reported as resolved URIs (the
//! envelope check and the WSDL walk both rely on it). Attribute values are
//! scanned raw — nothing is unescaped, so nothing expands.

use quick_xml::events::Event;
use quick_xml::Reader;

/// One scanned element (document order).
#[derive(Debug, Clone)]
pub(crate) struct ElementInfo {
    /// Local (prefix-stripped) name.
    pub(crate) local: String,
    /// Resolved namespace URI, when the element sits in one.
    pub(crate) ns: Option<String>,
    /// Raw attribute (name, value) pairs, document order. Names keep their
    /// prefix as written (`xmlns:tns` stays `xmlns:tns`).
    pub(crate) attrs: Vec<(String, String)>,
}

impl ElementInfo {
    pub(crate) fn attr(&self, local: &str) -> Option<&str> {
        self.attrs
            .iter()
            .find(|(name, _)| name == local || name.rsplit(':').next() == Some(local))
            .map(|(_, v)| v.as_str())
    }
}

/// A scanned document: root details plus every element in order.
#[derive(Debug, Clone)]
pub(crate) struct XmlDocument {
    pub(crate) root_local: String,
    pub(crate) root_ns: Option<String>,
    pub(crate) root_attrs: Vec<(String, String)>,
    pub(crate) elements: Vec<ElementInfo>,
}

impl XmlDocument {
    pub(crate) fn root_attribute(&self, name: &str) -> Option<String> {
        self.root_attrs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
    }

    pub(crate) fn has_element(&self, local: &str, ns: &str) -> bool {
        self.elements
            .iter()
            .any(|el| el.local == local && el.ns.as_deref() == Some(ns))
    }
}

/// Scan `text` under the strict rules; returns the parsed outline or the
/// reason the document is refused.
pub(crate) fn strict_scan(text: &str) -> Result<XmlDocument, String> {
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = true;
    reader.config_mut().expand_empty_elements = true;

    // prefix -> uri bindings; each element scope pushes/pops.
    type RootInfo = Option<(String, Option<String>, Vec<(String, String)>)>;
    let mut scopes: Vec<Vec<(String, String)>> = Vec::new();
    let mut default_ns: Vec<Option<String>> = vec![None];
    let mut elements: Vec<ElementInfo> = Vec::new();
    let mut root: RootInfo = None;
    let mut open: usize = 0;

    loop {
        let event = reader
            .read_event()
            .map_err(|e| format!("malformed XML: {e}"))?;
        match event {
            Event::Start(start) => {
                let mut bindings: Vec<(String, String)> = Vec::new();
                let mut new_default: Option<Option<String>> = None;
                let mut attrs: Vec<(String, String)> = Vec::new();
                for attr in start.attributes() {
                    let attr = attr.map_err(|e| format!("malformed attribute: {e}"))?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = String::from_utf8_lossy(&attr.value).into_owned();
                    if key == "xmlns" {
                        new_default = Some(Some(value));
                    } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                        bindings.push((prefix.to_string(), value));
                    } else {
                        attrs.push((key, value));
                    }
                }
                if let Some(d) = new_default {
                    default_ns.push(d);
                } else {
                    // Keep the stack aligned with every start/end pair.
                    default_ns.push(default_ns.last().cloned().flatten());
                }
                scopes.push(bindings);

                let qname = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let (prefix, local) = split_qname(&qname);
                let ns = resolve_ns(prefix, &scopes, &default_ns);
                if root.is_none() {
                    root = Some((local.clone(), ns.clone(), attrs.clone()));
                }
                open += 1;
                elements.push(ElementInfo { local, ns, attrs });
            }
            Event::Empty(start) => {
                // Self-closing: attributes bind for this element only.
                let mut bindings: Vec<(String, String)> = Vec::new();
                let mut self_default: Option<Option<String>> = None;
                let mut attrs: Vec<(String, String)> = Vec::new();
                for attr in start.attributes() {
                    let attr = attr.map_err(|e| format!("malformed attribute: {e}"))?;
                    let key = String::from_utf8_lossy(attr.key.as_ref()).into_owned();
                    let value = String::from_utf8_lossy(&attr.value).into_owned();
                    if key == "xmlns" {
                        self_default = Some(Some(value));
                    } else if let Some(prefix) = key.strip_prefix("xmlns:") {
                        bindings.push((prefix.to_string(), value));
                    } else {
                        attrs.push((key, value));
                    }
                }
                scopes.push(bindings);
                if let Some(d) = self_default {
                    default_ns.push(d);
                } else {
                    default_ns.push(default_ns.last().cloned().flatten());
                }
                let qname = String::from_utf8_lossy(start.name().as_ref()).into_owned();
                let (prefix, local) = split_qname(&qname);
                let ns = resolve_ns(prefix, &scopes, &default_ns);
                if root.is_none() {
                    root = Some((local.clone(), ns.clone(), attrs.clone()));
                }
                elements.push(ElementInfo { local, ns, attrs });
                default_ns.pop();
                scopes.pop();
            }
            Event::End(_) => {
                open = open
                    .checked_sub(1)
                    .ok_or_else(|| "malformed XML: unbalanced end tag".to_string())?;
                default_ns.pop();
                scopes.pop();
            }
            Event::Text(_) | Event::CData(_) | Event::Comment(_) => {}
            Event::Decl(_) => {}
            Event::DocType(_) => {
                return Err("document type declarations are refused (DTD/entity policy)".to_string())
            }
            Event::PI(_) => return Err("processing instructions are refused".to_string()),
            Event::GeneralRef(reference) => {
                let raw = String::from_utf8_lossy(reference.as_ref()).into_owned();
                if !allowed_reference(&raw) {
                    return Err(format!(
                        "entity reference {raw:?} is refused (predefined entities and \
                         character references only)"
                    ));
                }
            }
            Event::Eof => break,
        }
    }

    if open != 0 {
        return Err("malformed XML: unclosed elements".to_string());
    }
    let (root_local, root_ns, root_attrs) = root.ok_or_else(|| "empty document".to_string())?;
    Ok(XmlDocument {
        root_local,
        root_ns,
        root_attrs,
        elements,
    })
}

/// The five predefined entities plus numeric character references.
fn allowed_reference(raw: &str) -> bool {
    let name = raw
        .strip_prefix('&')
        .and_then(|r| r.strip_suffix(';'))
        .unwrap_or(raw);
    if matches!(name, "amp" | "lt" | "gt" | "quot" | "apos") {
        return true;
    }
    if let Some(num) = name.strip_prefix('#') {
        let digits = num
            .strip_prefix('x')
            .or_else(|| num.strip_prefix('X'))
            .map(|h| (h, 16))
            .unwrap_or((num, 10));
        return !digits.0.is_empty()
            && digits
                .0
                .chars()
                .all(|c| c.is_ascii_digit() || (digits.1 == 16 && c.is_ascii_hexdigit()));
    }
    false
}

fn split_qname(qname: &str) -> (Option<&str>, String) {
    match qname.split_once(':') {
        Some((prefix, local)) => (Some(prefix), local.to_string()),
        None => (None, qname.to_string()),
    }
}

fn resolve_ns(
    prefix: Option<&str>,
    scopes: &[Vec<(String, String)>],
    default_ns: &[Option<String>],
) -> Option<String> {
    match prefix {
        Some(prefix) => scopes
            .iter()
            .rev()
            .flatten()
            .find(|(p, _)| p == prefix)
            .map(|(_, uri)| uri.clone()),
        None => default_ns.last().cloned().flatten(),
    }
}

/// The SOAP 1.1 / 1.2 envelope namespaces a request may carry.
pub(crate) const ENVELOPE_NS: [&str; 2] = [
    "http://schemas.xmlsoap.org/soap/envelope/",
    "http://www.w3.org/2003/05/soap-envelope",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_namespaces_and_attributes() {
        let doc = strict_scan(
            r#"<wsdl:definitions xmlns:wsdl="http://schemas.xmlsoap.org/wsdl/" targetNamespace="urn:x">
                 <wsdl:service name="s"><soap:address xmlns:soap="http://x" location="loopback"/></wsdl:service>
                 <plain/>
               </wsdl:definitions>"#,
        )
        .expect("scan");
        assert_eq!(doc.root_local, "definitions");
        assert_eq!(
            doc.root_ns.as_deref(),
            Some("http://schemas.xmlsoap.org/wsdl/")
        );
        assert_eq!(doc.root_attribute("targetNamespace"), Some("urn:x".into()));
        let svc = doc
            .elements
            .iter()
            .find(|el| el.local == "service")
            .expect("service");
        assert_eq!(svc.attr("name"), Some("s"));
        let addr = doc
            .elements
            .iter()
            .find(|el| el.local == "address")
            .expect("address");
        assert_eq!(addr.ns.as_deref(), Some("http://x"));
        assert_eq!(addr.attr("location"), Some("loopback"));
        let plain = doc
            .elements
            .iter()
            .find(|el| el.local == "plain")
            .expect("plain");
        assert!(plain.ns.is_none());
    }

    #[test]
    fn rejects_doctype_entities_and_pis() {
        assert!(
            strict_scan("<!DOCTYPE a [<!ENTITY x SYSTEM 'file:///etc/passwd'>]><a>&x;</a>")
                .is_err()
        );
        assert!(strict_scan("<a>&custom;</a>").is_err());
        assert!(strict_scan("<a><?php ?></a>").is_err());
        assert!(strict_scan("<a></b>").is_err());
    }

    #[test]
    fn allows_predefined_entities_and_char_refs() {
        assert!(strict_scan("<a>5 &lt; 6 &amp; 7 &#65; &#x42;</a>").is_ok());
    }

    #[test]
    fn empty_and_unclosed_documents_are_refused() {
        assert!(strict_scan("").is_err());
        assert!(strict_scan("<a>").is_err());
    }
}
