//! The `--prompt` path (SW-03): inference as a **recipe composer**. The
//! model never edits the file directly — it returns a structured changes
//! object (owner decision Q5 2026-11-09: `add_steps` / `modify_steps` /
//! `remove_steps` / `new_params` / `remove_params`, NOT RFC 6902 — path
//! patches are brittle for structural edits) which is validated, applied,
//! re-validated through the full loader, shown to the user as a readable
//! diff, and only then saved — with a provenance stamp (owner decision:
//! every inference-sourced change is traceable). Approval is NOT part of
//! this flow: accepting a diff is a recipe review, never an execution
//! sign-off (C1 — see `crate::wrappers::approval`).
//!
//! Caching (owner decision): keyed by `(wrapper_sha256, prompt_sha256)` —
//! a repeat prompt against an unchanged wrapper skips inference entirely.
//!
//! Model selection (owner decision Q4): the wrapper's `model:` field, else
//! the configured/spec default — resolved in the CLI layer, never a new
//! hardcoded id here.

use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_yaml_ng::Value;

use crate::wrappers::loader::{parse_wrapper, WrapperDef, WrapperParam, WrapperStep};

/// One model-proposed step (add or modify payload).
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq)]
pub struct StepSpec {
    pub id: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(rename = "use")]
    pub use_: Option<String>,
    #[serde(default)]
    pub run: Option<Vec<String>>,
    #[serde(default)]
    pub with: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    #[serde(default)]
    pub output: Option<String>,
}

/// One model-proposed param.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq)]
pub struct ParamSpec {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub required: Option<bool>,
    #[serde(default)]
    pub default: Option<String>,
}

/// One `add_steps` entry.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq)]
pub struct AddStep {
    /// Insert after this step id; absent/null = append at the end.
    #[serde(default)]
    pub after: Option<String>,
    #[serde(flatten)]
    pub step: StepSpec,
}

/// One `modify_steps` entry.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq)]
pub struct ModifyStep {
    pub id: String,
    #[serde(flatten)]
    pub set: StepSpec,
}

/// The structured changes object (owner decision Q5): the model answers with
/// `{"explanation": "...", "changes": {<actions>}}` — this is the wire shape
/// [`compose_system_prompt`] documents, and anything else is rejected.
#[derive(Debug, Clone, serde::Serialize, Deserialize, PartialEq)]
pub struct RecipeChanges {
    pub explanation: String,
    #[serde(rename = "changes", default)]
    pub actions: RecipeActions,
}

/// The action lists inside the changes object (all optional; an empty
/// `changes: {}` is a legal no-op answer).
#[derive(Debug, Clone, Default, serde::Serialize, Deserialize, PartialEq)]
pub struct RecipeActions {
    #[serde(default)]
    pub add_steps: Vec<AddStep>,
    #[serde(default)]
    pub modify_steps: Vec<ModifyStep>,
    #[serde(default)]
    pub remove_steps: Vec<String>,
    #[serde(default)]
    pub new_params: Vec<ParamSpec>,
    #[serde(default)]
    pub remove_params: Vec<String>,
}

/// Extract the changes JSON from a model answer: bare JSON, a fenced
/// ```json block, or JSON embedded in prose (first `{` to last `}`).
pub fn parse_changes(answer: &str) -> Result<RecipeChanges, String> {
    let trimmed = answer.trim();
    let candidates: Vec<String> = if let Some(json) = strip_json_fence(trimmed) {
        vec![json]
    } else {
        vec![
            trimmed.to_string(),
            substring_braced(trimmed).unwrap_or_default(),
        ]
    };
    let mut last_err = String::from("no JSON object found in the answer");
    for candidate in candidates {
        match serde_json::from_str::<RecipeChanges>(&candidate) {
            Ok(changes) => {
                if changes.explanation.trim().is_empty() {
                    return Err("'explanation' must be non-empty".to_string());
                }
                return Ok(changes);
            }
            Err(err) => last_err = one_line(&err.to_string()),
        }
    }
    Err(format!(
        "the answer is not a valid changes object: {last_err} (expected {{explanation, changes: {{add_steps, modify_steps, remove_steps, new_params, remove_params}}}})"
    ))
}

