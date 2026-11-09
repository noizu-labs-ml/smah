//! Wrapper-file loading: parse, validate, and discover
//! `~/.config/smah/wrappers/*.md` (SW-01, owner decisions Q2/Q6 2026-11-09).
//!
//! One file per wrapper; the filename stem **is** the wrapper name (same
//! resolution contract as the virtual-tool loader). Strictness mirrors it:
//!
//! - unknown frontmatter keys reject the file;
//! - `name` must equal the stem, pass [`crate::virtual_tools::is_valid_name`],
//!   avoid [`crate::virtual_tools::RESERVED_NAMES`], **and** not collide
//!   with any installed virtual tool (disjoint namespaces — a collision is
//!   a load error, not a shadow);
//! - `steps` is required, non-empty, and fully validated: unique ids,
//!   exactly one of `use:` (a reusable tool, resolved by name) / `run:`
//!   (a fixed argv template — never a shell line), unique `output` bindings;
//! - every `{{...}}` placeholder in a template string must statically
//!   resolve: `{{params.<declared>}}` or `{{steps.<earlier-id>.output}}`
//!   where the earlier step declares that `output`. Forward references and
//!   unknown params are load errors — a recipe that cannot resolve is a
//!   broken recipe, and it must fail before anything prints an execution
//!   plan around it;
//! - `approved:` (optional, RFC 3339) is the explicit user sign-off
//!   `smah-run --execute` demands (owner decision 2026-11-09): absent, the
//!   wrapper may be planned and dry-run but never executed.
//!
//! The loader is registry-agnostic: `use:` names are shape-checked here and
//! resolved against the built-in registry by [`crate::wrappers::resolve`].

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use serde_yaml_ng::Value;

use crate::virtual_tools::{is_valid_name, RESERVED_NAMES};
use crate::wrappers::{Binding, PARAMS_MAX, STEPS_MAX};

/// A parsed, valid wrapper definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperDef {
    /// Wrapper name (== filename stem).
    pub name: String,
    /// One-line description, shown by `smah wrapper list`.
    pub description: String,
    /// `"<provider>.<model-id>"` or bare id for `--prompt` composition;
    /// `None` = the configured/spec default model.
    pub model: Option<String>,
    /// Persona injected into the `--prompt` composition prompt.
    pub behavior_prompt: Option<String>,
    /// RFC 3339 timestamp the user approved this recipe for execution.
    /// `None` = plan/dry-run only (`smah-run --execute` refuses).
    pub approved: Option<String>,
    /// Declared invocation params.
    pub params: Vec<WrapperParam>,
    /// Ordered recipe steps.
    pub steps: Vec<WrapperStep>,
    /// Declared final outputs (each must be bound by exactly one step).
    pub outputs: Vec<WrapperOutput>,
    /// Human-readable preconditions, echoed by the plan printer.
    pub preconditions: Vec<String>,
    /// The markdown body (docs; may be empty — the recipe is frontmatter).
    pub body: String,
    /// Where the file lives (errors, cache keys, provenance rewrites).
    pub path: PathBuf,
}

/// One declared wrapper param.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperParam {
    pub name: String,
    pub description: Option<String>,
    /// Required params must be supplied at invocation (missing ⇒ usage error).
    pub required: bool,
    /// Default applied when the param is absent and not required.
    pub default: Option<String>,
}

/// One ordered recipe step. Exactly one of `use_` / `run` is `Some`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperStep {
    pub id: String,
    pub description: Option<String>,
    /// `use: <name>` — a reusable tool from the built-in registry (SW-04).
    pub use_: Option<String>,
    /// `run: [argv…]` — a fixed argv template (NO shell, no `sh -c`).
    pub run: Option<Vec<String>>,
    /// `with:` bindings handed to a `use:` tool.
    pub with: Vec<(String, String)>,
    /// Working directory template for `run:` steps.
    pub cwd: Option<String>,
    /// Per-step wall-clock budget in seconds (smah-run enforces).
    pub timeout_secs: Option<u64>,
    /// Name this step's captured stdout is bound to for downstream steps.
    pub output: Option<String>,
}

