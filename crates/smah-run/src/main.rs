//! `smah-run` — the companion **executor** for saved script wrappers
//! (SW-05). This is the only workspace crate allowed to spawn processes:
//! the CI no-shell gate exempts exactly `crates/smah-run/src` (see
//! `scripts/no-shell-gate.sh`, which self-tests both directions), and
//! `smah` itself never invokes this binary — it only prints the exact
//! command line.
//!
//! Contract (owner decisions 2026-11-09):
//!
//! - executes only **saved, user-approved** wrapper recipes: `--execute`
//!   requires an `approved:` stamp; there is no stdin recipe, no composing,
//!   no inference here;
//! - **dry-run by default** — prints the resolved plan exactly like
//!   `smah --wrapper <name>`; `--execute` is the explicit opt-in;
//! - `run:` steps spawn with a **fixed argv** (elements map 1:1 onto exec
//!   arguments; no shell, no interpolation, ever); built-in `use:` steps
//!   execute in-process via the registry (pure Rust, no exec);
//! - **per-step confirmation** (yes / skip / abort) on a TTY unless `--yes`;
//!   `--execute` on a non-TTY without `--yes` refuses before running
//!   anything — never a silent unattended run;
//! - **per-step timeout**: the step's `timeout_secs`, else `--timeout`
//!   (default 60s) — a timed-out step is killed and the run fails;
//! - append-only **audit JSONL** at `~/.local/state/smah/audit-smah-run.jsonl`
//!   (honors `XDG_STATE_HOME`), 0700/0600, **metadata only**: argv is
//!   run through the registered-secret redactor and the values of
//!   secret-ish param names are masked before anything is written.
//!
//! Exit codes (mirrors the smah six-class table where it applies):
//! 0 success · 2 usage · 3 config (invalid or unapproved wrapper) ·
//! 1 a step failed, timed out, or the run aborted.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use serde::Serialize;

use smah_core::logging::redact;
use smah_core::wrappers::builtin;
use smah_core::wrappers::loader;
use smah_core::wrappers::resolve::{self, ResolvedPlan, ResolvedStep, ResolvedStepKind};

/// Captured-output cap per step (bytes). Over-cap output is truncated for
/// binding purposes; the step still succeeds (a transcript is data, not a
/// threat — but a runaway step must not eat RAM).
const OUTPUT_CAPTURE_MAX: usize = 1024 * 1024;

/// `smah-run <name> [--param k=v]... [--execute] [--yes] [--timeout SECS]`.
#[derive(Parser, Debug)]
#[command(
    name = "smah-run",
    version,
    about = "Execute a saved, approved smah script wrapper (dry-run by default)."
)]
struct Cli {
    /// Wrapper name (a file in the wrappers dir, e.g. `session-transcript`).
    name: String,

    /// Invocation param, repeatable: `--param <name>=<value>`.
    #[arg(long = "param", value_name = "NAME=VALUE", action = clap::ArgAction::Append)]
    param: Vec<String>,

    /// Actually execute (default: print the resolved plan and exit).
    #[arg(long)]
    execute: bool,

    /// Answer yes to every per-step confirmation (required for non-TTY
    /// --execute).
    #[arg(long)]
    yes: bool,
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let cli = Cli::parse();

    let wrapper = match loader::load_by_name(&cli.name) {
        Ok(w) => w,
        Err(reason) if reason.starts_with("not found:") => {
            eprintln!(
                "smah-run: no wrapper named '{}'. 'smah wrapper list' shows what is installed.",
                cli.name
            );
            return 2;
        }
        Err(reason) => {
            eprintln!("smah-run: wrapper '{}' is invalid: {}", cli.name, reason);
            return 3;
        }
    };

    let mut supplied: BTreeMap<String, String> = BTreeMap::new();
    for token in &cli.param {
        match resolve::parse_param_token(token) {
            Ok((k, v)) => {
                supplied.insert(k, v);
            }
            Err(err) => {
                eprintln!("{}", err.render("smah.log"));
                return err.exit_code().code();
            }
        }
    }

    let plan = match resolve::resolve(&wrapper, &supplied) {
        Ok(p) => p,
        Err(err) => {
            eprintln!("{}", err.render("smah.log"));
            return err.exit_code().code();
        }
    };

    if !cli.execute {
        print!("{}", resolve::render_plan(&plan));
        println!();
        println!(
            "(dry-run — pass --execute to run; execution requires an 'approved:' stamp on the wrapper)"
        );
        return 0;
    }