fn strip_json_fence(text: &str) -> Option<String> {
    let start = text.find("```")?;
    let after = &text[start + 3..];
    let after = after.strip_prefix("json").unwrap_or(after);
    let end = after.find("```")?;
    Some(after[..end].trim().to_string())
}

fn substring_braced(text: &str) -> Option<String> {
    let start = text.find('{')?;
    let end = text.rfind('}')?;
    (end > start).then(|| text[start..=end].to_string())
}

/// The system prompt for the composition turn.
pub fn compose_system_prompt(wrapper: &WrapperDef) -> String {
    let mut out = String::from(
        "You compose edits to smah script-wrapper recipes (ordered, executable step lists).\n\
You ALWAYS answer with a single JSON object and nothing else:\n\
{\n  \"explanation\": \"one short paragraph for the user\",\n  \"changes\": {\n    \"add_steps\": [{\"after\": \"<step-id or null>\", \"id\": \"...\", \"use\": \"<builtin>\" | \"run\": [\"argv\", ...], \"with\": {\"k\": \"v\"}, \"description\": \"...\", \"output\": \"...\", \"cwd\": \"...\", \"timeout_secs\": 30}],\n    \"modify_steps\": [{\"id\": \"...\", \"with\": {...}} | {\"id\": \"...\", \"run\": [...]}],\n    \"remove_steps\": [\"id\"],\n    \"new_params\": [{\"name\": \"...\", \"description\": \"...\", \"required\": false, \"default\": \"...\"}],\n    \"remove_params\": [\"name\"]\n  }\n}\n\
Rules: a step is exactly one of use (a registered built-in tool, with: binds its params) or run (a FIXED argv array — never a shell line, never sh -c); bindings are {{params.<name>}} or {{steps.<earlier-id>.output}} and must resolve; ids/param names are lowercase letters/digits/-/_ starting with a letter; omit empty change lists; propose the MINIMAL change that satisfies the request.\n",
    );
    if let Some(persona) = &wrapper.behavior_prompt {
        out.push_str("\nWrapper persona:\n");
        out.push_str(persona.trim());
        out.push('\n');
    }
    let names = crate::wrappers::builtin::builtins()
        .iter()
        .map(|b| b.name)
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("\nRegistered built-in tools: {names}.\n"));
    out
}

/// The user prompt: the current recipe + the NL request.
pub fn compose_user_prompt(wrapper: &WrapperDef, request: &str) -> String {
    let mut out = String::from("Current wrapper recipe (YAML frontmatter):\n---\n");
    out.push_str(&render_frontmatter(wrapper));
    out.push_str("---\n\n");
    if !wrapper.body.trim().is_empty() {
        out.push_str("Body:\n");
        out.push_str(wrapper.body.trim());
        out.push_str("\n\n");
    }
    out.push_str(&format!("User request: {request}\n"));
    out
}

