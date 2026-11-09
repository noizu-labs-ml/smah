//! The SQLite-backed virtual store for the db driver (ticket #24).
//!
//! The LLM **never writes SQL**: it emits a structured JSON schema spec
//! (`{"tables":[{"name","columns":[{name,type}],"rows":[[...]]}]}`); this
//! module validates the spec, emits the `CREATE TABLE`/`INSERT` statements
//! itself, and builds `~/.cache/smah/virtual-tools/<tool>/db/store.sqlite`.
//! Serving reopens the file **read-only**
//! ([`rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY`]) — the store physically
//! cannot be written through the wire surface even if the lexical SELECT
//! gate were bypassed.
//!
//! The spec is frozen under a content marker (`db.sha256` = tool-file
//! digest): editing the tool file regenerates the store; otherwise the
//! build is reused (caches are regenerable — `--regen` forces a rebuild).

use std::path::{Path, PathBuf};

use rusqlite::types::Value as SqlValue;
use rusqlite::OpenFlags;
use serde_json::Value as Json;

use crate::errors::SmahError;
use crate::virtual_tools::audit::Audit;
use crate::virtual_tools::drivers::ChatMessage;
use crate::virtual_tools::loader::ToolDef;

use super::support;
use super::wire::{QueryResult, QueryStore};

/// Spec caps (enforced in code; the prompt states tighter guidance).
pub(crate) const MAX_TABLES: usize = 64;
pub(crate) const MAX_COLUMNS: usize = 64;
pub(crate) const MAX_ROWS_PER_TABLE: usize = 500;
pub(crate) const MAX_ROWS_TOTAL: usize = 2000;
pub(crate) const SPEC_BYTES_MAX: usize = 512 * 1024;

/// Canonical column types the spec may declare. Anything else is a regen.
const KNOWN_TYPES: [&str; 16] = [
    "INTEGER",
    "BIGINT",
    "SMALLINT",
    "SERIAL",
    "TEXT",
    "VARCHAR",
    "UUID",
    "JSON",
    "JSONB",
    "TIMESTAMP",
    "DATE",
    "BOOLEAN",
    "REAL",
    "DOUBLE",
    "NUMERIC",
    "BLOB",
];

/// A validated fabricated table.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct TableSpec {
    pub name: String,
    pub columns: Vec<(String, String)>, // (name, canonical DECL)
    pub rows: Vec<Vec<SqlValue>>,
}

// --- type mapping -----------------------------------------------------------

/// Canonicalize a spec type name to the SQLite DECL it is created with.
/// `None` = unknown type (regen: the contract lists the exact set).
pub(crate) fn canonical_type(raw: &str) -> Option<&'static str> {
    let upper = raw.trim().to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or("").trim();
    KNOWN_TYPES.iter().copied().find(|known| *known == base)
}

/// Map a SQLite DECL to (Postgres oid, typlen) for RowDescription.
pub(crate) fn decl_to_pg(decl: &str) -> (i32, i16) {
    match decl.trim().to_ascii_uppercase().as_str() {
        "INTEGER" | "BIGINT" | "SMALLINT" | "SERIAL" => (23, 4),
        "BOOLEAN" => (16, 1),
        "REAL" | "DOUBLE" => (701, 8),
        "NUMERIC" => (1700, -1),
        "BLOB" => (17, -1),
        // TEXT/VARCHAR/UUID/JSON/JSONB/TIMESTAMP/DATE all serve as text.
        _ => (25, -1),
    }
}

// --- spec parsing + validation ---------------------------------------------