impl WrapperStep {
    /// The display kind: `"use <tool>"` or `"run"`.
    pub fn kind(&self) -> &str {
        if self.use_.is_some() {
            "use"
        } else {
            "run"
        }
    }
}

/// One declared output.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WrapperOutput {
    pub name: String,
    pub description: Option<String>,
}

/// A file present in the wrappers dir that failed validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvalidWrapper {
    pub path: PathBuf,
    pub name: String,
    pub reason: String,
}

/// Frontmatter keys the loader accepts. Anything else rejects the file.
pub const KNOWN_KEYS: [&str; 9] = [
    "name",
    "description",
    "model",
    "behavior_prompt",
    "approved",
    "params",
    "steps",
    "outputs",
    "preconditions",
];

/// The wrappers directory: `SMAH_WRAPPERS_DIR` (test/harness seam) >
/// `$XDG_CONFIG_HOME/smah/wrappers` > `~/.config/smah/wrappers`.
pub fn wrappers_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("SMAH_WRAPPERS_DIR") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    let mut base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(x) if !x.is_empty() => PathBuf::from(x),
        _ => {
            let mut h = home::home_dir().unwrap_or_else(|| PathBuf::from("."));
            h.push(".config");
            h
        }
    };
    base.push("smah");
    base.push("wrappers");
    base
}

/// Discover every `.md` file in `dir`, sorted by stem. Missing dir ⇒ empty.
pub fn discover_in(dir: &Path) -> (Vec<WrapperDef>, Vec<InvalidWrapper>) {
    let mut stems: Vec<String> = std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().map(|x| x == "md").unwrap_or(false))
                .filter_map(|e| {
                    e.path()
                        .file_stem()
                        .map(|s| s.to_string_lossy().to_string())
                })
                .collect()
        })
        .unwrap_or_default();
    stems.sort();
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for stem in stems {
        let path = dir.join(format!("{stem}.md"));
        match load_from(&path) {
            Ok(def) => valid.push(def),
            Err(reason) => invalid.push(InvalidWrapper {
                path,
                name: stem,
                reason,
            }),
        }
    }
    (valid, invalid)
}

/// Discover in the default [`wrappers_dir`].
pub fn discover() -> (Vec<WrapperDef>, Vec<InvalidWrapper>) {
    discover_in(&wrappers_dir())
}

/// Load `<wrappers_dir>/<name>.md` by name (resolution for the direct
/// invoke path). Missing file ⇒ `Err(reason)` beginning `"not found:"`;
/// present-but-invalid ⇒ the one-line rejection reason. The CLI layer maps
/// these onto the exit-code table (SW-02).
pub fn load_by_name(name: &str) -> Result<WrapperDef, String> {
    let dir = wrappers_dir();
    let path = dir.join(format!("{name}.md"));
    if !path.is_file() {
        let (valid, _) = discover_in(&dir);
        let mut names: Vec<String> = valid.into_iter().map(|w| w.name).collect();
        names.sort();
        let installed = if names.is_empty() {
            "(none)".to_string()
        } else {
            names.join(", ")
        };
        return Err(format!(
            "not found: no wrapper named '{name}'. Installed: {installed}."
        ));
    }
    load_from(&path)
}

/// Parse and validate one wrapper file. `Err(reason)` is a one-line rejection.
pub fn load_from(path: &Path) -> Result<WrapperDef, String> {
    load_with_tool_names(path, &virtual_tool_names())
}