    // Guard 1: approval. Guard 2: interactivity. Both BEFORE anything runs.
    if wrapper.approved.is_none() {
        eprintln!(
            "smah-run: wrapper '{}' carries no 'approved:' stamp — execution refused. \
Review the plan ('smah --wrapper {}'), accept a --prompt diff, or add the stamp by hand.",
            cli.name, cli.name
        );
        return 3;
    }
    if !cli.yes && !std::io::stdin().is_terminal() {
        eprintln!(
            "smah-run: --execute on a non-TTY needs --yes (no interactive confirmation possible); refusing."
        );
        return 2;
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let audit = RunAudit::open();
    execute_plan(&plan, &supplied, cli.yes, &run_id, &audit)
}

/// One audit JSONL record — metadata only.
#[derive(Serialize)]
struct StepRecord {
    ts: String,
    run: String,
    wrapper: String,
    step: String,
    kind: &'static str,
    /// Redacted, space-joined argv (or the built-in display argv).
    argv: String,
    /// executed | skipped | aborted | failed | refused.
    decision: &'static str,
    /// ok | exit N | timeout | error: ... | skipped | aborted.
    status: String,
    #[serde(rename = "millis")]
    duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout_bytes: Option<usize>,
}

/// Append-only audit sink (`~/.local/state/smah/audit-smah-run.jsonl`,
/// 0700 dir / 0600 file, best-effort — an unusable audit warns, never
/// blocks).
struct RunAudit {
    path: PathBuf,
    disabled: Option<String>,
}

impl RunAudit {
    fn open() -> Self {
        let path = smah_core::virtual_tools::audit::state_dir().join("audit-smah-run.jsonl");
        let mut disabled = None;
        if let Some(parent) = path.parent() {
            if let Err(err) = std::fs::create_dir_all(parent) {
                disabled = Some(format!("cannot create {}: {err}", parent.display()));
            } else {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = std::fs::metadata(parent) {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o700);
                        let _ = std::fs::set_permissions(parent, perms);
                    }
                }
            }
        }
        RunAudit { path, disabled }
    }

    fn record(&self, rec: &StepRecord) {
        if let Some(reason) = &self.disabled {
            eprintln!("smah-run: audit unavailable: {reason}");
            return;
        }
        let line = serde_json::to_string(rec).unwrap_or_else(|_| "{}".to_string());
        let append = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .and_then(|mut file| {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    if let Ok(meta) = file.metadata() {
                        let mut perms = meta.permissions();
                        perms.set_mode(0o600);
                        let _ = file.set_permissions(perms);
                    }
                }
                use std::io::Write;
                writeln!(file, "{line}")
            });
        if let Err(err) = append {
            eprintln!("smah-run: audit write failed: {err}");
        }
    }
}

/// Redact an argv for the audit log: registered secrets via the core
/// redactor, plus any value whose supplying param name looks secret-ish.
fn redact_argv(argv: &[String], supplied: &BTreeMap<String, String>) -> String {
    let mut joined = argv.join(" ");
    for (name, value) in supplied {
        if name_looks_secret(name) && !value.is_empty() {
            joined = joined.replace(value.as_str(), "***");
        }
    }
    redact(&joined)
}

/// Cheap heuristic on the PARAM NAME (never the value): token/password/
/// secret/key/api/credential/auth.
fn name_looks_secret(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    [
        "token",
        "password",
        "secret",
        "key",
        "api",
        "credential",
        "auth",
    ]
    .iter()
    .any(|frag| n.contains(frag))
}

/// Substitute `{{steps.<id>.output}}` bindings using the outputs produced
/// so far. Any placeholder that cannot resolve (its step was skipped) is an
/// error — never an empty-string silent pass-through.
fn substitute_step_outputs(
    template: &str,
    outputs: &BTreeMap<String, String>,
) -> Result<String, String> {
    let mut out = template.to_string();
    for (id, text) in outputs {
        out = out.replace(&format!("{{{{steps.{id}.output}}}}"), text);
    }
    if let Some(start) = out.find("{{steps.") {
        let _ = start;
        return Err(format!(
            "unresolved binding in '{template}' (its upstream step did not produce output)"
        ));
    }
    Ok(out)
}

/// Interactive per-step confirmation. Returns true to run, false to skip;
/// `abort` propagates as an error.
fn confirm_step(step: &ResolvedStep) -> Result<bool, String> {
    use std::io::BufRead;
    loop {
        println!();
        print!("run step '{}'? [y]es / [s]kip / [a]bort: ", step.id);
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let stdin = std::io::stdin();
        let line = match stdin.lock().lines().next() {
            Some(Ok(line)) => line,
            _ => return Err("stdin closed — aborting".to_string()),
        };
        match line.trim().to_ascii_lowercase().as_str() {
            "y" | "yes" | "" => return Ok(true),
            "s" | "skip" => return Ok(false),
            "a" | "abort" | "q" | "n" | "no" => return Err("aborted by the user".to_string()),
            other => println!("unknown answer '{other}' — [y]es / [s]kip / [a]bort."),
        }
    }
}

