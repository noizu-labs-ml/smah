//! `smah-run` — the companion **executor** for saved script wrappers
//! (SW-05). This is the only workspace crate allowed to spawn processes:
//! the CI no-shell gate exempts exactly `crates/smah-run/src` (see
//! `scripts/no-shell-gate.sh`, which self-tests both directions), and
//! `smah` itself never invokes this binary — it only prints the exact
//! command line.
//!
//! Contract (owner decisions 2026-11-09/10):
//!
//! - executes only **saved, user-approved** wrapper recipes: `--execute`
//!   requires an `approved:` stamp **bound to the content** (C1:
//!   `{at, sha256}` where the hash digests the canonicalized executable
//!   sections — see `smah_core::wrappers::approval`); the stamp is
//!   re-verified at execution time and a missing, malformed, or mismatched
//!   stamp refuses with a distinct exit code 4. There is no stdin recipe,
//!   no composing, no inference here;
//! - `smah-run approve <wrapper>` is the ONLY way a stamp lands: it prints
//!   the full resolved plan plus the exact bytes the hash binds and demands
//!   an interactive TTY `y/N` (a non-TTY must pass
//!   `--yes-i-reviewed-this`). A `--prompt` diff accepted in `smah` NEVER
//!   carries or adds approval — the composed file is unapproved until this
//!   explicit stamp;
//! - **dry-run by default** — prints the resolved plan exactly like
//!   `smah --wrapper <name>`; `--execute` is the explicit opt-in;
//! - `run:` steps spawn with a **fixed argv** (elements map 1:1 onto exec
//!   arguments; no shell, no interpolation, ever); built-in `use:` steps
//!   execute in-process via the registry (pure Rust, no exec);
//! - **per-step confirmation** (yes / skip / abort) on a TTY unless `--yes`;
//!   `--execute` on a non-TTY without `--yes` refuses before running
//!   anything — never a silent unattended run;
//! - **per-step timeout**: the step's `timeout_secs`, else `--timeout`
//!   (default 60s). Each step spawns in **its own process group** (H1) and
//!   a timeout kills the WHOLE group — SIGTERM, then SIGKILL after a short
//!   grace — so grandchildren never outlive the run;
//! - append-only **audit JSONL** at `~/.local/state/smah/audit-smah-run.jsonl`
//!   (honors `XDG_STATE_HOME`), 0700/0600, **metadata only**, and
//!   **fail-closed** (C2): an fsync'd `started` record is written BEFORE
//!   each step executes and a `finished` record after; if the `started`
//!   write fails the step does NOT run. An unusable audit therefore blocks
//!   execution rather than allowing an unaudited run;
//! - **redaction (M1)**: EVERY param value is recorded as its name plus a
//!   `sha256:<12-hex prefix>` token by default; a param may opt its value
//!   in for verbatim audit with `audit: plain` in the wrapper schema (the
//!   value still passes the registered-secret redactor).
//!
//! Exit codes (mirrors the smah six-class table where it applies):
//! 0 success · 2 usage · 3 config (invalid wrapper) ·
//! 4 approval integrity (no stamp / malformed stamp / content changed
//! since approval) · 1 a step failed, timed out, the audit-before-exec
//! write failed, or the run aborted.

use std::collections::BTreeMap;
use std::io::{IsTerminal, Read};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use clap::Parser;
use serde::Serialize;

use smah_core::errors::SmahError;
use smah_core::logging::redact;
use smah_core::wrappers::resolve::{ResolvedPlan, ResolvedStep, ResolvedStepKind};
use smah_core::wrappers::{approval, builtin, loader, prompt, resolve};

/// Captured-output cap per step (bytes). Over-cap output is truncated for
/// binding purposes; the step still succeeds (a transcript is data, not a
/// threat — but a runaway step must not eat RAM).
const OUTPUT_CAPTURE_MAX: usize = 1024 * 1024;

/// Grace between SIGTERM and SIGKILL when killing a timed-out process
/// group: long enough for a well-behaved step to flush and exit, short
/// enough that a runaway does not linger.
#[cfg(unix)]
const KILL_GRACE: Duration = Duration::from_millis(500);

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