/// [`load_from`] with an explicit virtual-tool name set (test seam — the
/// namespace-disjointness check without touching process env).
pub fn load_with_tool_names(
    path: &Path,
    tool_names: &HashSet<String>,
) -> Result<WrapperDef, String> {
    let raw = std::fs::read_to_string(path)
        .map_err(|err| format!("unreadable: {}", one_line(&err.to_string())))?;
    let stem = path
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    let (frontmatter, body) = split_frontmatter(&raw, path)?;

    let Value::Mapping(map) = frontmatter else {
        return Err("frontmatter is not a key/value mapping".to_string());
    };
    for key in map.keys() {
        let key = key.as_str().unwrap_or("<non-string key>");
        if !KNOWN_KEYS.contains(&key) {
            return Err(format!("unknown frontmatter key '{key}'"));
        }
    }

    let get_str = |key: &str| -> Option<Result<String, String>> {
        map.get(Value::String(key.to_string())).map(|v| {
            v.as_str()
                .map(str::to_string)
                .ok_or_else(|| format!("'{key}' must be a string"))
        })
    };
    let get_list = |key: &str| -> Option<Result<Vec<String>, String>> {
        map.get(Value::String(key.to_string())).map(|v| {
            v.as_sequence()
                .map(|seq| {
                    seq.iter()
                        .map(|item| {
                            item.as_str()
                                .map(str::to_string)
                                .ok_or_else(|| format!("'{key}' entries must be strings"))
                        })
                        .collect::<Result<Vec<_>, _>>()
                })
                .unwrap_or_else(|| Err(format!("'{key}' must be a list")))
        })
    };

    // --- name / description / model / behavior_prompt / approved ----------
    let name = match get_str("name") {
        Some(Ok(n)) => n,
        Some(Err(err)) => return Err(err),
        None => return Err("missing required key 'name'".to_string()),
    };
    if name != stem {
        return Err(format!(
            "frontmatter name '{name}' does not match the filename stem '{stem}'"
        ));
    }
    if !is_valid_name(&name) {
        return Err(format!(
            "name '{name}' is invalid: lowercase letters/digits/'-'/'_', starting with a letter, at most 64 characters"
        ));
    }
    if RESERVED_NAMES.contains(&name.as_str()) {
        return Err(format!(
            "name '{name}' is reserved (it would collide with a smah command)"
        ));
    }
    // Disjoint namespace with virtual tools (owner decision Q6): a wrapper
    // shadowing a tool of the same name would make `smah-<name>` dispatch
    // ambiguous — refuse at load time.
    if tool_names.contains(&name) {
        return Err(format!(
            "name '{name}' collides with an installed virtual tool of the same name (namespaces are disjoint — rename one)"
        ));
    }

    let description = match get_str("description") {
        Some(Ok(d)) => d,
        Some(Err(err)) => return Err(err),
        None => return Err("missing required key 'description'".to_string()),
    };
    if description.trim().is_empty() {
        return Err("'description' must be non-empty".to_string());
    }

    let model = match get_str("model") {
        Some(Ok(m)) if m.trim().is_empty() => {
            return Err("'model' must be non-empty when present".to_string())
        }
        Some(Ok(m)) => Some(m),
        Some(Err(err)) => return Err(err),
        None => None,
    };

    let behavior_prompt = match get_str("behavior_prompt") {
        Some(Ok(p)) if p.trim().is_empty() => {
            return Err("'behavior_prompt' must be non-empty when present".to_string())
        }
        Some(Ok(p)) => Some(p),
        Some(Err(err)) => return Err(err),
        None => None,
    };

    let approved = match get_str("approved") {
        Some(Ok(a)) => {
            if chrono::DateTime::parse_from_rfc3339(&a).is_err() {
                return Err(format!(
                    "'approved' must be an RFC 3339 timestamp (e.g. 2026-11-09T14:30:00-08:00), got '{a}'"
                ));
            }
            Some(a)
        }
        Some(Err(err)) => return Err(err),
        None => None,
    };

    // --- params ------------------------------------------------------------
    let mut params: Vec<WrapperParam> = Vec::new();
    if let Some(entries) = map.get(Value::String("params".to_string())) {
        let seq = entries
            .as_sequence()
            .ok_or_else(|| "'params' must be a list".to_string())?;
        if seq.len() > PARAMS_MAX {
            return Err(format!("'params' declares more than {PARAMS_MAX} entries"));
        }
        for entry in seq {
            let emap = entry
                .as_mapping()
                .ok_or_else(|| "'params' entries must be mappings".to_string())?;
            for key in emap.keys() {
                let key = key.as_str().unwrap_or("<non-string key>");
                if !["name", "description", "required", "default"].contains(&key) {
                    return Err(format!("unknown param key '{key}'"));
                }
            }
            let pname = string_field(emap, "name")?
                .ok_or_else(|| "'params' entry missing 'name'".to_string())?;
            if !is_valid_name(&pname) {
                return Err(format!(
                    "param name '{pname}' is invalid (same rule as wrapper names)"
                ));
            }
            if params.iter().any(|p| p.name == pname) {
                return Err(format!("duplicate param name '{pname}'"));
            }
            let description = string_field(emap, "description")?;
            let required = match emap.get(Value::String("required".to_string())) {
                Some(Value::Bool(b)) => *b,
                Some(_) => return Err("'required' must be a boolean".to_string()),
                None => false,
            };
            let default = string_field(emap, "default")?;
            if let Some(d) = &default {
                if d.is_empty() {
                    return Err(format!("param '{pname}': 'default' must be non-empty"));
                }
            }
            if required && default.is_some() {
                return Err(format!(
                    "param '{pname}' cannot be both required and carry a default"
                ));
            }
            params.push(WrapperParam {
                name: pname,
                description,
                required,
                default,
            });
        }
    }
    let param_names: HashSet<&str> = params.iter().map(|p| p.name.as_str()).collect();

    // --- steps ---------------------------------------------------------------
    let steps_value = map
        .get(Value::String("steps".to_string()))
        .ok_or_else(|| "missing required key 'steps'".to_string())?;
    let steps_seq = steps_value
        .as_sequence()
        .ok_or_else(|| "'steps' must be a list".to_string())?;
    if steps_seq.is_empty() {
        return Err("'steps' must declare at least one step".to_string());
    }
    if steps_seq.len() > STEPS_MAX {
        return Err(format!("'steps' declares more than {STEPS_MAX} steps"));
    }

    let mut steps: Vec<WrapperStep> = Vec::new();
    let mut step_ids: HashSet<String> = HashSet::new();
    let mut output_names: HashSet<String> = HashSet::new();
    for (idx, entry) in steps_seq.iter().enumerate() {
        let smap = entry
            .as_mapping()
            .ok_or_else(|| "'steps' entries must be mappings".to_string())?;
        for key in smap.keys() {
            let key = key.as_str().unwrap_or("<non-string key>");
            if ![
                "id",
                "description",
                "use",
                "run",
                "with",
                "cwd",
                "timeout_secs",
                "output",
            ]
            .contains(&key)
            {
                return Err(format!("unknown step key '{key}'"));
            }
        }
        let id = string_field(smap, "id")?.ok_or_else(|| "every step needs an 'id'".to_string())?;
        if !is_valid_name(&id) {
            return Err(format!(
                "step id '{id}' is invalid (same rule as wrapper names)"
            ));
        }
        if !step_ids.insert(id.clone()) {
            return Err(format!("duplicate step id '{id}'"));
        }

        let description = string_field(smap, "description")?;
        let use_ = string_field(smap, "use")?;
        if let Some(u) = &use_ {
            if !is_valid_name(u) {
                return Err(format!("step '{id}': 'use' name '{u}' is invalid"));
            }
        }
        let run: Option<Vec<String>> = match smap.get(Value::String("run".to_string())) {
            Some(Value::Sequence(seq)) => {
                let argv: Vec<String> = seq
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_string)
                            .ok_or_else(|| "step 'run' entries must be strings".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                Some(argv)
            }
            Some(_) => return Err("'run' must be a list of strings".to_string()),
            None => None,
        };
        match (&use_, &run) {
            (Some(_), Some(_)) => {
                return Err(format!(
                    "step '{id}': 'use' and 'run' are mutually exclusive"
                ))
            }
            (None, None) => {
                return Err(format!("step '{id}': needs exactly one of 'use' or 'run'"))
            }
            _ => {}
        }
        if let Some(argv) = &run {
            if argv.is_empty() {
                return Err(format!("step '{id}': 'run' argv must not be empty"));
            }
            if argv.iter().any(|a| a.is_empty()) {
                return Err(format!(
                    "step '{id}': 'run' argv elements must be non-empty (found an empty element)"
                ));
            }
        }

        let with: Vec<(String, String)> = match smap.get(Value::String("with".to_string())) {
            Some(Value::Mapping(wmap)) => {
                if run.is_some() {
                    return Err(format!(
                        "step '{id}': 'with' belongs to 'use' steps (a 'run' step binds values directly into its argv)"
                    ));
                }
                let mut pairs = Vec::new();
                for (k, v) in wmap.iter() {
                    let k = k
                        .as_str()
                        .ok_or_else(|| "'with' keys must be strings".to_string())?;
                    let v = v
                        .as_str()
                        .ok_or_else(|| "'with' values must be strings".to_string())?;
                    if !is_valid_name(k) {
                        return Err(format!("step '{id}': 'with' key '{k}' is invalid"));
                    }
                    pairs.push((k.to_string(), v.to_string()));
                }
                pairs
            }
            Some(_) => return Err("'with' must be a mapping".to_string()),
            None => Vec::new(),
        };

        let cwd = string_field(smap, "cwd")?;
        if let Some(c) = &cwd {
            if c.is_empty() {
                return Err(format!("step '{id}': 'cwd' must be non-empty when present"));
            }
        }
        let timeout_secs = match smap.get(Value::String("timeout_secs".to_string())) {
            Some(Value::Number(n)) => {
                let secs = n
                    .as_u64()
                    .ok_or_else(|| "'timeout_secs' must be a positive integer".to_string())?;
                if secs == 0 {
                    return Err("'timeout_secs' must be greater than zero".to_string());
                }
                Some(secs)
            }
            Some(_) => return Err("'timeout_secs' must be a positive integer".to_string()),
            None => None,
        };
        let output = string_field(smap, "output")?;
        if let Some(o) = &output {
            if !is_valid_name(o) {
                return Err(format!("step '{id}': output name '{o}' is invalid"));
            }
            if !output_names.insert(o.clone()) {
                return Err(format!("output name '{o}' is bound by more than one step"));
            }
        }

        // Static binding validation: every placeholder in this step's
        // template strings must resolve NOW. Step outputs may only reference
        // EARLIER steps (data flows forward), and the referenced step must
        // actually declare an `output`.
        let mut templates: Vec<&str> = Vec::new();
        templates.extend(with.iter().map(|(_, v)| v.as_str()));
        if let Some(argv) = &run {
            templates.extend(argv.iter().map(|a| a.as_str()));
        }
        if let Some(c) = &cwd {
            templates.push(c.as_str());
        }
        for template in templates {
            for binding in scan_bindings(template)? {
                match &binding {
                    Binding::Param(p) => {
                        if !param_names.contains(p.as_str()) {
                            return Err(format!(
                                "step '{id}': {} references undeclared param '{p}'",
                                binding.to_placeholder()
                            ));
                        }
                    }
                    Binding::StepOutput(upstream) => {
                        let earlier = steps.iter().take(idx).find(|s| &s.id == upstream);
                        match earlier {
                            Some(s) if s.output.is_some() => {}
                            Some(_) => {
                                return Err(format!(
                                    "step '{id}': {} references step '{upstream}', which declares no output",
                                    binding.to_placeholder()
                                ))
                            }
                            None => {
                                return Err(format!(
                                    "step '{id}': {} references step '{upstream}', which is not an earlier step",
                                    binding.to_placeholder()
                                ))
                            }
                        }
                    }
                }
            }
        }

        steps.push(WrapperStep {
            id,
            description,
            use_,
            run,
            with,
            cwd,
            timeout_secs,
            output,
        });
    }

    // --- outputs -------------------------------------------------------------
    let mut outputs: Vec<WrapperOutput> = Vec::new();
    if let Some(entries) = map.get(Value::String("outputs".to_string())) {
        let seq = entries
            .as_sequence()
            .ok_or_else(|| "'outputs' must be a list".to_string())?;
        for entry in seq {
            let emap = entry
                .as_mapping()
                .ok_or_else(|| "'outputs' entries must be mappings".to_string())?;
            for key in emap.keys() {
                let key = key.as_str().unwrap_or("<non-string key>");
                if !["name", "description"].contains(&key) {
                    return Err(format!("unknown output key '{key}'"));
                }
            }
            let oname = string_field(emap, "name")?
                .ok_or_else(|| "'outputs' entry missing 'name'".to_string())?;
            if !is_valid_name(&oname) {
                return Err(format!("output name '{oname}' is invalid"));
            }
            if outputs.iter().any(|o| o.name == oname) {
                return Err(format!("duplicate output name '{oname}'"));
            }
            if !output_names.contains(&oname) {
                return Err(format!(
                    "output '{oname}' is not bound by any step (every declared output must be a step output binding)"
                ));
            }
            outputs.push(WrapperOutput {
                name: oname,
                description: string_field(emap, "description")?,
            });
        }
    }

    // --- preconditions ---------------------------------------------------------
    let preconditions = match get_list("preconditions") {
        Some(Ok(list)) => {
            if list.iter().any(|p| p.trim().is_empty()) {
                return Err("'preconditions' entries must be non-empty".to_string());
            }
            list
        }
        Some(Err(err)) => return Err(err),
        None => Vec::new(),
    };

    Ok(WrapperDef {
        name,
        description,
        model,
        behavior_prompt,
        approved,
        params,
        steps,
        outputs,
        preconditions,
        body: body.trim().to_string(),
        path: path.to_path_buf(),
    })
}

