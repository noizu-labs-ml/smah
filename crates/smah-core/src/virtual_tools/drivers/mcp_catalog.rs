//! MCP tool-catalog fabrication, freezing, and the call memo (ticket #23).
//!
//! The tool discovery surface is **fabricated**: one bounded model turn
//! invents the MCP tool catalog (`[{name, description, inputSchema}, …]`)
//! for the declared persona/`interface:`; this module validates it against
//! hard caps, freezes it to `~/.cache/smah/virtual-tools/<tool>/mcp-tools.json`
//! keyed by a tool-file digest marker (`mcp.sha256`), and hands the frozen
//! catalog to `tools/list`. `--show-catalog` exists so a human can review
//! exactly what the model invented before any client sees it (T-027).
//!
//! `tools/call` results are memoized in `mcp-memo.jsonl` keyed by
//! `sha256(tool name + canonical arguments)`: identical calls are answered
//! from the memo without another model turn.

use std::path::Path;

use serde_json::Value as Json;

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::drivers::ChatMessage;
use crate::virtual_tools::loader::ToolDef;

use super::support;

/// Catalog caps (T-027).
pub(crate) const MAX_TOOLS: usize = 64;
pub(crate) const NAME_MAX: usize = 64;
pub(crate) const DESCRIPTION_MAX: usize = 1024;
pub(crate) const SCHEMA_BYTES_MAX: usize = 16 * 1024;
pub(crate) const CATALOG_BYTES_MAX: usize = 256 * 1024;
/// A memoized tool result over this is refused (fabrication caps).
pub(crate) const CALL_TEXT_MAX: usize = 64 * 1024;

/// One frozen MCP tool.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct CatalogEntry {
    pub name: String,
    pub description: String,
    pub input_schema: Json,
}

impl CatalogEntry {
    /// The wire shape for `tools/list`.
    pub(crate) fn to_wire(&self) -> Json {
        serde_json::json!({
            "name": self.name,
            "description": self.description,
            "inputSchema": self.input_schema,
        })
    }
}

/// MCP tool names: letter first, then letters/digits/`_`/`-`, 1..=64.
pub(crate) fn valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    name.len() <= NAME_MAX && chars.all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Parse + validate a catalog array. `Err` is a one-line rejection fed back
/// for the single retry.
pub(crate) fn parse_catalog(root: &Json) -> Result<Vec<CatalogEntry>, String> {
    let items = root
        .as_array()
        .ok_or("catalog must be a JSON array of tool objects")?;
    if items.is_empty() {
        return Err("catalog is empty — declare at least one tool".to_string());
    }
    if items.len() > MAX_TOOLS {
        return Err(format!(
            "catalog declares {} tools (max {MAX_TOOLS})",
            items.len()
        ));
    }
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let name = item
            .get("name")
            .and_then(Json::as_str)
            .ok_or("each catalog entry needs a string 'name'")?;
        if !valid_name(name) {
            return Err(format!(
                "tool name '{name}' is invalid (letter first, then letters/digits/'_'/'-', max {NAME_MAX})"
            ));
        }
        if out.iter().any(|e: &CatalogEntry| e.name == name) {
            return Err(format!("duplicate tool name '{name}'"));
        }
        let description = item
            .get("description")
            .and_then(Json::as_str)
            .ok_or(format!("tool '{name}' needs a string 'description'"))?;
        if description.trim().is_empty() {
            return Err(format!("tool '{name}' description must be non-empty"));
        }
        if description.len() > DESCRIPTION_MAX {
            return Err(format!(
                "tool '{name}' description is {} bytes (max {DESCRIPTION_MAX})",
                description.len()
            ));
        }
        let schema = item
            .get("inputSchema")
            .cloned()
            .unwrap_or_else(|| Json::Object(serde_json::Map::new()));
        if !schema.is_object() {
            return Err(format!("tool '{name}' inputSchema must be a JSON object"));
        }
        if schema.to_string().len() > SCHEMA_BYTES_MAX {
            return Err(format!(
                "tool '{name}' inputSchema is over the {SCHEMA_BYTES_MAX} byte cap"
            ));
        }
        out.push(CatalogEntry {
            name: name.to_string(),
            description: description.to_string(),
            input_schema: schema,
        });
    }
    let total = serde_json::to_string(&out.iter().map(CatalogEntry::to_wire).collect::<Vec<_>>())
        .map_err(|err| format!("catalog serialization: {err}"))?
        .len();
    if total > CATALOG_BYTES_MAX {
        return Err(format!(
            "catalog is {total} bytes total (max {CATALOG_BYTES_MAX})"
        ));
    }
    Ok(out)
}

