//! The deterministic path (SW-02): bind invocation params into a wrapper's
//! steps and render the ordered, resolved plan — **zero inference, zero
//! execution**. `smah` prints the plan and the exact `smah-run` invocation;
//! execution belongs to the companion `smah-run` crate (owner decision
//! 2026-11-09: smah is print-only; smah never invokes smah-run).
//!
//! Binding semantics (the loader already validated shapes at load time —
//! this pass applies *values*):
//! - every supplied `--param` must be declared ([`SmahError::WrapperParamUnknown`]);
//! - every required param must be supplied ([`SmahError::WrapperParamMissing`]);
//! - defaults apply when a param is absent;
//! - `{{params.<name>}}` substitutes anywhere inside a template string
//!   (embedded forms like `--file={{params.x}}.txt` work);
//! - `{{steps.<id>.output}}` stays **symbolic** in the printed plan — the
//!   value only exists at run time; `smah-run` substitutes it between steps.

use std::collections::BTreeMap;

use crate::errors::SmahError;
use crate::wrappers::approval::ApprovalStamp;
use crate::wrappers::builtin::{self, BuiltinTool};
use crate::wrappers::loader::{WrapperDef, WrapperOutput, WrapperStep};

/// Default per-step wall-clock budget when a step declares none (smah-run
/// enforces; surfaced in the plan so the user sees the budget up front).
pub const DEFAULT_STEP_TIMEOUT_SECS: u64 = 60;

/// One resolved step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedStep {
    pub id: String,
    pub description: Option<String>,
    pub kind: ResolvedStepKind,
    pub cwd: Option<String>,
    pub timeout_secs: u64,
    pub output: Option<String>,
}

/// What the step does once params are bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedStepKind {
    /// A registry built-in: display argv (fully param-substituted) plus the
    /// bound `with` map. `smah-run` executes it in-process via `run`.
    Builtin {
        tool_name: &'static str,
        display_argv: Vec<String>,
        bound: BTreeMap<String, String>,
    },
    /// A fixed argv (param-substituted; `{{steps.*.output}}` symbolic).
    /// No shell, ever — argv elements map 1:1 onto exec arguments.
    Run { argv: Vec<String> },
}

/// A fully resolved, ordered plan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPlan {
    pub name: String,
    pub description: String,
    /// The wrapper's approval stamp as loaded (integrity is verified at
    /// execution time by smah-run, not here — the plan printer is honest
    /// about what the file claims, not what it proves).
    pub approved: Option<ApprovalStamp>,
    pub steps: Vec<ResolvedStep>,
    pub outputs: Vec<WrapperOutput>,
    pub preconditions: Vec<String>,
    /// The exact `smah-run` line that executes this plan with these params
    /// (echoed at the bottom of the rendered plan).
    pub run_invocation: String,
}

/// Parse one `--param name=value` CLI token.
pub fn parse_param_token(token: &str) -> Result<(String, String), SmahError> {
    let (name, value) = token.split_once('=').ok_or_else(|| SmahError::Usage {
        message: format!(
            "smah: --param expects <name>=<value>, got '{token}'. Example: --param cast_file=session.cast."
        ),
    })?;
    if name.is_empty() || value.is_empty() {
        return Err(SmahError::Usage {
            message: format!("smah: --param expects non-empty <name>=<value>, got '{token}'."),
        });
    }
    Ok((name.to_string(), value.to_string()))
}

/// Resolve a wrapper against supplied params.
pub fn resolve(
    wrapper: &WrapperDef,
    supplied: &BTreeMap<String, String>,
) -> Result<ResolvedPlan, SmahError> {
    let declared: Vec<&str> = wrapper.params.iter().map(|p| p.name.as_str()).collect();

    // Unknown supplied params reject (typo footgun).
    for name in supplied.keys() {
        if !declared.contains(&name.as_str()) {
            return Err(SmahError::WrapperParamUnknown {
                param: name.clone(),
                wrapper: wrapper.name.clone(),
                declared: if declared.is_empty() {
                    "(none)".to_string()
                } else {
                    declared.join(", ")
                },
            });
        }
    }

    // Effective values: supplied > default > missing.
    let mut values: BTreeMap<String, String> = BTreeMap::new();
    let mut missing: Vec<&str> = Vec::new();
    for p in &wrapper.params {
        if let Some(v) = supplied.get(&p.name) {
            values.insert(p.name.clone(), v.clone());
        } else if let Some(d) = &p.default {
            values.insert(p.name.clone(), d.clone());
        } else if p.required {
            missing.push(&p.name);
        }
    }
    if !missing.is_empty() {
        return Err(SmahError::WrapperParamMissing {
            name: wrapper.name.clone(),
            params: missing.join(", "),
        });
    }

    let mut steps = Vec::new();
    for step in &wrapper.steps {
        steps.push(resolve_step(step, &values)?);
    }

    let run_invocation = build_run_invocation(&wrapper.name, supplied);
    Ok(ResolvedPlan {
        name: wrapper.name.clone(),
        description: wrapper.description.clone(),
        approved: wrapper.approved.clone(),
        steps,
        outputs: wrapper.outputs.clone(),
        preconditions: wrapper.preconditions.clone(),
        run_invocation,
    })
}