fn is_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    name.len() <= 64 && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Parse + validate a spec JSON document. `Err` is a one-line reason fed back
/// to the model for one retry; a second failure aborts the run.
pub(crate) fn parse_spec(spec: &Json) -> Result<Vec<TableSpec>, String> {
    let tables = spec
        .get("tables")
        .and_then(Json::as_array)
        .ok_or("spec must be an object with a 'tables' array")?;
    if tables.is_empty() {
        return Err("spec declares no tables".to_string());
    }
    if tables.len() > MAX_TABLES {
        return Err(format!(
            "spec declares {} tables (max {MAX_TABLES})",
            tables.len()
        ));
    }
    let mut out = Vec::new();
    let mut total_rows = 0usize;
    for table in tables {
        let name = table
            .get("name")
            .and_then(Json::as_str)
            .ok_or("each table needs a string 'name'")?;
        if !is_identifier(name) {
            return Err(format!("table name '{name}' is not a plain SQL identifier"));
        }
        let cols = table
            .get("columns")
            .and_then(Json::as_array)
            .ok_or(format!("table '{name}' needs a 'columns' array"))?;
        if cols.is_empty() {
            return Err(format!("table '{name}' declares no columns"));
        }
        if cols.len() > MAX_COLUMNS {
            return Err(format!(
                "table '{name}' declares {} columns (max {MAX_COLUMNS})",
                cols.len()
            ));
        }
        let mut columns = Vec::new();
        for col in cols {
            let cname = col
                .get("name")
                .and_then(Json::as_str)
                .ok_or(format!("table '{name}': each column needs a string 'name'"))?;
            if !is_identifier(cname) {
                return Err(format!(
                    "table '{name}': column name '{cname}' is not a plain SQL identifier"
                ));
            }
            let raw_type = col.get("type").and_then(Json::as_str).ok_or(format!(
                "table '{name}': column '{cname}' needs a string 'type'"
            ))?;
            let decl = canonical_type(raw_type).ok_or_else(|| {
                format!(
                    "table '{name}': column '{cname}' type '{raw_type}' is not one of {}",
                    KNOWN_TYPES.join(", ")
                )
            })?;
            columns.push((cname.to_string(), decl.to_string()));
        }
        let rows_json = table
            .get("rows")
            .and_then(Json::as_array)
            .ok_or(format!("table '{name}' needs a 'rows' array"))?;
        if rows_json.len() > MAX_ROWS_PER_TABLE {
            return Err(format!(
                "table '{name}' declares {} rows (max {MAX_ROWS_PER_TABLE})",
                rows_json.len()
            ));
        }
        total_rows += rows_json.len();
        if total_rows > MAX_ROWS_TOTAL {
            return Err(format!(
                "spec declares more than {MAX_ROWS_TOTAL} rows in total"
            ));
        }
        let mut rows = Vec::with_capacity(rows_json.len());
        for (idx, row) in rows_json.iter().enumerate() {
            let cells = row.as_array().ok_or(format!(
                "table '{name}' row {idx}: rows are arrays of scalars"
            ))?;
            if cells.len() != columns.len() {
                return Err(format!(
                    "table '{name}' row {idx}: {} values for {} columns",
                    cells.len(),
                    columns.len()
                ));
            }
            let mut converted = Vec::with_capacity(cells.len());
            for (cidx, cell) in cells.iter().enumerate() {
                let decl = &columns[cidx].1;
                converted.push(convert_cell(decl, cell).ok_or_else(|| {
                    format!(
                        "table '{name}' row {idx} column '{}': value does not fit its {decl} type",
                        columns[cidx].0
                    )
                })?);
            }
            rows.push(converted);
        }
        out.push(TableSpec {
            name: name.to_string(),
            columns,
            rows,
        });
    }
    Ok(out)
}

/// JSON scalar → SQLite value for the given canonical DECL.
fn convert_cell(decl: &str, cell: &Json) -> Option<SqlValue> {
    let upper = decl.to_ascii_uppercase();
    match cell {
        Json::Null => Some(SqlValue::Null),
        Json::Bool(b) if upper == "BOOLEAN" => Some(SqlValue::Integer(i64::from(*b))),
        Json::Number(n) => {
            if upper.contains("INT") || upper == "SERIAL" {
                n.as_i64().map(SqlValue::Integer)
            } else if matches!(upper.as_str(), "REAL" | "DOUBLE" | "NUMERIC") {
                n.as_f64().map(SqlValue::Real)
            } else {
                Some(SqlValue::Text(n.to_string()))
            }
        }
        Json::String(s) => {
            if upper.contains("INT") || upper == "SERIAL" {
                s.parse::<i64>().ok().map(SqlValue::Integer)
            } else if matches!(upper.as_str(), "REAL" | "DOUBLE" | "NUMERIC") {
                s.parse::<f64>().ok().map(SqlValue::Real)
            } else if upper == "BOOLEAN" {
                match s.to_ascii_lowercase().as_str() {
                    "true" | "t" | "yes" | "1" => Some(SqlValue::Integer(1)),
                    "false" | "f" | "no" | "0" => Some(SqlValue::Integer(0)),
                    _ => None,
                }
            } else {
                Some(SqlValue::Text(s.clone()))
            }
        }
        _ => None, // objects/arrays are not scalars
    }
}