fn execute_plan(
    plan: &ResolvedPlan,
    supplied: &BTreeMap<String, String>,
    yes: bool,
    run_id: &str,
    audit: &RunAudit,
) -> i32 {
    // Two key spaces: step-id keyed for `{{steps.<id>.output}}` substitution,
    // output-name keyed for the final `outputs:` printing.
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    let mut produced: BTreeMap<String, String> = BTreeMap::new();
    println!(
        "smah-run: executing wrapper '{}' ({} steps, approved {})",
        plan.name,
        plan.steps.len(),
        plan.approved.as_deref().unwrap_or("?")
    );

    for step in &plan.steps {
        let kind_label: &str = match &step.kind {
            ResolvedStepKind::Builtin { .. } => "builtin",
            ResolvedStepKind::Run { .. } => "run",
        };
        let display_argv: Vec<String> = match &step.kind {
            ResolvedStepKind::Builtin { display_argv, .. } => display_argv.clone(),
            ResolvedStepKind::Run { argv } => argv.clone(),
        };

        // Confirmation (unless --yes).
        if !yes {
            println!();
            println!(
                "step '{}' ({kind_label}): {}",
                step.id,
                display_argv.join(" ")
            );
            match confirm_step(step) {
                Ok(true) => {}
                Ok(false) => {
                    audit.record(&StepRecord {
                        ts: now_rfc3339(),
                        run: run_id.to_string(),
                        wrapper: plan.name.clone(),
                        step: step.id.clone(),
                        kind: kind_label,
                        argv: redact_argv(&display_argv, supplied),
                        decision: "skipped",
                        status: "skipped".to_string(),
                        duration_ms: 0,
                        stdout_bytes: None,
                    });
                    println!("smah-run: step '{}' skipped.", step.id);
                    continue;
                }
                Err(reason) => {
                    audit.record(&StepRecord {
                        ts: now_rfc3339(),
                        run: run_id.to_string(),
                        wrapper: plan.name.clone(),
                        step: step.id.clone(),
                        kind: kind_label,
                        argv: redact_argv(&display_argv, supplied),
                        decision: "aborted",
                        status: "aborted".to_string(),
                        duration_ms: 0,
                        stdout_bytes: None,
                    });
                    eprintln!("smah-run: {reason} (before step '{}').", step.id);
                    return 1;
                }
            }
        }

        let started = Instant::now();
        let outcome = execute_step(step, &outputs);
        let duration_ms = started.elapsed().as_millis();
        let record = |decision: &'static str, status: String, bytes: Option<usize>| StepRecord {
            ts: now_rfc3339(),
            run: run_id.to_string(),
            wrapper: plan.name.clone(),
            step: step.id.clone(),
            kind: kind_label,
            argv: redact_argv(&display_argv, supplied),
            decision,
            status,
            duration_ms,
            stdout_bytes: bytes,
        };

        match outcome {
            Ok(Some(text)) => {
                let bytes = Some(text.len());
                audit.record(&record("executed", "ok".to_string(), bytes));
                if let Some(name) = &step.output {
                    outputs.insert(step.id.clone(), text.clone());
                    produced.insert(name.clone(), text);
                }
            }
            Ok(None) => {
                audit.record(&record("executed", "ok".to_string(), None));
            }
            Err(reason) => {
                audit.record(&record("failed", reason.clone(), None));
                eprintln!("smah-run: step '{}' failed: {reason}", step.id);
                return 1;
            }
        }
    }

    // Final outputs.
    if !plan.outputs.is_empty() {
        println!();
        for out in &plan.outputs {
            match produced.get(&out.name) {
                Some(text) => {
                    println!("---- output {} ----", out.name);
                    println!("{text}");
                }
                None => eprintln!(
                    "smah-run: output '{}' was not produced (its step was skipped)",
                    out.name
                ),
            }
        }
    }
    println!("smah-run: wrapper '{}' completed.", plan.name);
    0
}