/// Extract the JSON array from a model reply (tolerates fences and prose).
pub(crate) fn extract_json_array(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let start = trimmed.find('[')?;
    let end = trimmed.rfind(']')? + 1;
    if end <= start || start > CATALOG_BYTES_MAX {
        return None;
    }
    Some(&trimmed[start..end])
}

/// Frozen catalog path: `<cache leaf>/mcp-tools.json`.
pub(crate) fn catalog_path(dir: &Path) -> std::path::PathBuf {
    dir.join("mcp-tools.json")
}

/// Marker path: `<cache leaf>/mcp.sha256`.
pub(crate) fn marker_path(dir: &Path) -> std::path::PathBuf {
    dir.join(support::MARKER_MCP)
}

/// Load the frozen catalog if the marker matches `digest` and it parses.
pub(crate) fn load_frozen(dir: &Path, digest: &str) -> Option<Vec<CatalogEntry>> {
    if std::fs::read_to_string(marker_path(dir)).ok()?.trim() != digest {
        return None;
    }
    let raw = std::fs::read_to_string(catalog_path(dir)).ok()?;
    if raw.len() > CATALOG_BYTES_MAX {
        return None;
    }
    let parsed: Json = serde_json::from_str(&raw).ok()?;
    parse_catalog(&parsed).ok()
}

/// Freeze a validated catalog + its tool-file digest marker.
fn freeze(dir: &Path, entries: &[CatalogEntry], digest: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cache dir: {}", support::one_line(&err.to_string())))?;
    let wire: Vec<Json> = entries.iter().map(CatalogEntry::to_wire).collect();
    let text = serde_json::to_string_pretty(&wire).map_err(|err| format!("serialize: {err}"))?;
    std::fs::write(catalog_path(dir), text)
        .map_err(|err| format!("catalog write: {}", support::one_line(&err.to_string())))?;
    std::fs::write(marker_path(dir), digest)
        .map_err(|err| format!("marker write: {}", support::one_line(&err.to_string())))?;
    Ok(())
}

pub(crate) const FABRICATE_SYSTEM: &str = concat!(
    "You fabricate the MCP tool catalog of a VIRTUAL MCP server for a smah virtual tool.\n",
    "Output ONLY one JSON array, no markdown fences, no prose:\n",
    "[{\"name\":\"<Tool_Name>\",\"description\":\"<one paragraph>\",\"inputSchema\":{\"type\":\"object\",\"properties\":{...},\"required\":[...]}}]\n",
    "Names start with a letter and use only letters, digits, '_' and '-' (max 64 chars).\n",
    "inputSchema is a JSON-Schema object describing the tool's arguments.\n",
    "Keep the catalog small and coherent (a handful of tools that fit the persona).\n",
    "Do not output any other JSON or explanation."
);

fn fabricate_user_prompt(tool: &ToolDef) -> String {
    let mut out = String::new();
    out.push_str("VIRTUAL MCP SERVER PERSONA:\n");
    out.push_str(&tool.body);
    if !tool.constraints.is_empty() {
        out.push_str("\n\nHard constraints:\n");
        for constraint in &tool.constraints {
            out.push_str(&format!("- {constraint}\n"));
        }
    }
    out.push_str("\nContract artifact (interface):\n");
    match &tool.interface {
        Some(interface) if interface.len() <= CATALOG_BYTES_MAX => out.push_str(interface),
        Some(_) => out.push_str(
            "(interface artifact over the size cap — invent a coherent tool set fitting the persona)",
        ),
        None => out.push_str(
            "(none declared — invent a coherent MCP tool set fitting the persona)",
        ),
    }
    out.push_str("\n\nFabricate the MCP tool catalog now. Output only the JSON array.");
    out
}