// --- DDL emission (code-owned; the model never writes SQL) ------------------

/// The `CREATE TABLE` script for a validated spec (transactional header).
pub(crate) fn emit_ddl(tables: &[TableSpec]) -> String {
    let mut out = String::from("PRAGMA journal_mode=DELETE;\nBEGIN;\n");
    for table in tables {
        let cols = table
            .columns
            .iter()
            .map(|(name, decl)| format!("\"{name}\" {decl}"))
            .collect::<Vec<_>>()
            .join(", ");
        out.push_str(&format!("CREATE TABLE \"{}\" ({cols});\n", table.name));
    }
    out.push_str("COMMIT;\n");
    out
}

/// Build the store file from a validated spec (fresh file, DDL + inserts).
/// The file is left journal-free (`journal_mode=DELETE`) so the read-only
/// serving connection never trips over a hot WAL.
pub(crate) fn build_store(path: &Path, tables: &[TableSpec]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("store dir: {}", support::one_line(&err.to_string())))?;
    }
    let _ = std::fs::remove_file(path);
    let conn = rusqlite::Connection::open(path)
        .map_err(|err| format!("store create: {}", support::one_line(&err.to_string())))?;
    let ddl = emit_ddl(tables);
    conn.execute_batch(&ddl)
        .map_err(|err| format!("store ddl: {}", support::one_line(&err.to_string())))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|err| format!("store tx: {}", support::one_line(&err.to_string())))?;
    for table in tables {
        let cols = table
            .columns
            .iter()
            .map(|(name, _)| format!("\"{name}\""))
            .collect::<Vec<_>>()
            .join(", ");
        let placeholders = table
            .columns
            .iter()
            .map(|_| "?")
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "INSERT INTO \"{}\" ({cols}) VALUES ({placeholders})",
            table.name
        );
        for row in &table.rows {
            let mut stmt = conn.prepare(&sql).map_err(|err| {
                format!("insert prepare: {}", support::one_line(&err.to_string()))
            })?;
            stmt.execute(rusqlite::params_from_iter(row.iter()))
                .map_err(|err| format!("insert: {}", support::one_line(&err.to_string())))?;
        }
    }
    tx.commit()
        .map_err(|err| format!("store commit: {}", support::one_line(&err.to_string())))?;
    Ok(())
}

// --- read-only serving store ------------------------------------------------

/// The serving-side store: opened read-only, implements the wire layer's
/// SELECT execution.
pub(crate) struct DbStore {
    conn: rusqlite::Connection,
}

impl DbStore {
    /// Open the store read-only (the physical write-refusal boundary).
    pub fn open_readonly(path: &Path) -> Result<Self, String> {
        let conn = rusqlite::Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|err| format!("store open: {}", support::one_line(&err.to_string())))?;
        Ok(DbStore { conn })
    }
}

impl QueryStore for DbStore {
    fn select(&self, sql: &str) -> Result<QueryResult, String> {
        let mut stmt = self.conn.prepare(sql).map_err(|err| {
            format!(
                "virtual store rejected the statement: {}",
                support::one_line(&err.to_string())
            )
        })?;
        let count = stmt.column_count();
        let mut columns = Vec::with_capacity(count);
        for idx in 0..count {
            let name = stmt
                .column_name(idx)
                .map_err(|err| support::one_line(&err.to_string()))?
                .to_string();
            let decl = column_decl(&stmt, idx);
            let (oid, typlen) = decl_to_pg(&decl);
            columns.push((name, oid, typlen));
        }
        let mut rows_out = Vec::new();
        let mut rows = stmt.query([]).map_err(|err| {
            format!(
                "virtual store rejected the statement: {}",
                support::one_line(&err.to_string())
            )
        })?;
        while let Some(row) = rows.next().map_err(|err| {
            format!(
                "virtual store error: {}",
                support::one_line(&err.to_string())
            )
        })? {
            let mut served = Vec::with_capacity(count);
            for idx in 0..count {
                served.push(
                    match row
                        .get_ref(idx)
                        .map_err(|err| support::one_line(&err.to_string()))?
                    {
                        rusqlite::types::ValueRef::Null => None,
                        rusqlite::types::ValueRef::Integer(v) => Some(v.to_string()),
                        rusqlite::types::ValueRef::Real(v) => Some(v.to_string()),
                        rusqlite::types::ValueRef::Text(bytes) => {
                            Some(String::from_utf8_lossy(bytes).into_owned())
                        }
                        rusqlite::types::ValueRef::Blob(bytes) => {
                            let mut hex = String::with_capacity(bytes.len() * 2 + 2);
                            hex.push_str("\\x");
                            for byte in bytes {
                                hex.push_str(&format!("{byte:02x}"));
                            }
                            Some(hex)
                        }
                    },
                );
            }
            rows_out.push(served);
        }
        Ok((columns, rows_out))
    }
}