/// Execute one step. `Ok(Some(text))` = captured output (step declared
/// `output:`); `Ok(None)` = no capture; `Err(reason)` = failure.
fn execute_step(
    step: &ResolvedStep,
    outputs: &BTreeMap<String, String>,
) -> Result<Option<String>, String> {
    match &step.kind {
        ResolvedStepKind::Builtin {
            tool_name, bound, ..
        } => {
            let tool = builtin::builtin(tool_name)
                .ok_or_else(|| format!("unknown built-in tool '{tool_name}'"))?;
            let mut resolved = BTreeMap::new();
            for (k, v) in bound {
                resolved.insert(k.clone(), substitute_step_outputs(v, outputs)?);
            }
            let text = (tool.run)(&resolved)?;
            Ok(Some(text))
        }
        ResolvedStepKind::Run { argv } => {
            let mut resolved_argv = Vec::with_capacity(argv.len());
            for el in argv {
                resolved_argv.push(substitute_step_outputs(el, outputs)?);
            }
            let cwd = match &step.cwd {
                Some(c) => Some(substitute_step_outputs(c, outputs)?),
                None => None,
            };
            spawn_and_wait(&resolved_argv, cwd.as_deref(), step)
        }
    }
}

/// Spawn a fixed argv (the ONLY process spawn in the workspace) and wait
/// under the step's timeout. Stdout is captured (capped) when the step
/// declares an `output:`; otherwise it streams to the terminal live.
fn spawn_and_wait(
    argv: &[String],
    cwd: Option<&str>,
    step: &ResolvedStep,
) -> Result<Option<String>, String> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    if let Some(dir) = cwd {
        command.current_dir(dir);
    }
    let capture = step.output.is_some();
    command.stdout(if capture {
        Stdio::piped()
    } else {
        Stdio::inherit()
    });
    command.stderr(Stdio::inherit());

    let mut child = command
        .spawn()
        .map_err(|err| format!("cannot spawn '{}': {err}", argv[0]))?;

    // Reader thread for captured stdout (prevents pipe-buffer deadlock on
    // chatty steps; caps memory).
    let reader = capture.then(|| {
        let stdout = child.stdout.take();
        std::thread::spawn(move || {
            let mut collected: Vec<u8> = Vec::new();
            if let Some(mut stdout) = stdout {
                let mut chunk = [0u8; 8192];
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if collected.len() < OUTPUT_CAPTURE_MAX {
                                let room = OUTPUT_CAPTURE_MAX - collected.len();
                                collected.extend_from_slice(&chunk[..n.min(room)]);
                            }
                        }
                    }
                }
            }
            collected
        })
    });

    let deadline = Instant::now() + Duration::from_secs(step.timeout_secs.max(1));
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break Err("timeout".to_string());
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(err) => break Err(err.to_string()),
        }
    };

    let captured = reader.map(|handle| handle.join().unwrap_or_default());
    match status {
        Ok(status) if status.success() => Ok(captured.map(|bytes| {
            let mut text = String::from_utf8_lossy(&bytes).to_string();
            if bytes.len() >= OUTPUT_CAPTURE_MAX {
                text.push_str(&format!(
                    "\n[smah-run: output capped at {OUTPUT_CAPTURE_MAX} bytes]"
                ));
            }
            text
        })),
        Ok(status) => Err(format!("exit status {}", status.code().unwrap_or(-1))),
        Err(_) => Err(format!(
            "timed out after {}s (killed)",
            step.timeout_secs.max(1)
        )),
    }
}

/// Local time, RFC 3339 (repo convention).
fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secretish_names_are_masked_in_audit_argv() {
        let mut supplied = BTreeMap::new();
        supplied.insert("api_token".to_string(), "hunter2".to_string());
        supplied.insert("label".to_string(), "public".to_string());
        let argv = vec![
            "curl".to_string(),
            "-H".to_string(),
            "x:hunter2".to_string(),
            "label=public".to_string(),
        ];
        let redacted = redact_argv(&argv, &supplied);
        assert!(redacted.contains("***"), "secret value masked: {redacted}");
        assert!(!redacted.contains("hunter2"));
        assert!(
            redacted.contains("public"),
            "non-secret survives: {redacted}"
        );
    }

    #[test]
    fn name_heuristic_hits_and_misses() {
        assert!(name_looks_secret("api_token"));
        assert!(name_looks_secret("Password"));
        assert!(name_looks_secret("ssh_key"));
        assert!(!name_looks_secret("label"));
        assert!(!name_looks_secret("cast_file"));
    }

    #[test]
    fn step_output_substitution_resolves_and_rejects() {
        let mut outputs = BTreeMap::new();
        outputs.insert("to-text".to_string(), "hello".to_string());
        assert_eq!(
            substitute_step_outputs("{{steps.to-text.output}}!", &outputs).unwrap(),
            "hello!"
        );
        // An output whose step never ran stays symbolic → hard error.
        assert!(substitute_step_outputs("x {{steps.ghost.output}}", &outputs).is_err());
        // Fully substituted text passes through.
        let mut o = BTreeMap::new();
        o.insert("text-steps".to_string(), "v".to_string());
        assert_eq!(
            substitute_step_outputs("plain {{steps.text-steps.output}}", &o).unwrap(),
            "plain v"
        );
    }
}