/// Resolve one step against the effective param values.
fn resolve_step(
    step: &WrapperStep,
    values: &BTreeMap<String, String>,
) -> Result<ResolvedStep, SmahError> {
    let unresolved = |detail: String| SmahError::WrapperResolve {
        name: step.id.clone(),
        detail,
    };
    let kind = if let Some(tool_name) = &step.use_ {
        let tool: &'static BuiltinTool = builtin::builtin(tool_name).ok_or_else(|| {
            unresolved(format!(
                "unknown built-in tool '{tool_name}' (registered: {})",
                builtin::builtins()
                    .iter()
                    .map(|b| b.name)
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        })?;
        // Substitute params into the with-values, then bind against the
        // tool's param specs (unknown keys / missing required reject).
        let substituted: Vec<(String, String)> = step
            .with
            .iter()
            .map(|(k, v)| Ok((k.clone(), subst_params(v, values)?)))
            .collect::<Result<_, _>>()
            .map_err(unresolved)?;
        let bound = tool.bind(&substituted).map_err(unresolved)?;
        let display_argv = (tool.argv_display)(&bound);
        ResolvedStepKind::Builtin {
            tool_name: tool.name,
            display_argv,
            bound,
        }
    } else {
        let argv = step
            .run
            .as_ref()
            .expect("loader guarantees use XOR run")
            .iter()
            .map(|a| subst_params(a, values))
            .collect::<Result<_, _>>()
            .map_err(unresolved)?;
        ResolvedStepKind::Run { argv }
    };
    let cwd = match &step.cwd {
        Some(c) => Some(subst_params(c, values).map_err(unresolved)?),
        None => None,
    };
    Ok(ResolvedStep {
        id: step.id.clone(),
        description: step.description.clone(),
        kind,
        cwd,
        timeout_secs: step.timeout_secs.unwrap_or(DEFAULT_STEP_TIMEOUT_SECS),
        output: step.output.clone(),
    })
}

/// Substitute `{{params.<name>}}` placeholders with their values. `{{steps.*}}`
/// placeholders pass through symbolically (they resolve at run time in
/// smah-run; the loader already validated their shape).
fn subst_params(template: &str, values: &BTreeMap<String, String>) -> Result<String, String> {
    let mut out = template.to_string();
    for (name, value) in values {
        out = out.replace(&format!("{{{{params.{name}}}}}"), value);
    }
    // Any *remaining* {{params.x}} means the loader validated a param the
    // resolver did not bind — impossible in the CLI flow, but fail loudly
    // rather than print a plan with live placeholders.
    if let Some(idx) = out.find("{{params.") {
        let rest = &out[idx..];
        if let Some(end) = rest.find("}}") {
            return Err(format!(
                "unresolved placeholder '{}' (param not in the effective set)",
                &rest[..end + 2]
            ));
        }
    }
    Ok(out)
}

/// The exact `smah-run` invocation for these supplied params (provided
/// params only — defaults live in the wrapper file and re-apply there).
fn build_run_invocation(name: &str, supplied: &BTreeMap<String, String>) -> String {
    let mut line = format!("smah-run {name}");
    for (k, v) in supplied {
        line.push_str(&format!(" --param {k}={v}"));
    }
    line
}