/// One fabrication attempt: extract → parse → validate → freeze.
fn attempt(reply: &str, dir: &Path, digest: &str) -> Result<usize, String> {
    let text = extract_json_array(reply).ok_or("reply contained no JSON array")?;
    let parsed: Json = serde_json::from_str(text).map_err(|err| {
        format!(
            "catalog JSON did not parse: {}",
            support::one_line(&err.to_string())
        )
    })?;
    let entries = parse_catalog(&parsed)?;
    freeze(dir, &entries, digest)?;
    Ok(entries.len())
}

/// Ensure the frozen catalog exists and matches the current tool file.
/// Returns the entries and the model turns consumed. Two invalid catalogs
/// abort with [`SmahError::DirectiveLoopOverflow`] (exit 5).
pub(crate) fn ensure_catalog(
    tool: &ToolDef,
    backend: &dyn crate::virtual_tools::drivers::ChatBackend,
    model: &str,
    audit: &Audit,
    run_id: &str,
    dir: &Path,
    force: bool,
) -> Result<(Vec<CatalogEntry>, usize), SmahError> {
    let (digest, tool_len) =
        support::sha256_file(&tool.path).map_err(|err| SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!("unreadable while keying the cache: {err}"),
        })?;
    if !force {
        if let Some(entries) = load_frozen(dir, &digest) {
            support::audit_server_op(
                audit,
                run_id,
                &tool.name,
                "mcp.catalog",
                "reuse",
                Some(tool_len as usize),
                "allowed",
                "",
            );
            return Ok((entries, 0));
        }
    }
    let mut turns = 0usize;
    let mut prompt = fabricate_user_prompt(tool);
    loop {
        if turns >= crate::virtual_tools::TURNS_MAX {
            return Err(SmahError::DirectiveLoopOverflow {
                tool: tool.name.clone(),
                turns,
            });
        }
        turns += 1;
        let reply = backend.stream_turn(
            model,
            FABRICATE_SYSTEM,
            &[ChatMessage::user(&prompt)],
            &mut |_| {},
        )?;
        match attempt(&reply, dir, &digest) {
            Ok(_count) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "mcp.catalog",
                    "generate",
                    Some(reply.len()),
                    "allowed",
                    "",
                );
                if let Some(entries) = load_frozen(dir, &digest) {
                    return Ok((entries, turns));
                }
                return Err(SmahError::ToolFileInvalid {
                    path: catalog_path(dir).display().to_string(),
                    detail: "frozen catalog failed to reload after generation".to_string(),
                });
            }
            Err(reason) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "mcp.catalog",
                    "generate",
                    Some(reply.len()),
                    "denied",
                    &reason,
                );
                if turns >= 2 {
                    return Err(SmahError::DirectiveLoopOverflow {
                        tool: tool.name.clone(),
                        turns,
                    });
                }
                prompt = format!(
                    "{}\n\nYour previous catalog was rejected: {reason}\n\
                     Fix it and output only the corrected JSON array.",
                    fabricate_user_prompt(tool)
                );
            }
        }
    }
}

// --- the tools/call memo ----------------------------------------------------

/// Memo path: `<cache leaf>/mcp-memo.jsonl`.
pub(crate) fn memo_path(dir: &Path) -> std::path::PathBuf {
    dir.join("mcp-memo.jsonl")
}

/// The memo key: `sha256(tool name \u{0} canonical arguments JSON)`.
pub(crate) fn memo_key(name: &str, arguments: &Json) -> String {
    let canonical = serde_json::to_string(arguments).unwrap_or_default();
    support::sha256_hex(format!("{name}\u{0}{canonical}").as_bytes())
}

/// Look up a memoized result (last entry wins).
pub(crate) fn memo_get(dir: &Path, key: &str) -> Option<String> {
    let raw = std::fs::read_to_string(memo_path(dir)).ok()?;
    for line in raw.lines().rev() {
        if let Ok(value) = serde_json::from_str::<Json>(line) {
            if value.get("key").and_then(Json::as_str) == Some(key) {
                return value.get("text").and_then(Json::as_str).map(str::to_string);
            }
        }
    }
    None
}