/// Apply a changes object to a loaded wrapper. Returns the NEW wrapper
/// (re-parsed through the full loader — an applied diff that cannot load is
/// rejected here, never written) and its file content.
pub fn apply_changes(
    wrapper: &WrapperDef,
    changes: &RecipeChanges,
    tool_names: &HashSet<String>,
) -> Result<(WrapperDef, String), String> {
    let mut next = wrapper.clone();

    // Approval never survives a composition turn (C1): model output cannot
    // carry or create an `approved:` stamp — the composed recipe is
    // unapproved until the user explicitly stamps it with
    // `smah-run approve <wrapper>`.
    next.approved = None;

    // Params first (step bindings referencing new params must validate).
    for name in &changes.actions.remove_params {
        if !next.params.iter().any(|p| &p.name == name) {
            return Err(format!("remove_params: no param named '{name}'"));
        }
    }
    next.params
        .retain(|p| !changes.actions.remove_params.contains(&p.name));
    for spec in &changes.actions.new_params {
        if next.params.iter().any(|p| p.name == spec.name) {
            return Err(format!("new_params: param '{}' already exists", spec.name));
        }
        next.params.push(WrapperParam {
            name: spec.name.clone(),
            description: spec.description.clone(),
            required: spec.required.unwrap_or(false),
            default: spec.default.clone(),
            // Model-proposed params can NEVER opt into verbatim audit
            // (M1): `audit: plain` is a human-authored-file decision only.
            audit_plain: false,
        });
    }

    // Step removals.
    for id in &changes.actions.remove_steps {
        if !next.steps.iter().any(|s| &s.id == id) {
            return Err(format!("remove_steps: no step named '{id}'"));
        }
    }
    next.steps
        .retain(|s| !changes.actions.remove_steps.contains(&s.id));

    // Additions (after-position semantics; unknown anchor rejects).
    for add in &changes.actions.add_steps {
        let new_step = step_from_spec(&add.step)?;
        let pos = match &add.after {
            None => next.steps.len(),
            Some(anchor) => next
                .steps
                .iter()
                .position(|s| &s.id == anchor)
                .map(|p| p + 1)
                .ok_or_else(|| format!("add_steps: 'after' names unknown step '{anchor}'"))?,
        };
        if next.steps.iter().any(|s| s.id == new_step.id) {
            return Err(format!(
                "add_steps: step id '{}' already exists",
                new_step.id
            ));
        }
        next.steps.insert(pos, new_step);
    }

    // Modifications (field-level; use<->run shape changes go via remove+add).
    for m in &changes.actions.modify_steps {
        let step = next
            .steps
            .iter_mut()
            .find(|s| s.id == m.id)
            .ok_or_else(|| format!("modify_steps: no step named '{}'", m.id))?;
        if let Some(d) = &m.set.description {
            step.description = Some(d.clone());
        }
        if let Some(argv) = &m.set.run {
            if m.set.use_.is_some() {
                return Err(format!(
                    "modify_steps: step '{}' cannot switch between use and run (remove + re-add it instead)",
                    m.id
                ));
            }
            step.run = Some(argv.clone());
        }
        if m.set.use_.is_some() {
            return Err(format!(
                "modify_steps: step '{}' cannot switch between use and run (remove + re-add it instead)",
                m.id
            ));
        }
        if let Some(with) = &m.set.with {
            step.with = with.clone().into_iter().collect();
        }
        if let Some(cwd) = &m.set.cwd {
            step.cwd = Some(cwd.clone());
        }
        if let Some(secs) = m.set.timeout_secs {
            step.timeout_secs = Some(secs);
        }
    }

    // Serialize + full re-validation (bindings, ordering, outputs, names).
    let content = format!(
        "---\n{}\n---\n\n{}\n",
        render_frontmatter(&next).trim_end(),
        next.body
    );
    let reloaded = parse_wrapper(&content, &next.name, tool_names)?;
    Ok((reloaded, content))
}

/// Convert a model step spec into a [`WrapperStep`] (shape rules enforced;
/// deeper binding validation happens in the re-parse).
fn step_from_spec(spec: &StepSpec) -> Result<WrapperStep, String> {
    let id = spec
        .id
        .clone()
        .ok_or_else(|| "add_steps: a new step needs an 'id'".to_string())?;
    match (&spec.use_, &spec.run) {
        (Some(_), Some(_)) => Err(format!(
            "step '{id}': 'use' and 'run' are mutually exclusive"
        )),
        (None, None) => Err(format!("step '{id}': needs exactly one of 'use' or 'run'")),
        _ => Ok(WrapperStep {
            id,
            description: spec.description.clone(),
            use_: spec.use_.clone(),
            run: spec.run.clone(),
            with: spec
                .with
                .clone()
                .map(|m| m.into_iter().collect())
                .unwrap_or_default(),
            cwd: spec.cwd.clone(),
            timeout_secs: spec.timeout_secs,
            output: spec.output.clone(),
        }),
    }
}