/// `smah-run approve <wrapper> [--param k=v]... [--yes-i-reviewed-this]` —
/// routed ahead of clap so the flat run-mode CLI above stays intact.
#[derive(Parser, Debug)]
#[command(
    name = "smah-run approve",
    about = "Show the full resolved plan and, after an explicit review, stamp a content-bound approval ({at, sha256}) onto the wrapper file."
)]
struct ApproveCli {
    /// Wrapper name to approve.
    wrapper: String,

    /// Invocation param (display only — the stamp binds the FILE content).
    #[arg(long = "param", value_name = "NAME=VALUE", action = clap::ArgAction::Append)]
    param: Vec<String>,

    /// Confirm the review without a TTY (scripted approval). Never combine
    /// with a wrapper you have not actually read.
    #[arg(long = "yes-i-reviewed-this")]
    yes_i_reviewed_this: bool,
}

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let argv: Vec<String> = std::env::args().collect();
    if argv.get(1).map(String::as_str) == Some("approve") {
        let approve = ApproveCli::parse_from(
            std::iter::once(argv[0].clone()).chain(argv[2..].iter().cloned()),
        );
        return cmd_approve(&approve);
    }

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
            "(dry-run — pass --execute to run; execution requires the content-bound \
             'approved:' stamp — 'smah-run approve {}' after reviewing the plan)",
            cli.name
        );
        return 0;
    }

    // Guard 1: approval INTEGRITY (C1) — present, well-formed, AND bound to
    // the current executable content; distinct exit code 4. Guard 2:
    // interactivity. Both BEFORE anything runs.
    if let Err(problem) = approval::verify(&wrapper) {
        eprintln!(
            "smah-run: wrapper '{}' execution refused: {}",
            cli.name,
            problem.describe()
        );
        return 4;
    }
    if !cli.yes && !std::io::stdin().is_terminal() {
        eprintln!(
            "smah-run: --execute on a non-TTY needs --yes (no interactive confirmation possible); refusing."
        );
        return 2;
    }

    let run_id = uuid::Uuid::new_v4().to_string();
    let audit = RunAudit::open();
    let param_tokens = audit_param_tokens(&wrapper, &supplied);
    let param_record: BTreeMap<String, String> = param_tokens
        .iter()
        .map(|(name, _, token)| (name.clone(), token.clone()))
        .collect();
    execute_plan(
        &plan,
        cli.yes,
        &run_id,
        &audit,
        &param_tokens,
        &param_record,
    )
}