/// Names of the installed virtual tools (namespace-disjointness check).
/// Unreadable/missing tools dir ⇒ empty set (the wrapper side enforces what
/// it can see; the tool side stays wrapper-agnostic).
fn virtual_tool_names() -> HashSet<String> {
    let (tools, _) = crate::virtual_tools::loader::discover();
    tools.into_iter().map(|t| t.name).collect()
}

/// String field from a mapping, with a uniform type error.
fn string_field(map: &serde_yaml_ng::Mapping, key: &str) -> Result<Option<String>, String> {
    match map.get(Value::String(key.to_string())) {
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("'{key}' must be a string")),
        None => Ok(None),
    }
}

/// Scan one template string for `{{...}}` placeholders. Malformed or
/// unrecognized inner text is an error (fail closed — a typo'd binding must
/// not silently pass through to argv).
pub fn scan_bindings(template: &str) -> Result<Vec<Binding>, String> {
    let mut bindings = Vec::new();
    let mut rest = template;
    while let Some(start) = rest.find("{{") {
        let after = &rest[start + 2..];
        let Some(end) = after.find("}}") else {
            return Err(format!(
                "unbalanced placeholder in '{template}' (a '{{{{' without its '}}}}')"
            ));
        };
        let inner = &after[..end];
        let binding = if let Some(p) = inner.strip_prefix("params.") {
            Binding::Param(p.to_string())
        } else if let Some(s) = inner.strip_prefix("steps.") {
            let Some((sid, tail)) = s.rsplit_once(".output") else {
                return Err(format!(
                    "unrecognized placeholder '{{{{{inner}}}}}' (expected params.<name> or steps.<id>.output)"
                ));
            };
            if sid.is_empty() || !tail.is_empty() {
                return Err(format!(
                    "unrecognized placeholder '{{{{{inner}}}}}' (expected params.<name> or steps.<id>.output)"
                ));
            }
            Binding::StepOutput(sid.to_string())
        } else {
            return Err(format!(
                "unrecognized placeholder '{{{{{inner}}}}}' (expected params.<name> or steps.<id>.output)"
            ));
        };
        bindings.push(binding);
        rest = &after[end + 2..];
    }
    Ok(bindings)
}