/// Render wrapper frontmatter as YAML (canonical key order). `with` maps
/// and step fields emit in a stable order; provenance comments are handled
/// by [`render_with_provenance`], not here.
pub fn render_frontmatter(wrapper: &WrapperDef) -> String {
    let mut map = serde_yaml_ng::Mapping::new();
    map.insert(
        Value::String("name".into()),
        Value::String(wrapper.name.clone()),
    );
    map.insert(
        Value::String("description".into()),
        Value::String(wrapper.description.clone()),
    );
    if let Some(m) = &wrapper.model {
        map.insert(Value::String("model".into()), Value::String(m.clone()));
    }
    if let Some(p) = &wrapper.behavior_prompt {
        map.insert(
            Value::String("behavior_prompt".into()),
            Value::String(p.clone()),
        );
    }
    if let Some(a) = &wrapper.approved {
        // Content-bound stamp (C1): `{at, sha256}` mapping — the hash binds
        // the executable sections (params/steps/outputs), not just a time.
        let mut amap = serde_yaml_ng::Mapping::new();
        amap.insert(Value::String("at".into()), Value::String(a.at.clone()));
        amap.insert(
            Value::String("sha256".into()),
            Value::String(a.sha256.clone()),
        );
        map.insert(Value::String("approved".into()), Value::Mapping(amap));
    }
    if let Some(p) = params_value(&wrapper.params) {
        map.insert(Value::String("params".into()), p);
    }
    map.insert(Value::String("steps".into()), steps_value(&wrapper.steps));
    if let Some(o) = outputs_value(&wrapper.outputs) {
        map.insert(Value::String("outputs".into()), o);
    }
    if !wrapper.preconditions.is_empty() {
        map.insert(
            Value::String("preconditions".into()),
            Value::Sequence(
                wrapper
                    .preconditions
                    .iter()
                    .map(|p| Value::String(p.clone()))
                    .collect(),
            ),
        );
    }
    serde_yaml_ng::to_string(&Value::Mapping(map)).unwrap_or_default()
}

/// Canonical `steps:` section value (the shared renderer used by both the
/// file writer and the approval canonicalizer — one byte form, one hash).
pub(crate) fn steps_value(steps: &[WrapperStep]) -> Value {
    Value::Sequence(steps.iter().map(step_value).collect())
}

/// Canonical `params:` section value; `None` when no params are declared.
/// The per-param `audit: plain` opt-in is part of the canonical form.
pub(crate) fn params_value(params: &[WrapperParam]) -> Option<Value> {
    if params.is_empty() {
        return None;
    }
    Some(Value::Sequence(
        params
            .iter()
            .map(|p| {
                let mut m = serde_yaml_ng::Mapping::new();
                m.insert(Value::String("name".into()), Value::String(p.name.clone()));
                if let Some(d) = &p.description {
                    m.insert(
                        Value::String("description".into()),
                        Value::String(d.clone()),
                    );
                }
                if p.required {
                    m.insert(Value::String("required".into()), Value::Bool(true));
                }
                if let Some(d) = &p.default {
                    m.insert(Value::String("default".into()), Value::String(d.clone()));
                }
                if p.audit_plain {
                    m.insert(Value::String("audit".into()), Value::String("plain".into()));
                }
                Value::Mapping(m)
            })
            .collect(),
    ))
}

/// Canonical `outputs:` section value; `None` when no outputs are declared.
pub(crate) fn outputs_value(outputs: &[crate::wrappers::loader::WrapperOutput]) -> Option<Value> {
    if outputs.is_empty() {
        return None;
    }
    Some(Value::Sequence(
        outputs
            .iter()
            .map(|o| {
                let mut m = serde_yaml_ng::Mapping::new();
                m.insert(Value::String("name".into()), Value::String(o.name.clone()));
                if let Some(d) = &o.description {
                    m.insert(
                        Value::String("description".into()),
                        Value::String(d.clone()),
                    );
                }
                Value::Mapping(m)
            })
            .collect(),
    ))
}