/// Render the plan for the terminal: ordered steps, risk notes, declared
/// outputs, and the exact `smah-run` line. Print-only — nothing here runs.
pub fn render_plan(plan: &ResolvedPlan) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "smah wrapper '{}' — {}\n",
        plan.name, plan.description
    ));
    match &plan.approved {
        Some(stamp) => out.push_str(&format!(
            "approval: approved {} — stamp binds content sha256 {}… (smah-run --execute re-verifies it; edits after approval refuse with exit 4)\n",
            stamp.at,
            &stamp.sha256[..stamp.sha256.len().min(12)]
        )),
        None => out.push_str(
            "approval: NOT approved — plan/dry-run only. Review this plan, then stamp it with `smah-run approve <name>` to enable `smah-run --execute`.\n",
        ),
    }
    if !plan.preconditions.is_empty() {
        out.push_str("preconditions:\n");
        for p in &plan.preconditions {
            out.push_str(&format!("  - {p}\n"));
        }
    }
    out.push_str("plan (nothing executes here — smah is print-only):\n");
    for (idx, step) in plan.steps.iter().enumerate() {
        out.push_str(&format!("  {}. {}\n", idx + 1, step_header(step)));
        match &step.kind {
            ResolvedStepKind::Builtin {
                display_argv,
                bound,
                ..
            } => {
                out.push_str(&format!("       argv: {}\n", shell_display(display_argv)));
                out.push_str(&format!(
                    "       risk: in-process pure-Rust built-in ({} file path(s): {})\n",
                    if bound.values().count() == 1 {
                        "reads"
                    } else {
                        "params"
                    },
                    bound.values().cloned().collect::<Vec<_>>().join(" ")
                ));
            }
            ResolvedStepKind::Run { argv } => {
                out.push_str(&format!("       argv: {}\n", shell_display(argv)));
                out.push_str(&format!(
                    "       risk: executes '{}' resolved via PATH with fixed argv (no shell); NOT run by smah\n",
                    argv.first().map(String::as_str).unwrap_or("?")
                ));
            }
        }
        if let Some(cwd) = &step.cwd {
            out.push_str(&format!("       cwd: {cwd}\n"));
        }
        out.push_str(&format!("       timeout: {}s\n", step.timeout_secs));
        if let Some(output) = &step.output {
            out.push_str(&format!("       output: {output}\n"));
        }
    }
    if !plan.outputs.is_empty() {
        out.push_str("outputs:\n");
        for o in &plan.outputs {
            match &o.description {
                Some(d) => out.push_str(&format!("  {} — {d}\n", o.name)),
                None => out.push_str(&format!("  {}\n", o.name)),
            }
        }
    }
    out.push_str(&format!(
        "execute with (review first — execution lives in smah-run, dry-run by default):\n  {}\n",
        plan.run_invocation
    ));
    out
}

/// One-line step header: `run <id> — description` / `use cast-to-text — …`.
fn step_header(step: &ResolvedStep) -> String {
    let kind = match &step.kind {
        ResolvedStepKind::Builtin { tool_name, .. } => format!("use {tool_name}"),
        ResolvedStepKind::Run { argv } => {
            format!("run {}", argv.first().map(String::as_str).unwrap_or("?"))
        }
    };
    match &step.description {
        Some(d) => format!("{kind} [{}] — {d}", step.id),
        None => format!("{kind} [{}]", step.id),
    }
}