/// Split a leading `---`-fenced YAML frontmatter from the markdown body
/// (same contract as the virtual-tool loader; comments are legal YAML, so
/// provenance stamps survive round-trips).
fn split_frontmatter(raw: &str, path: &Path) -> Result<(Value, String), String> {
    let nometa = format!("{}: ", path.display());
    let mut lines = raw.lines();
    if lines.next().map(str::trim_end) != Some("---") {
        return Err(format!("{nometa}must start with a '---' frontmatter fence"));
    }
    let mut front = Vec::new();
    let mut closed = false;
    for line in lines {
        if line.trim_end() == "---" {
            closed = true;
            break;
        }
        front.push(line);
    }
    if !closed {
        return Err(format!("{nometa}frontmatter fence is never closed"));
    }
    let body = {
        let mut consumed = 0usize;
        for (idx, line) in raw.lines().enumerate() {
            consumed += line.len() + 1;
            if idx > 0 && line.trim_end() == "---" {
                break;
            }
        }
        if consumed >= raw.len() {
            String::new()
        } else {
            raw[consumed..].to_string()
        }
    };
    let value: Value = serde_yaml_ng::from_str(&front.join("\n")).map_err(|err| {
        format!(
            "{nometa}frontmatter YAML did not parse: {}",
            one_line(&err.to_string())
        )
    })?;
    Ok((value, body))
}