/// The declared type of a result column ("" when none). Isolated here so the
/// rusqlite `DeclType` access shape has exactly one place to fix.
fn column_decl(stmt: &rusqlite::Statement<'_>, idx: usize) -> String {
    stmt.columns()
        .get(idx)
        .and_then(rusqlite::Column::decl_type)
        .map(|d| d.to_string())
        .unwrap_or_default()
}

// --- fabrication loop -------------------------------------------------------

pub(crate) const FABRICATE_SYSTEM: &str = concat!(
    "You fabricate the schema and rows of a VIRTUAL postgres database for a smah virtual tool.\n",
    "Output ONLY one JSON object, no markdown fences, no prose:\n",
    "{\"tables\":[{\"name\":\"<identifier>\",\"columns\":[{\"name\":\"<identifier>\",\"type\":\"<TYPE>\"}],\"rows\":[[<scalar>,...]]}]}\n",
    "<TYPE> is exactly one of: INTEGER, BIGINT, SMALLINT, SERIAL, TEXT, VARCHAR, UUID, JSON, JSONB, TIMESTAMP, DATE, BOOLEAN, REAL, DOUBLE, NUMERIC, BLOB.\n",
    "Every row value is a JSON scalar (string, number, boolean, or null) that fits its column type.\n",
    "Do NOT output CREATE TABLE, INSERT, or any SQL: smah builds the database itself from this JSON.\n",
    "Keep it small and coherent: a handful of realistic rows per table."
);

/// Fabrication user prompt: persona + constraints + the tool's `interface:`
/// seed (the declared contract the schema must embody).
pub(crate) fn fabricate_user_prompt(tool: &ToolDef) -> String {
    let mut out = String::new();
    out.push_str("VIRTUAL DATABASE PERSONA:\n");
    out.push_str(&tool.body);
    if !tool.constraints.is_empty() {
        out.push_str("\n\nHard constraints:\n");
        for constraint in &tool.constraints {
            out.push_str(&format!("- {constraint}\n"));
        }
    }
    out.push_str("\nContract artifact (interface):\n");
    match &tool.interface {
        Some(interface) if interface.len() <= SPEC_BYTES_MAX => out.push_str(interface),
        Some(_) => out.push_str(
            "(interface artifact over the size cap — invent a coherent schema fitting the persona)",
        ),
        None => {
            out.push_str("(none declared — invent a coherent schema and rows fitting the persona)")
        }
    }
    out.push_str(
        "\n\nFabricate the virtual database schema + rows now. Output only the JSON object.",
    );
    out
}

/// Extract the JSON object from a model reply (tolerates fences and prose).
pub(crate) fn extract_json(text: &str) -> Option<&str> {
    let trimmed = text.trim();
    let start = trimmed.find('{')?;
    let end = trimmed.rfind('}')? + 1;
    if end <= start || start > SPEC_BYTES_MAX {
        return None;
    }
    Some(&trimmed[start..end])
}

/// Store path: `<cache leaf>/store.sqlite`.
pub(crate) fn store_path(dir: &Path) -> PathBuf {
    dir.join("store.sqlite")
}

/// Marker path: `<cache leaf>/db.sha256`.
pub(crate) fn marker_path(dir: &Path) -> PathBuf {
    dir.join(support::MARKER_DB)
}