fn step_value(step: &WrapperStep) -> Value {
    let mut m = serde_yaml_ng::Mapping::new();
    m.insert(Value::String("id".into()), Value::String(step.id.clone()));
    if let Some(d) = &step.description {
        m.insert(
            Value::String("description".into()),
            Value::String(d.clone()),
        );
    }
    if let Some(u) = &step.use_ {
        m.insert(Value::String("use".into()), Value::String(u.clone()));
    }
    if let Some(argv) = &step.run {
        m.insert(
            Value::String("run".into()),
            Value::Sequence(argv.iter().map(|a| Value::String(a.clone())).collect()),
        );
    }
    if !step.with.is_empty() {
        let mut w = serde_yaml_ng::Mapping::new();
        for (k, v) in &step.with {
            w.insert(Value::String(k.clone()), Value::String(v.clone()));
        }
        m.insert(Value::String("with".into()), Value::Mapping(w));
    }
    if let Some(c) = &step.cwd {
        m.insert(Value::String("cwd".into()), Value::String(c.clone()));
    }
    if let Some(secs) = step.timeout_secs {
        m.insert(
            Value::String("timeout_secs".into()),
            Value::Number(secs.into()),
        );
    }
    if let Some(o) = &step.output {
        m.insert(Value::String("output".into()), Value::String(o.clone()));
    }
    Value::Mapping(m)
}

/// Render the full wrapper file (frontmatter + body).
pub fn render_wrapper_file(wrapper: &WrapperDef) -> String {
    format!(
        "---\n{}\n---\n\n{}\n",
        render_frontmatter(wrapper).trim_end(),
        wrapper.body
    )
}

/// `# provenance:` comment lines carried by a wrapper file (oldest first).
pub fn extract_provenance(raw: &str) -> Vec<String> {
    raw.lines()
        .filter(|l| l.trim_start().starts_with("# provenance:"))
        .map(|l| l.trim().to_string())
        .collect()
}

/// Insert provenance lines right after the opening `---` fence.
pub fn render_with_provenance(content: &str, provenance: &[String]) -> String {
    let mut lines = content.lines();
    let mut out = String::new();
    if lines.next().map(str::trim_end) == Some("---") {
        out.push_str("---\n");
        for p in provenance {
            out.push_str(p);
            out.push('\n');
        }
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
        out
    } else {
        content.to_string()
    }
}

/// Build one provenance stamp line.
pub fn provenance_stamp(
    ts_rfc3339: &str,
    model: &str,
    run_id: &str,
    prompt_sha256: &str,
) -> String {
    format!("# provenance: {ts_rfc3339} model={model} run={run_id} prompt_sha256={prompt_sha256}")
}

/// Local time, RFC 3339 (repo convention) — approval/provenance stamps.
pub fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

/// sha256 hex (cache keys).
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(data);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// One cached composition result.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq)]
pub struct PromptCacheEntry {
    pub wrapper_sha256: String,
    pub prompt_sha256: String,
    pub model: String,
    pub ts: String,
    /// The raw changes JSON (re-parsed + re-validated on every hit — a
    /// poisoned entry is dropped loudly, never served blind).
    pub changes_json: String,
}

/// The (wrapper, prompt) → changes cache for one wrapper.
pub struct PromptCache {
    path: PathBuf,
    pub entries: Vec<PromptCacheEntry>,
}

impl PromptCache {
    /// Load `~/.cache/smah/wrappers/<name>/prompt-cache.json` (honors
    /// XDG_CACHE_HOME; missing file = empty cache).
    pub fn load(name: &str) -> Self {
        let path = crate::virtual_tools::audit::cache_dir()
            .join("wrappers")
            .join(name)
            .join("prompt-cache.json");
        Self::load_at(&path)
    }