/// Append a memo entry; compacts to the newest 256 entries past 4 MiB.
pub(crate) fn memo_put(dir: &Path, key: &str, name: &str, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(dir)
        .map_err(|err| format!("cache dir: {}", support::one_line(&err.to_string())))?;
    let path = memo_path(dir);
    let entry = serde_json::json!({"key": key, "name": name, "text": text}).to_string();
    if path.is_file() && std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0) > 4 * 1024 * 1024 {
        let raw = std::fs::read_to_string(&path).unwrap_or_default();
        let kept: Vec<&str> = raw.lines().rev().take(256).collect();
        let mut compacted = kept.into_iter().rev().collect::<Vec<_>>().join("\n");
        compacted.push('\n');
        std::fs::write(&path, compacted)
            .map_err(|err| format!("memo compact: {}", support::one_line(&err.to_string())))?;
    }
    use std::io::Write;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .and_then(|mut file| writeln!(file, "{entry}"))
        .map_err(|err| format!("memo append: {}", support::one_line(&err.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str) -> Json {
        json!({
            "name": name,
            "description": "does a thing",
            "inputSchema": {"type": "object", "properties": {"q": {"type": "string"}}}
        })
    }

    #[test]
    fn valid_name_enforces_the_shape() {
        for good in ["search", "Search_Users", "get-item-2", "A"] {
            assert!(valid_name(good), "{good}");
        }
        for bad in [
            "",
            "1st",
            "_x",
            "has space",
            "tool/name",
            "dot.name",
            &"x".repeat(65),
        ] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn parse_catalog_accepts_valid_and_rejects_violations() {
        let ok = json!([entry("search"), {"name": "fetch", "description": "fetches"}]);
        let entries = parse_catalog(&ok).expect("valid");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "search");
        assert_eq!(entries[1].input_schema, json!({}));

        let dup = json!([entry("search"), entry("search")]);
        assert!(parse_catalog(&dup).is_err());
        let empty = json!([]);
        assert!(parse_catalog(&empty).is_err());
        let no_desc = json!([{"name": "x"}]);
        assert!(parse_catalog(&no_desc).is_err());
        let bad_schema = json!([{"name": "x", "description": "d", "inputSchema": [1]}]);
        assert!(parse_catalog(&bad_schema).is_err());
        let not_array = json!({"tools": []});
        assert!(parse_catalog(&not_array).is_err());
    }

    #[test]
    fn extract_json_array_tolerates_fences_and_prose() {
        assert_eq!(extract_json_array("[]"), Some("[]"));
        assert_eq!(extract_json_array("```json\n[1]\n```"), Some("[1]"));
        assert_eq!(
            extract_json_array("here: [{\"a\":1}] done"),
            Some("[{\"a\":1}]")
        );
        assert_eq!(extract_json_array("nope"), None);
    }

    #[test]
    fn freeze_and_load_frozen_round_trip_on_matching_digest() {
        let dir = tempfile::tempdir().expect("tempdir");
        let entries = parse_catalog(&json!([entry("search")])).expect("entries");
        freeze(dir.path(), &entries, "digest-1").expect("freeze");
        assert!(load_frozen(dir.path(), "digest-1").is_some());
        assert!(load_frozen(dir.path(), "digest-2").is_none());
    }

    #[test]
    fn memo_round_trip_and_last_wins() {
        let dir = tempfile::tempdir().expect("tempdir");
        let key = memo_key("search", &json!({"q":"ada"}));
        assert_eq!(memo_get(dir.path(), &key), None);
        memo_put(dir.path(), &key, "search", "first").expect("put");
        memo_put(dir.path(), &key, "search", "second").expect("put");
        assert_eq!(memo_get(dir.path(), &key), Some("second".to_string()));
        // Different args → different key.
        let other = memo_key("search", &json!({"q":"grace"}));
        assert_eq!(memo_get(dir.path(), &other), None);
    }

    #[test]
    fn memo_key_depends_on_name_and_args() {
        let a = memo_key("search", &json!({"q":"ada"}));
        let b = memo_key("fetch", &json!({"q":"ada"}));
        let c = memo_key("search", &json!({"q":"grace"}));
        assert_ne!(a, b);
        assert_ne!(a, c);
        assert_eq!(a, memo_key("search", &json!({"q":"ada"})));
    }
}