/// One fabrication attempt: extract → parse → validate → build → marker.
/// `Err` is a one-line rejection reason (fed back for the single retry).
fn attempt(reply: &str, store: &Path, marker: &Path, digest: &str) -> Result<(), String> {
    let json_text = extract_json(reply).ok_or("reply contained no JSON object")?;
    let parsed: Json = serde_json::from_str(json_text).map_err(|err| {
        format!(
            "spec JSON did not parse: {}",
            support::one_line(&err.to_string())
        )
    })?;
    let tables = parse_spec(&parsed)?;
    build_store(store, &tables)?;
    if let Some(parent) = marker.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cache dir: {}", support::one_line(&err.to_string())))?;
    }
    // The marker records the tool-file digest that produced this store.
    std::fs::write(marker, digest)
        .map_err(|err| format!("cache marker: {}", support::one_line(&err.to_string())))?;
    Ok(())
}

/// Ensure the fabricated store exists and matches the current tool file.
/// Returns the store path and the model turns consumed. At most
/// [`crate::virtual_tools::TURNS_MAX`] bounded attempts; two invalid specs
/// abort with [`SmahError::DirectiveLoopOverflow`] (exit 5).
pub(crate) fn ensure_store(
    tool: &ToolDef,
    backend: &dyn crate::virtual_tools::drivers::ChatBackend,
    model: &str,
    audit: &Audit,
    run_id: &str,
    dir: &Path,
    force: bool,
) -> Result<(PathBuf, usize), SmahError> {
    let store = store_path(dir);
    let marker = marker_path(dir);
    let (digest, tool_len) =
        support::sha256_file(&tool.path).map_err(|err| SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!("unreadable while keying the cache: {err}"),
        })?;

    if !force && store.is_file() {
        if let Ok(recorded) = std::fs::read_to_string(&marker) {
            if recorded.trim() == digest {
                // Reuse: opening read-only also validates the file.
                DbStore::open_readonly(&store).map_err(|err| SmahError::ToolFileInvalid {
                    path: store.display().to_string(),
                    detail: format!("cached store is corrupt ({err}); rerun with --regen"),
                })?;
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "db.fabricate",
                    "reuse",
                    Some(tool_len as usize),
                    "allowed",
                    "",
                );
                return Ok((store, 0));
            }
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
        match attempt(&reply, &store, &marker, &digest) {
            Ok(()) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "db.fabricate",
                    "generate",
                    Some(reply.len()),
                    "allowed",
                    "",
                );
                return Ok((store, turns));
            }
            Err(reason) => {
                support::audit_server_op(
                    audit,
                    run_id,
                    &tool.name,
                    "db.fabricate",
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
                    "{}\n\nYour previous spec was rejected: {reason}\n\
                     Fix it and output only the corrected JSON object.",
                    fabricate_user_prompt(tool)
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn canonical_type_accepts_the_contract_set_and_rejects_the_rest() {
        for name in KNOWN_TYPES {
            assert_eq!(canonical_type(name), Some(name), "{name}");
        }
        assert_eq!(canonical_type("varchar(255)"), Some("VARCHAR"));
        assert_eq!(canonical_type("  integer "), Some("INTEGER"));
        assert_eq!(canonical_type("MONEY"), None);
        assert_eq!(canonical_type(""), None);
    }

    #[test]
    fn decl_to_pg_maps_the_families() {
        assert_eq!(decl_to_pg("INTEGER"), (23, 4));
        assert_eq!(decl_to_pg("BOOLEAN"), (16, 1));
        assert_eq!(decl_to_pg("DOUBLE"), (701, 8));
        assert_eq!(decl_to_pg("NUMERIC"), (1700, -1));
        assert_eq!(decl_to_pg("BLOB"), (17, -1));
        // JSON/JSONB serve as plain text — the store is SQLite, the wire lies gently.
        assert_eq!(decl_to_pg("JSONB"), (25, -1));
        assert_eq!(decl_to_pg("weird"), (25, -1));
    }

    #[test]
    fn parse_spec_accepts_a_valid_spec() {
        let spec = json!({
            "tables": [{
                "name": "users",
                "columns": [
                    {"name": "id", "type": "SERIAL"},
                    {"name": "email", "type": "TEXT"},
                    {"name": "active", "type": "BOOLEAN"}
                ],
                "rows": [
                    [1, "ada@example.com", true],
                    [2, null, false]
                ]
            }]
        });
        let tables = parse_spec(&spec).expect("valid spec");
        assert_eq!(tables.len(), 1);
        assert_eq!(tables[0].name, "users");
        assert_eq!(tables[0].columns.len(), 3);
        assert_eq!(tables[0].rows.len(), 2);
        assert_eq!(tables[0].rows[1][1], SqlValue::Null);
    }

    #[test]
    fn parse_spec_rejects_bad_names_types_and_shapes() {
        let bad_name = json!({"tables":[{"name":"drop table","columns":[{"name":"a","type":"TEXT"}],"rows":[]}]});
        assert!(parse_spec(&bad_name).is_err());
        let bad_type =
            json!({"tables":[{"name":"t","columns":[{"name":"a","type":"MONEY"}],"rows":[]}]});
        assert!(parse_spec(&bad_type).is_err());
        let ragged =
            json!({"tables":[{"name":"t","columns":[{"name":"a","type":"TEXT"}],"rows":[[1,2]]}]});
        assert!(parse_spec(&ragged).is_err());
        let no_tables = json!({"tables":[]});
        assert!(parse_spec(&no_tables).is_err());
        let dup_rows = json!({"tables":[{"name":"t","columns":[{"name":"a","type":"TEXT"}],"rows":[["x"],["y"]]}]});
        assert!(parse_spec(&dup_rows).is_ok());
    }

    #[test]
    fn convert_cell_respects_the_column_type() {
        assert_eq!(
            convert_cell("INTEGER", &json!("42")),
            Some(SqlValue::Integer(42))
        );
        assert_eq!(convert_cell("INTEGER", &json!("nope")), None);
        assert_eq!(
            convert_cell("BOOLEAN", &json!(true)),
            Some(SqlValue::Integer(1))
        );
        assert_eq!(
            convert_cell("BOOLEAN", &json!("false")),
            Some(SqlValue::Integer(0))
        );
        assert_eq!(convert_cell("REAL", &json!(1.5)), Some(SqlValue::Real(1.5)));
        assert_eq!(
            convert_cell("TEXT", &json!(12)),
            Some(SqlValue::Text("12".into()))
        );
        assert_eq!(convert_cell("TEXT", &json!({"x":1})), None);
    }

    #[test]
    fn emit_ddl_is_code_owned_and_transactional() {
        let tables = vec![TableSpec {
            name: "users".into(),
            columns: vec![
                ("id".into(), "INTEGER".into()),
                ("email".into(), "TEXT".into()),
            ],
            rows: Vec::new(),
        }];
        let ddl = emit_ddl(&tables);
        assert!(ddl.starts_with("PRAGMA journal_mode=DELETE;"));
        assert!(ddl.contains("CREATE TABLE \"users\" (\"id\" INTEGER, \"email\" TEXT);"));
        assert!(ddl.contains("BEGIN;") && ddl.contains("COMMIT;"));
    }

    #[test]
    fn extract_json_tolerates_fences_and_prose() {
        assert_eq!(extract_json("{\"tables\":[]}"), Some("{\"tables\":[]}"));
        assert_eq!(
            extract_json("```json\n{\"tables\":[]}\n```"),
            Some("{\"tables\":[]}")
        );
        assert_eq!(
            extract_json("Here you go: {\"tables\":[]} hope it helps"),
            Some("{\"tables\":[]}")
        );
        assert_eq!(extract_json("no json at all"), None);
    }

    #[test]
    fn build_store_and_readonly_round_trip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("store.sqlite");
        let tables = vec![TableSpec {
            name: "users".into(),
            columns: vec![
                ("id".into(), "INTEGER".into()),
                ("email".into(), "TEXT".into()),
            ],
            rows: vec![vec![
                SqlValue::Integer(1),
                SqlValue::Text("ada@example.com".into()),
            ]],
        }];
        build_store(&path, &tables).expect("build");
        let store = DbStore::open_readonly(&path).expect("open");
        let (columns, rows) = store
            .select("SELECT id, email FROM users ORDER BY id")
            .expect("select");
        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].0, "id");
        assert_eq!(
            rows,
            vec![vec![Some("1".into()), Some("ada@example.com".into())]]
        );
        // The write refusal is physical: the connection is read-only.
        assert!(store.select("INSERT INTO users VALUES (2, 'x')").is_err());
        assert!(store.select("DROP TABLE users").is_err());
    }
}