    /// Load from an explicit path (tests).
    pub fn load_at(path: &Path) -> Self {
        let entries = std::fs::read_to_string(path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        PromptCache {
            path: path.to_path_buf(),
            entries,
        }
    }

    /// Look up a (wrapper_sha256, prompt_sha256) pair. Returns the parsed
    /// changes plus the model that composed them on a hit; an entry whose
    /// JSON no longer parses as valid changes is removed from memory and
    /// reported as an error string (caller warns and treats it as a miss).
    pub fn lookup(
        &mut self,
        wrapper_sha256: &str,
        prompt_sha256: &str,
    ) -> Result<Option<(RecipeChanges, String)>, String> {
        let idx = self
            .entries
            .iter()
            .position(|e| e.wrapper_sha256 == wrapper_sha256 && e.prompt_sha256 == prompt_sha256);
        let Some(idx) = idx else {
            return Ok(None);
        };
        match parse_changes(&self.entries[idx].changes_json) {
            Ok(changes) => Ok(Some((changes, self.entries[idx].model.clone()))),
            Err(err) => {
                self.entries.remove(idx);
                Err(format!(
                    "cached entry failed validation ({err}) — dropped; recomposing"
                ))
            }
        }
    }

    /// Record an entry (replacing any same-key predecessor) and persist
    /// best-effort.
    pub fn record(&mut self, entry: PromptCacheEntry) -> Result<(), String> {
        self.entries.retain(|e| {
            !(e.wrapper_sha256 == entry.wrapper_sha256 && e.prompt_sha256 == entry.prompt_sha256)
        });
        self.entries.push(entry);
        self.save()
    }

    fn save(&self) -> Result<(), String> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| err.to_string())?;
        }
        let json = serde_json::to_string_pretty(&self.entries).map_err(|err| err.to_string())?;
        std::fs::write(&self.path, json).map_err(|err| err.to_string())
    }
}