/// `smah-run approve <wrapper>` — show the plan + the exact hashed bytes,
/// demand an explicit review, then atomically stamp
/// `approved: {at, sha256}` onto the wrapper file.
fn cmd_approve(cli: &ApproveCli) -> i32 {
    let wrapper = match loader::load_by_name(&cli.wrapper) {
        Ok(w) => w,
        Err(reason) if reason.starts_with("not found:") => {
            eprintln!(
                "smah-run: no wrapper named '{}'. 'smah wrapper list' shows what is installed.",
                cli.wrapper
            );
            return 2;
        }
        Err(reason) => {
            eprintln!("smah-run: wrapper '{}' is invalid: {}", cli.wrapper, reason);
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

    // The full resolved plan (params bound where possible). Missing
    // required params only degrade the DISPLAY — the stamp binds the file,
    // not an invocation.
    match resolve::resolve(&wrapper, &supplied) {
        Ok(plan) => {
            print!("{}", resolve::render_plan(&plan));
            println!();
        }
        Err(SmahError::WrapperParamMissing { params, .. }) => {
            eprintln!(
                "smah-run: note: required param(s) not supplied for display: {params} — \
                 step argv shows their placeholders below; the stamp binds the FILE, \
                 not an invocation."
            );
        }
        Err(err) => {
            eprintln!("{}", err.render("smah.log"));
            return err.exit_code().code();
        }
    }

    // The exact bytes the hash binds — what "I reviewed this" means.
    println!("---- approval binds exactly this content (params/steps/outputs) ----");
    print!("{}", approval::canonical_executable_yaml(&wrapper));
    println!("----------------------------------------------------------------------");
    if let Some(stamp) = &wrapper.approved {
        match approval::verify(&wrapper) {
            Ok(()) => println!(
                "current stamp: approved {} sha256 {}… — matches this content (re-approving only refreshes the time).",
                stamp.at,
                &stamp.sha256[..stamp.sha256.len().min(12)]
            ),
            Err(problem) => println!("existing stamp does NOT verify: {}", problem.describe()),
        }
    }
    let sha = approval::content_sha256(&wrapper);

    // Approval is a human act: interactive y/N on a TTY, and the default
    // answer is NO. A script must say --yes-i-reviewed-this out loud.
    if !std::io::stdin().is_terminal() && !cli.yes_i_reviewed_this {
        eprintln!(
            "smah-run: approve on a non-TTY needs --yes-i-reviewed-this (no interactive \
             review is possible here); refusing."
        );
        return 2;
    }
    if std::io::stdin().is_terminal() {
        use std::io::BufRead;
        print!(
            "stamp approval for '{}' (binds sha256 {}…)? type 'y' to approve: [y/N] ",
            cli.wrapper,
            &sha[..sha.len().min(12)]
        );
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let stdin = std::io::stdin();
        let answer = stdin.lock().lines().next();
        match answer {
            Some(Ok(line))
                if line.trim().eq_ignore_ascii_case("y")
                    || line.trim().eq_ignore_ascii_case("yes") => {}
            _ => {
                eprintln!("smah-run: not approved — nothing written.");
                return 1;
            }
        }
    }

    // Stamp: approved = {at: now, sha256: content}. Atomic write, and the
    // exact bytes re-validate through the loader (stamp included) before
    // the rename — a stamp that would not round-trip never lands.
    let stamp = approval::stamp_for(&wrapper, &now_rfc3339());
    let mut stamped = wrapper.clone();
    stamped.approved = Some(stamp);
    let content = prompt::render_wrapper_file(&stamped);
    let provenance =
        prompt::extract_provenance(&std::fs::read_to_string(&wrapper.path).unwrap_or_default());
    let content = prompt::render_with_provenance(&content, &provenance);
    let reparsed =
        match loader::parse_wrapper(&content, &wrapper.name, &prompt::current_tool_names()) {
            Ok(def) => def,
            Err(detail) => {
                eprintln!("smah-run: stamped file failed re-validation: {detail}");
                return 3;
            }
        };
    if approval::verify(&reparsed).is_err() {
        eprintln!("smah-run: stamped file does not verify after round-trip — refusing to write.");
        return 3;
    }
    let tmp = wrapper.path.with_extension("md.tmp");
    if let Err(err) = std::fs::write(&tmp, &content) {
        eprintln!("smah-run: cannot write {}: {err}", tmp.display());
        return 3;
    }
    if let Err(err) = std::fs::rename(&tmp, &wrapper.path) {
        eprintln!("smah-run: cannot replace {}: {err}", wrapper.path.display());
        return 3;
    }
    println!(
        "smah: approved '{}' at {} (sha256 {}…). 'smah-run {} --execute' now verifies \
         against this hash — any later executable edit refuses with exit 4 until re-approved.",
        cli.wrapper,
        stamped.approved.as_ref().expect("just stamped").at,
        &sha[..sha.len().min(12)],
        cli.wrapper
    );
    0
}

/// One audit JSONL record — metadata only.
#[derive(Serialize)]
struct StepRecord {
    ts: String,
    run: String,
    wrapper: String,
    step: String,
    kind: &'static str,
    /// Redacted, space-joined argv (or the built-in display argv): every
    /// param value replaced by its audit token (M1).
    argv: String,
    /// Param name → audit token: `sha256:<12-hex prefix>` by default, the
    /// declared value (post-redactor) for `audit: plain` params.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    params: BTreeMap<String, String>,
    /// started | finished | skipped | aborted.
    decision: &'static str,
    /// started | ok | exit N | timeout | error: ... | skipped | aborted.
    status: String,
    #[serde(rename = "millis")]
    duration_ms: u128,
    #[serde(skip_serializing_if = "Option::is_none")]
    stdout_bytes: Option<usize>,
}

/// Append-only audit sink (`~/.local/state/smah/audit-smah-run.jsonl`,
/// 0700 dir / 0600 file). C2: `record` fsyncs every line and REPORTS
/// failure — the caller of a `started` record must refuse to run on error
/// (fail-closed); later records (finished/skipped/aborted) only warn.
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

    /// Append one record, fsync'd. `Err(reason)` = the line is NOT durably
    /// on disk (or can never be) — the pre-execution caller must refuse.
    fn record(&self, rec: &StepRecord) -> Result<(), String> {
        if let Some(reason) = &self.disabled {
            return Err(reason.clone());
        }
        let line = serde_json::to_string(rec).map_err(|err| err.to_string())?;
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
                writeln!(file, "{line}")?;
                file.sync_data()?;
                Ok(())
            });
        append.map_err(|err| err.to_string())
    }
}

