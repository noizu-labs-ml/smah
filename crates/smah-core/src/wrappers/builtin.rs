//! The built-in tool registry (SW-04): reusable, pure-Rust step tools a
//! wrapper `use:` may reference by name (owner decision Q3 — steps
//! reference reusable tools BY NAME).
//!
//! A built-in is data + two function pointers: an argv rendering for the
//! plan printer (what the step *does*, display-only) and a pure-Rust
//! `run` (no exec — `smah-run` executes built-ins in-process; `smah`
//! itself never executes anything).
//!
//! Registry growth rule: a new built-in lands with its own tests and a
//! registry row here; wrapper files reference these names, so names are
//! frozen once shipped (same discipline as driver names).

use std::collections::BTreeMap;

use crate::wrappers::cast;

/// One declared built-in param.
pub struct BuiltinParam {
    pub name: &'static str,
    pub required: bool,
    pub default: Option<&'static str>,
}

/// A compiled-in step tool.
pub struct BuiltinTool {
    pub name: &'static str,
    pub description: &'static str,
    pub params: &'static [BuiltinParam],
    /// Display argv for the deterministic plan (display-only; never run by smah).
    pub argv_display: fn(&BTreeMap<String, String>) -> Vec<String>,
    /// Pure-Rust execution (`smah-run` calls this in-process).
    pub run: fn(&BTreeMap<String, String>) -> Result<String, String>,
}

impl BuiltinTool {
    /// Validate a step's `with` map against the declared params: unknown
    /// keys reject; required keys must be present (or carry a default).
    /// Returns the effective binding map (defaults applied).
    pub fn bind(&self, with: &[(String, String)]) -> Result<BTreeMap<String, String>, String> {
        let mut bound: BTreeMap<String, String> = BTreeMap::new();
        for (key, value) in with {
            if !self.params.iter().any(|p| p.name == key) {
                return Err(format!(
                    "built-in '{}' has no param '{}' (params: {})",
                    self.name,
                    key,
                    self.params
                        .iter()
                        .map(|p| p.name)
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            bound.insert(key.clone(), value.clone());
        }
        for p in self.params {
            if let Some(default) = p.default {
                bound
                    .entry(p.name.to_string())
                    .or_insert_with(|| default.to_string());
            }
        }
        for p in self.params {
            if p.required && !bound.contains_key(p.name) {
                return Err(format!(
                    "built-in '{}' requires param '{}' (no default)",
                    self.name, p.name
                ));
            }
        }
        Ok(bound)
    }
}

/// The registry, in stable display order.
static REGISTRY: [BuiltinTool; 1] = [BuiltinTool {
    name: "cast-to-text",
    description: "Render an existing asciinema v2 .cast recording to a plain-text transcript (pure Rust, no exec)",
    params: &[BuiltinParam {
        name: "cast_file",
        required: true,
        default: None,
    }],
    argv_display: argv_cast_to_text,
    run: run_cast_to_text,
}];

/// The registry slice, in stable display order.
pub fn builtins() -> &'static [BuiltinTool] {
    &REGISTRY
}

/// Resolve a built-in by name.
pub fn builtin(name: &str) -> Option<&'static BuiltinTool> {
    builtins().iter().find(|b| b.name == name)
}

fn argv_cast_to_text(with: &BTreeMap<String, String>) -> Vec<String> {
    let file = with.get("cast_file").cloned().unwrap_or_default();
    vec!["cast-to-text".to_string(), "--cast-file".to_string(), file]
}

fn run_cast_to_text(with: &BTreeMap<String, String>) -> Result<String, String> {
    let file = with
        .get("cast_file")
        .ok_or_else(|| "cast-to-text: missing cast_file".to_string())?;
    let raw = std::fs::read_to_string(file)
        .map_err(|err| format!("cast-to-text: cannot read '{file}': {err}"))?;
    cast::cast_to_text(&raw).map_err(|err| format!("cast-to-text: {file}: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_map(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn registry_resolves_by_name() {
        assert!(builtin("cast-to-text").is_some());
        assert!(builtin("nope").is_none());
        assert_eq!(builtins().len(), 1);
    }

    #[test]
    fn bind_rejects_unknown_and_missing_required() {
        let tool = builtin("cast-to-text").expect("registered");
        assert!(tool.bind(&with_map(&[("cast_file", "a.cast")])).is_ok());
        let unknown = tool.bind(&with_map(&[("bogus", "x")])).unwrap_err();
        assert!(unknown.contains("no param 'bogus'"));
        let missing = tool.bind(&[]).unwrap_err();
        assert!(missing.contains("requires param 'cast_file'"));
    }

    #[test]
    fn argv_display_shapes_the_plan_line() {
        let tool = builtin("cast-to-text").expect("registered");
        let bound = tool
            .bind(&with_map(&[("cast_file", "session.cast")]))
            .expect("binds");
        assert_eq!(
            (tool.argv_display)(&bound),
            vec!["cast-to-text", "--cast-file", "session.cast"]
        );
    }

    #[test]
    fn run_renders_a_transcript_from_disk() {
        let dir = tempfile::tempdir().expect("tmp");
        let file = dir.path().join("session.cast");
        std::fs::write(
            &file,
            concat!(
                r#"{"version": 2, "width": 10, "height": 4}"#,
                "\n",
                r#"[0.1, "o", "$ echo hi\n"]"#,
                "\n",
                r#"[0.2, "o", "hi\n"]"#,
                "\n"
            ),
        )
        .expect("fixture");
        let tool = builtin("cast-to-text").expect("registered");
        let bound = tool
            .bind(&with_map(&[("cast_file", file.to_str().unwrap())]))
            .expect("binds");
        assert_eq!((tool.run)(&bound).expect("runs"), "$ echo hi\nhi\n");
    }

    #[test]
    fn run_names_missing_file() {
        let tool = builtin("cast-to-text").expect("registered");
        let bound = tool
            .bind(&with_map(&[("cast_file", "/nonexistent/x.cast")]))
            .expect("binds");
        assert!((tool.run)(&bound).unwrap_err().contains("cannot read"));
    }
}