/// Render an argv for display: elements joined with spaces, each element
/// quoted only when it contains whitespace (display fidelity — never a
/// re-parsable shell line; there is no shell).
fn shell_display(argv: &[String]) -> String {
    argv.iter()
        .map(|a| {
            if a.contains(char::is_whitespace) {
                format!("\"{a}\"")
            } else {
                a.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wrappers::loader::load_with_tool_names;
    use std::collections::HashSet;

    fn load_named(content: &str, stem: &str) -> WrapperDef {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join(format!("{stem}.md"));
        std::fs::write(&path, content).expect("fixture");
        let def = load_with_tool_names(&path, &HashSet::new()).expect("loads");
        std::mem::forget(dir);
        def
    }

    const WRAPPER: &str = "---\n\
name: demo\ndescription: demo recipe\n\
params:\n  - name: src\n    required: true\n  - name: label\n    default: transcript\n\
steps:\n  - id: convert\n    use: cast-to-text\n    with: {cast_file: \"{{params.src}}\"}\n    output: text\n  - id: show\n    run: [\"echo\", \"{{params.label}}:\", \"{{steps.convert.output}}\"]\n    timeout_secs: 5\noutputs:\n  - name: text\n---\n";

    fn values(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn resolves_params_defaults_and_symbolic_outputs() {
        let w = load_named(WRAPPER, "demo");
        let plan = resolve(&w, &values(&[("src", "session.cast")])).expect("resolves");
        assert_eq!(plan.steps.len(), 2);
        match &plan.steps[0].kind {
            ResolvedStepKind::Builtin {
                display_argv,
                bound,
                ..
            } => {
                assert_eq!(
                    display_argv,
                    &vec!["cast-to-text", "--cast-file", "session.cast"]
                );
                assert_eq!(
                    bound.get("cast_file").map(String::as_str),
                    Some("session.cast")
                );
            }
            other => panic!("expected builtin, got {other:?}"),
        }
        match &plan.steps[1].kind {
            ResolvedStepKind::Run { argv } => {
                assert_eq!(argv[0], "echo");
                assert_eq!(argv[1], "transcript:"); // default applied
                assert_eq!(argv[2], "{{steps.convert.output}}"); // symbolic
            }
            other => panic!("expected run, got {other:?}"),
        }
        assert_eq!(plan.steps[0].timeout_secs, DEFAULT_STEP_TIMEOUT_SECS);
        assert_eq!(plan.steps[1].timeout_secs, 5);
        assert_eq!(
            plan.run_invocation,
            "smah-run demo --param src=session.cast"
        );
    }

    #[test]
    fn unknown_supplied_param_rejects() {
        let w = load_named(WRAPPER, "demo");
        let err = resolve(&w, &values(&[("src", "a"), ("typo", "b")])).unwrap_err();
        assert!(matches!(err, SmahError::WrapperParamUnknown { .. }));
        assert!(err.render("log").contains("not declared"));
    }

    #[test]
    fn missing_required_param_rejects() {
        let w = load_named(WRAPPER, "demo");
        let err = resolve(&w, &values(&[])).unwrap_err();
        assert!(matches!(err, SmahError::WrapperParamMissing { .. }));
        assert!(err.render("log").contains("missing required param"));
    }

    #[test]
    fn unknown_builtin_rejects_at_resolve() {
        let w = load_named(
            "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    use: ghost-tool\n    with: {x: \"1\"}\n---\n",
            "demo",
        );
        let err = resolve(&w, &values(&[])).unwrap_err();
        assert!(matches!(err, SmahError::WrapperResolve { .. }));
        assert!(err
            .render("log")
            .contains("unknown built-in tool 'ghost-tool'"));
    }

    #[test]
    fn embedded_binding_substitutes() {
        let w = load_named(
            "---\nname: demo\ndescription: d\nparams:\n  - name: base\n    required: true\nsteps:\n  - id: s1\n    run: [\"cp\", \"{{params.base}}.src\", \"{{params.base}}.dst\"]\n---\n",
            "demo",
        );
        let plan = resolve(&w, &values(&[("base", "x")])).expect("resolves");
        match &plan.steps[0].kind {
            ResolvedStepKind::Run { argv } => {
                assert_eq!(argv, &vec!["cp", "x.src", "x.dst"]);
            }
            other => panic!("expected run, got {other:?}"),
        }
    }

    #[test]
    fn rendered_plan_names_risks_and_invocation() {
        let w = load_named(WRAPPER, "demo");
        let plan = resolve(&w, &values(&[("src", "session.cast")])).expect("resolves");
        let text = render_plan(&plan);
        assert!(text.contains("smah wrapper 'demo' — demo recipe"));
        assert!(text.contains("NOT approved"), "no approved: stamp: {text}");
        assert!(text.contains("1. use cast-to-text [convert]"));
        assert!(text.contains("argv: cast-to-text --cast-file session.cast"));
        assert!(text.contains("risk: executes 'echo' resolved via PATH"));
        assert!(text.contains("{{steps.convert.output}}"));
        assert!(text.contains("smah-run demo --param src=session.cast"));
        assert!(text.contains("outputs:"));
    }

    #[test]
    fn approved_wrapper_shows_approval_line() {
        let w = load_named(
            "---\nname: demo\ndescription: d\nsteps:\n  - id: s1\n    run: [\"true\"]\n---\n",
            "demo",
        );
        let mut stamped = w.clone();
        stamped.approved = Some(crate::wrappers::approval::stamp_for(
            &w,
            "2026-11-10T08:00:00-08:00",
        ));
        let plan = resolve(&stamped, &values(&[])).expect("resolves");
        let text = render_plan(&plan);
        assert!(
            text.contains("approved 2026-11-10T08:00:00-08:00"),
            "{text}"
        );
        assert!(text.contains("content sha256"), "{text}");
    }

    #[test]
    fn param_token_parsing() {
        assert_eq!(
            parse_param_token("a=b").expect("ok"),
            ("a".to_string(), "b".to_string())
        );
        assert!(parse_param_token("novalue").is_err());
        assert!(parse_param_token("=b").is_err());
        assert!(parse_param_token("a=").is_err());
    }

    #[test]
    fn shell_display_quotes_only_whitespace() {
        assert_eq!(
            shell_display(&["a b".to_string(), "c".to_string()]),
            "\"a b\" c"
        );
    }
}