/// Effective param rows for audit: (name, value, token), longest value
/// first (safe argv replacement when one value contains another). M1: the
/// token is `sha256:<12-hex prefix>` for every param unless it declares
/// `audit: plain` — then the value survives verbatim (still through the
/// registered-secret redactor). Defaults redact like supplied values.
fn audit_param_tokens(
    wrapper: &loader::WrapperDef,
    supplied: &BTreeMap<String, String>,
) -> Vec<(String, String, String)> {
    let mut rows = Vec::new();
    for p in &wrapper.params {
        let Some(value) = supplied.get(&p.name).cloned().or_else(|| p.default.clone()) else {
            continue;
        };
        let token = if p.audit_plain {
            redact(&value)
        } else {
            format!("sha256:{}", &prompt::sha256_hex(value.as_bytes())[..12])
        };
        rows.push((p.name.clone(), value, token));
    }
    // Longest value first so the argv replacement cannot re-match inside an
    // already-substituted token.
    rows.sort_by_key(|row| std::cmp::Reverse(row.1.len()));
    rows
}

/// Redact an argv for the audit log: every effective param value replaced
/// by its audit token, then the registered-secret redactor as defense in
/// depth for anything that is not a param.
fn redact_argv(argv: &[String], tokens: &[(String, String, String)]) -> String {
    let mut joined = argv.join(" ");
    for (_name, value, token) in tokens {
        if !value.is_empty() {
            joined = joined.replace(value.as_str(), token);
        }
    }
    redact(&joined)
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
    yes: bool,
    run_id: &str,
    audit: &RunAudit,
    param_tokens: &[(String, String, String)],
    param_record: &BTreeMap<String, String>,
) -> i32 {
    // Two key spaces: step-id keyed for `{{steps.<id>.output}}` substitution,
    // output-name keyed for the final `outputs:` printing.
    let mut outputs: BTreeMap<String, String> = BTreeMap::new();
    let mut produced: BTreeMap<String, String> = BTreeMap::new();
    println!(
        "smah-run: executing wrapper '{}' ({} steps, approved {})",
        plan.name,
        plan.steps.len(),
        plan.approved.as_ref().map(|s| s.at.as_str()).unwrap_or("?")
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
                    warn_err(audit.record(&StepRecord {
                        ts: now_rfc3339(),
                        run: run_id.to_string(),
                        wrapper: plan.name.clone(),
                        step: step.id.clone(),
                        kind: kind_label,
                        argv: redact_argv(&display_argv, param_tokens),
                        params: param_record.clone(),
                        decision: "skipped",
                        status: "skipped".to_string(),
                        duration_ms: 0,
                        stdout_bytes: None,
                    }));
                    println!("smah-run: step '{}' skipped.", step.id);
                    continue;
                }
                Err(reason) => {
                    warn_err(audit.record(&StepRecord {
                        ts: now_rfc3339(),
                        run: run_id.to_string(),
                        wrapper: plan.name.clone(),
                        step: step.id.clone(),
                        kind: kind_label,
                        argv: redact_argv(&display_argv, param_tokens),
                        params: param_record.clone(),
                        decision: "aborted",
                        status: "aborted".to_string(),
                        duration_ms: 0,
                        stdout_bytes: None,
                    }));
                    eprintln!("smah-run: {reason} (before step '{}').", step.id);
                    return 1;
                }
            }
        }

        // C2: audit BEFORE execution — the fsync'd `started` record is the
        // durable proof the step ran. If it cannot be written, the step
        // does NOT run: an unauditable execution is exactly the T-032
        // bypass, and fail-closed beats fail-open here.
        let started_record = StepRecord {
            ts: now_rfc3339(),
            run: run_id.to_string(),
            wrapper: plan.name.clone(),
            step: step.id.clone(),
            kind: kind_label,
            argv: redact_argv(&display_argv, param_tokens),
            params: param_record.clone(),
            decision: "started",
            status: "started".to_string(),
            duration_ms: 0,
            stdout_bytes: None,
        };
        if let Err(err) = audit.record(&started_record) {
            eprintln!(
                "smah-run: audit 'started' write for step '{}' failed ({err}) — \
                 refusing to execute (fail-closed).",
                step.id
            );
            return 1;
        }

        let started = Instant::now();
        let outcome = execute_step(step, &outputs);
        let duration_ms = started.elapsed().as_millis();
        let record = |status: String, bytes: Option<usize>| StepRecord {
            ts: now_rfc3339(),
            run: run_id.to_string(),
            wrapper: plan.name.clone(),
            step: step.id.clone(),
            kind: kind_label,
            argv: redact_argv(&display_argv, param_tokens),
            params: param_record.clone(),
            decision: "finished",
            status,
            duration_ms,
            stdout_bytes: bytes,
        };

        match outcome {
            Ok(Some(text)) => {
                warn_err(audit.record(&record("ok".to_string(), Some(text.len()))));
                if let Some(name) = &step.output {
                    outputs.insert(step.id.clone(), text.clone());
                    produced.insert(name.clone(), text);
                }
            }
            Ok(None) => {
                warn_err(audit.record(&record("ok".to_string(), None)));
            }
            Err(reason) => {
                warn_err(audit.record(&record(reason.clone(), None)));
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

/// Post-decision audit records are best-effort: warn, never mask the
/// outcome (the fail-closed rule covers only the pre-execution record).
fn warn_err(result: Result<(), String>) {
    if let Err(err) = result {
        eprintln!("smah-run: audit write failed: {err}");
    }
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
///
/// H1: the child spawns as its OWN process group (unix
/// `process_group(0)`), so a timeout signals the whole tree — SIGTERM to
/// the group, then SIGKILL after [`KILL_GRACE`] — and grandchildren never
/// outlive the run.
fn spawn_and_wait(
    argv: &[String],
    cwd: Option<&str>,
    step: &ResolvedStep,
) -> Result<Option<String>, String> {
    let mut command = Command::new(&argv[0]);
    command.args(&argv[1..]);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
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
                    terminate_group(&mut child);
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
            "timed out after {}s (whole process group killed)",
            step.timeout_secs.max(1)
        )),
    }
}