/// Collapse whitespace to one line (error `<detail>` is one line).
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_tmp(name: &str, content: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join(format!("{name}.md"));
        std::fs::write(&path, content).expect("fixture");
        (dir, path)
    }

    const MINIMAL: &str = "---\n\
name: demo\ndescription: a demo\n\
steps:\n  - id: s1\n    run: [\"echo\", \"hi\"]\n\
---\nbody\n";

    #[test]
    fn minimal_wrapper_loads() {
        let (_d, path) = write_tmp("demo", MINIMAL);
        let def = load_from(&path).expect("loads");
        assert_eq!(def.name, "demo");
        assert_eq!(def.steps.len(), 1);
        assert_eq!(
            def.steps[0].run.as_deref(),
            Some(&["echo".to_string(), "hi".to_string()][..])
        );
        assert_eq!(def.body, "body");
        assert!(def.approved.is_none());
    }

    #[test]
    fn name_must_match_stem() {
        let (_d, path) = write_tmp("other", MINIMAL);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("does not match the filename stem"));
    }

    #[test]
    fn unknown_frontmatter_key_rejects() {
        let bad = MINIMAL.replace("description: a demo", "description: a demo\npostfix: x");
        let (_d, path) = write_tmp("demo", &bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("unknown frontmatter key 'postfix'"));
    }

    #[test]
    fn missing_steps_rejects() {
        let bad = "---\nname: demo\ndescription: d\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("missing required key 'steps'"));
    }

    #[test]
    fn empty_steps_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps: []\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path).unwrap_err().contains("at least one step"));
    }

    #[test]
    fn use_and_run_mutually_exclusive() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    use: cast-to-text\n    run: [\"ls\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path).unwrap_err().contains("mutually exclusive"));
    }

    #[test]
    fn step_needs_use_or_run() {
        let bad =
            "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    description: nothing\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("exactly one of 'use' or 'run'"));
    }

    #[test]
    fn duplicate_step_id_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"a\"]\n  - id: s1\n    run: [\"b\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("duplicate step id 's1'"));
    }

    #[test]
    fn run_argv_elements_must_be_non_empty() {
        let bad =
            "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"ls\", \"\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path).unwrap_err().contains("non-empty"));
    }

    #[test]
    fn with_rejected_on_run_step() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"ls\"]\n    with: {x: \"1\"}\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("'with' belongs to 'use' steps"));
    }

    #[test]
    fn unknown_param_binding_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\", \"{{params.nope}}\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("undeclared param 'nope'"));
    }

    #[test]
    fn forward_step_reference_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\", \"{{steps.s2.output}}\"]\n    output: o1\n  - id: s2\n    run: [\"echo\", \"x\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("not an earlier step"));
    }

    #[test]
    fn step_output_reference_needs_declared_output() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\", \"x\"]\n  - id: s2\n    run: [\"echo\", \"{{steps.s1.output}}\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path).unwrap_err().contains("declares no output"));
    }

    #[test]
    fn valid_step_output_chain_loads() {
        let good = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\", \"x\"]\n    output: mid\n  - id: s2\n    run: [\"echo\", \"{{steps.s1.output}}\"]\noutputs:\n  - name: mid\n---\n";
        let (_d, path) = write_tmp("demo", good);
        let def = load_from(&path).expect("loads");
        assert_eq!(def.outputs.len(), 1);
        assert_eq!(def.outputs[0].name, "mid");
    }

    #[test]
    fn dangling_output_declaration_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\", \"x\"]\noutputs:\n  - name: ghost\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("not bound by any step"));
    }

    #[test]
    fn required_param_with_default_rejects() {
        let bad = "---\nname: demo\ndescription: d\nparams:\n  - name: p\n    required: true\n    default: x\nsteps:\n  - id: s1\n    run: [\"echo\", \"{{params.p}}\"]\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path)
            .unwrap_err()
            .contains("required and carry a default"));
    }

    #[test]
    fn approved_must_be_rfc3339() {
        let bad = MINIMAL.replace(
            "description: a demo",
            "description: a demo\napproved: yesterday",
        );
        let (_d, path) = write_tmp("demo", &bad);
        assert!(load_from(&path).unwrap_err().contains("RFC 3339"));
        let good = MINIMAL.replace(
            "description: a demo",
            "description: a demo\napproved: 2026-11-09T14:30:00-08:00",
        );
        let (_d, path) = write_tmp("demo", &good);
        assert!(load_from(&path).expect("loads").approved.is_some());
    }

    #[test]
    fn reserved_name_rejects() {
        let bad =
            "---\nname: hooks\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n";
        let (_d, path) = write_tmp("hooks", bad);
        assert!(load_from(&path).unwrap_err().contains("reserved"));
    }

    #[test]
    fn namespace_collision_with_virtual_tool_rejects() {
        let (_d, path) = write_tmp(
            "clash",
            "---\nname: clash\ndescription: wrapper\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n",
        );
        let tools: HashSet<String> = ["clash".to_string()].into_iter().collect();
        assert!(load_with_tool_names(&path, &tools)
            .unwrap_err()
            .contains("virtual tool of the same name"));
        // A disjoint name loads fine against the same tool set.
        let (_d, path) = write_tmp(
            "distinct",
            "---\nname: distinct\ndescription: wrapper\nsteps:\n  - id: s1\n    run: [\"echo\"]\n---\n",
        );
        assert!(load_with_tool_names(&path, &tools).is_ok());
    }

    #[test]
    fn scan_bindings_forms() {
        assert_eq!(
            scan_bindings("{{params.x}}").expect("scan"),
            vec![Binding::Param("x".into())]
        );
        assert_eq!(
            scan_bindings("a {{steps.s1.output}} b {{params.y}}").expect("scan"),
            vec![Binding::StepOutput("s1".into()), Binding::Param("y".into())]
        );
        assert!(
            scan_bindings("{{ params.x }}").is_err(),
            "no spaces allowed"
        );
        assert!(scan_bindings("{{env.HOME}}").is_err(), "unknown source");
        assert!(scan_bindings("{{params.x").is_err(), "unbalanced");
        assert!(scan_bindings("plain").expect("scan").is_empty());
    }

    #[test]
    fn timeout_secs_zero_rejects() {
        let bad = "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"echo\"]\n    timeout_secs: 0\n---\n";
        let (_d, path) = write_tmp("demo", bad);
        assert!(load_from(&path).unwrap_err().contains("greater than zero"));
    }
}