/// Render the human-readable diff summary for the review flow.
pub fn render_changes_summary(changes: &RecipeChanges) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "proposed change: {}\n",
        one_line(&changes.explanation)
    ));
    for add in &changes.actions.add_steps {
        let kind = if let Some(u) = &add.step.use_ {
            format!("use {u}")
        } else {
            "run".to_string()
        };
        let id = add.step.id.clone().unwrap_or_default();
        let after = add.after.clone().unwrap_or_else(|| "(end)".to_string());
        out.push_str(&format!("  + step {id} ({kind}) after {after}\n"));
    }
    for m in &changes.actions.modify_steps {
        let mut fields: Vec<&str> = Vec::new();
        if m.set.description.is_some() {
            fields.push("description");
        }
        if m.set.run.is_some() {
            fields.push("run");
        }
        if m.set.with.is_some() {
            fields.push("with");
        }
        if m.set.cwd.is_some() {
            fields.push("cwd");
        }
        if m.set.timeout_secs.is_some() {
            fields.push("timeout_secs");
        }
        out.push_str(&format!(
            "  ~ step {}: {}\n",
            m.id,
            if fields.is_empty() {
                "(no-op)".to_string()
            } else {
                fields.join(", ")
            }
        ));
    }
    for id in &changes.actions.remove_steps {
        out.push_str(&format!("  - step {id}\n"));
    }
    for p in &changes.actions.new_params {
        out.push_str(&format!(
            "  + param {}{}{}\n",
            p.name,
            if p.required.unwrap_or(false) {
                " (required)"
            } else {
                ""
            },
            p.default
                .as_ref()
                .map(|d| format!(" default={d}"))
                .unwrap_or_default()
        ));
    }
    for name in &changes.actions.remove_params {
        out.push_str(&format!("  - param {name}\n"));
    }
    out
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// The installed **virtual-tool** name set for re-validation (the
/// disjoint-namespace check a generated wrapper must still pass).
pub fn current_tool_names() -> HashSet<String> {
    let (tools, _) = crate::virtual_tools::loader::discover();
    tools.into_iter().map(|t| t.name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrappers::loader;

    const WRAPPER: &str = "---\n\
name: demo\ndescription: demo recipe\n\
params:\n  - name: src\n    required: true\n\
steps:\n  - id: first\n    run: [\"echo\", \"{{params.src}}\"]\n    output: text\n  - id: second\n    run: [\"wc\", \"-l\"]\noutputs:\n  - name: text\n---\nbody text\n";

    fn loaded() -> WrapperDef {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("demo.md");
        std::fs::write(&path, WRAPPER).expect("fixture");
        // Hand the tempdir to the test (kept, not deleted): the file must
        // outlive the load call.
        let _keep = dir.keep();
        loader::load_with_tool_names(&path, &HashSet::new()).expect("loads")
    }

    fn changes(json: &str) -> RecipeChanges {
        parse_changes(json).expect("valid changes")
    }

    #[test]
    fn parse_changes_bare_fenced_and_embedded() {
        let bare = r#"{"explanation":"e","changes":{"add_steps":[{"after":"first","id":"third","run":["ls"],"output":"listing"}]}}"#;
        assert!(parse_changes(bare).is_ok());
        let fenced = format!("Here you go:\n```json\n{bare}\n```\nDone!");
        assert!(parse_changes(&fenced).is_ok());
        let embedded = "Sure! {\"explanation\":\"e\",\"changes\":{}} — hope that helps".to_string();
        assert!(parse_changes(&embedded).is_ok());
        assert!(parse_changes("no json at all").is_err());
        assert!(
            parse_changes("{\"changes\":{}}").is_err(),
            "explanation required"
        );
    }

    #[test]
    fn apply_add_modify_remove_round_trips() {
        let w = loaded();
        let c = changes(
            r#"{"explanation":"rework","changes":{
                "add_steps":[{"after":"first","id":"label","run":["echo","step-count:"]}],
                "modify_steps":[{"id":"second","run":["wc","-c"],"timeout_secs":30}],
                "remove_steps":[],
                "new_params":[{"name":"verbose","required":false,"default":"no"}],
                "remove_params":[]}}"#,
        );
        let (next, content) = apply_changes(&w, &c, &HashSet::new()).expect("applies");
        assert_eq!(next.steps.len(), 3);
        assert_eq!(next.steps[1].id, "label"); // after "first"
        assert_eq!(
            next.steps[2].run,
            Some(vec!["wc".to_string(), "-c".to_string()])
        );
        assert_eq!(next.steps[2].timeout_secs, Some(30));
        assert_eq!(next.params.len(), 2);
        // Content re-parses identically through the loader.
        let again = parse_wrapper(&content, "demo", &HashSet::new()).expect("reparses");
        assert_eq!(again, next);
    }

    #[test]
    fn apply_changes_strips_approval_never_carries_or_adds() {
        // C1: an approved wrapper composed through --prompt yields an
        // UNAPPROVED recipe — the composed file carries no stamp even when
        // the source was approved (and model output cannot add one: the
        // changes object has no approval channel).
        let mut def = loaded();
        let stamp = crate::wrappers::approval::stamp_for(&def, "2026-11-10T08:00:00-08:00");
        def.approved = Some(stamp);
        let noop = changes(r#"{"explanation":"no-op","changes":{}}"#);
        let (next, content) = apply_changes(&def, &noop, &HashSet::new()).expect("applies");
        assert!(next.approved.is_none(), "approval must be stripped");
        assert!(
            !content.contains("approved:"),
            "composed file must not carry a stamp: {content}"
        );
    }

    #[test]
    fn apply_rejects_unknown_anchor_and_duplicate_ids() {
        let w = loaded();
        let bad_anchor = changes(
            r#"{"explanation":"e","changes":{"add_steps":[{"after":"ghost","id":"x","run":["ls"]}]}}"#,
        );
        assert!(apply_changes(&w, &bad_anchor, &HashSet::new())
            .unwrap_err()
            .contains("unknown step 'ghost'"));
        let dup =
            changes(r#"{"explanation":"e","changes":{"add_steps":[{"id":"first","run":["ls"]}]}}"#);
        assert!(apply_changes(&w, &dup, &HashSet::new())
            .unwrap_err()
            .contains("already exists"));
    }

    #[test]
    fn apply_rejects_broken_resulting_recipe() {
        let w = loaded();
        // Remove a step whose output a remaining step binds → re-validation
        // must reject before anything is written.
        let c = changes(r#"{"explanation":"drop first","changes":{"remove_steps":["first"]}}"#);
        assert!(apply_changes(&w, &c, &HashSet::new()).is_err());
    }

    #[test]
    fn modify_cannot_switch_use_and_run() {
        let w = loaded();
        let c = changes(
            r#"{"explanation":"e","changes":{"modify_steps":[{"id":"second","use":"cast-to-text"}]}}"#,
        );
        assert!(apply_changes(&w, &c, &HashSet::new())
            .unwrap_err()
            .contains("cannot switch"));
    }

    #[test]
    fn provenance_lines_round_trip() {
        let stamped = render_with_provenance(
            WRAPPER,
            &[
                "# provenance: 2026-11-09T10:00:00-08:00 model=m run=r prompt_sha256=ab"
                    .to_string(),
            ],
        );
        assert!(stamped.starts_with("---\n# provenance:"));
        let extracted = extract_provenance(&stamped);
        assert_eq!(extracted.len(), 1);
        // And the stamped file still loads.
        assert!(parse_wrapper(&stamped, "demo", &HashSet::new()).is_ok());
    }

    #[test]
    fn cache_round_trip_hit_and_poison() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("prompt-cache.json");
        let mut cache = PromptCache::load_at(&path);
        assert!(cache.lookup("wsha", "psha").expect("empty").is_none());
        let json = r#"{"explanation":"e","changes":{"new_params":[{"name":"extra"}]}}"#;
        cache
            .record(PromptCacheEntry {
                wrapper_sha256: "wsha".into(),
                prompt_sha256: "psha".into(),
                model: "test-model".into(),
                ts: "2026-11-09T10:00:00-08:00".into(),
                changes_json: json.to_string(),
            })
            .expect("saves");
        let reloaded = PromptCache::load_at(&path);
        assert_eq!(reloaded.entries.len(), 1);
        let mut hit = reloaded;
        assert!(hit.lookup("wsha", "psha").expect("hit").is_some());
        // Poisoned entry: dropped loudly, treated as miss.
        let mut poisoned = PromptCache::load_at(&path);
        poisoned.entries[0].changes_json = "{not json".into();
        let err = poisoned.lookup("wsha", "psha").unwrap_err();
        assert!(err.contains("failed validation"));
    }

    #[test]
    fn summary_renders_each_change_kind() {
        let c = changes(
            r#"{"explanation":"big rework","changes":{
                "add_steps":[{"after":null,"id":"third","use":"cast-to-text","with":{"cast_file":"{{params.src}}"}}],
                "modify_steps":[{"id":"second","run":["true"]}],
                "remove_steps":["first"],
                "new_params":[{"name":"verbose","required":true}],
                "remove_params":["src"]}}"#,
        );
        let text = render_changes_summary(&c);
        assert!(text.contains("big rework"));
        assert!(text.contains("+ step third (use cast-to-text) after (end)"));
        assert!(text.contains("~ step second: run"));
        assert!(text.contains("- step first"));
        assert!(text.contains("+ param verbose (required)"));
        assert!(text.contains("- param src"));
    }

    #[test]
    fn prompts_embed_recipe_and_request() {
        let w = loaded();
        let system = compose_system_prompt(&w);
        assert!(system.contains("JSON object"));
        assert!(system.contains("cast-to-text"));
        let user = compose_user_prompt(&w, "add a caption step");
        assert!(user.contains("name: demo"));
        assert!(user.contains("User request: add a caption step"));
    }
}