/// H1: kill the child's ENTIRE process group on timeout — SIGTERM first,
/// SIGKILL after the grace period if anything survived. The child was
/// spawned as its own group leader (`process_group(0)`), so the negative
/// pid targets every descendant.
#[cfg(unix)]
fn terminate_group(child: &mut std::process::Child) {
    let group = -(child.id() as i32);
    // SAFETY: kill(2) with a negative pid signals the process group; no
    // memory is touched.
    unsafe {
        libc::kill(group, libc::SIGTERM);
    }
    let grace_deadline = Instant::now() + KILL_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if Instant::now() >= grace_deadline => break,
            Ok(None) => std::thread::sleep(Duration::from_millis(25)),
            Err(_) => break,
        }
    }
    if matches!(child.try_wait(), Ok(None)) {
        unsafe {
            libc::kill(group, libc::SIGKILL);
        }
    }
    let _ = child.wait();
}

/// Non-unix fallback: no process groups — kill the direct child (platform
/// limitation, documented in docs/ux/wrappers.md).
#[cfg(not(unix))]
fn terminate_group(child: &mut std::process::Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Local time, RFC 3339 (repo convention).
fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    fn def_of(raw: &str) -> loader::WrapperDef {
        loader::parse_wrapper(raw, "demo", &HashSet::new()).expect("loads")
    }

    /// Both params DEFAULT-redacted (no `audit:` key anywhere).
    const REDACT_WRAPPER: &str = "---\n\
name: demo\ndescription: d\n\
params:\n  - name: token\n    required: true\n  - name: label\n    default: public\n\
steps:\n  - id: s1\n    run: [\"true\"]\n---\n";

    /// `label` opts into verbatim audit — the only way a value survives.
    const PLAIN_WRAPPER: &str = "---\n\
name: demo\ndescription: d\n\
params:\n  - name: token\n    required: true\n  - name: label\n    default: public\n    audit: plain\n\
steps:\n  - id: s1\n    run: [\"true\"]\n---\n";

    #[test]
    fn all_param_values_hashed_in_audit_by_default() {
        // M1: EVERY param value is a name + sha256 prefix in the audit by
        // default — including values whose param name sounds harmless.
        let def = def_of(REDACT_WRAPPER);
        let mut supplied = BTreeMap::new();
        supplied.insert("token".to_string(), "hunter2".to_string());
        let tokens = audit_param_tokens(&def, &supplied);
        let argv = vec![
            "curl".to_string(),
            "-H".to_string(),
            "x:hunter2".to_string(),
            "label=public".to_string(),
        ];
        let redacted = redact_argv(&argv, &tokens);
        assert!(!redacted.contains("hunter2"), "hashed: {redacted}");
        assert!(
            !redacted.contains("public"),
            "defaults hash too: {redacted}"
        );
        assert!(redacted.contains("sha256:"), "{redacted}");
        // The record map: name → token for BOTH params.
        let record: BTreeMap<String, String> = tokens
            .iter()
            .map(|(n, _, t)| (n.clone(), t.clone()))
            .collect();
        let label = record.get("label").expect("label row");
        assert!(label.starts_with("sha256:"), "{label}");
        let tok = record.get("token").expect("token row");
        assert!(tok.starts_with("sha256:"));
        assert_eq!(tok.len(), "sha256:".len() + 12);
    }

    #[test]
    fn defaults_redact_like_supplied_values() {
        let def = def_of(REDACT_WRAPPER);
        // No --param at all: the label default 'public' still hashes.
        let tokens = audit_param_tokens(&def, &BTreeMap::new());
        let record: BTreeMap<String, String> = tokens
            .iter()
            .map(|(n, _, t)| (n.clone(), t.clone()))
            .collect();
        let label = record.get("label").expect("default row present");
        assert!(label.starts_with("sha256:"), "defaults hash too: {label}");
        assert!(!label.contains("public"));
    }

    #[test]
    fn audit_plain_opt_in_is_the_only_verbatim_path() {
        let def = def_of(PLAIN_WRAPPER);
        let mut supplied = BTreeMap::new();
        supplied.insert("token".to_string(), "hunter2".to_string());
        let tokens = audit_param_tokens(&def, &supplied);
        let record: BTreeMap<String, String> = tokens
            .iter()
            .map(|(n, _, t)| (n.clone(), t.clone()))
            .collect();
        // Opt-in keeps the value (post-redactor); everything else hashes.
        assert_eq!(record.get("label").map(String::as_str), Some("public"));
        assert!(record
            .get("token")
            .expect("token row")
            .starts_with("sha256:"));
        let argv = vec![
            "curl".to_string(),
            "x:hunter2".to_string(),
            "public".to_string(),
        ];
        let redacted = redact_argv(&argv, &tokens);
        assert!(!redacted.contains("hunter2"), "{redacted}");
        assert!(
            redacted.contains("public"),
            "opt-in value stays: {redacted}"
        );
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
