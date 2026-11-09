//! `smah wrapper …` + `smah --wrapper <name> [--param k=v …]` + argv[0]
//! `smah-<name>` dispatch — the deterministic wrapper surface (SW-02).
//!
//! Everything here is **print-only**: the flag form and `wrapper run`
//! resolve the recipe against the supplied params and print the ordered
//! plan (commands, risk notes, approval status) plus the exact `smah-run`
//! invocation that would execute it. Execution lives in the `smah-run`
//! crate; `smah` never invokes it.
//!
//! Postfix dispatch (owner decision: implement the review's recommended
//! argv[0] form AND a subcommand; both documented in docs/ux/wrappers.md):
//! a binary invoked as `smah-<name>` (symlink or hardlink to `smah`) whose
//! `<name>` matches an installed wrapper file re-dispatches as
//! `smah --wrapper <name>`. `smah-run` is excluded by name — that argv[0]
//! belongs to the companion runner, never to a wrapper called "run".

use std::collections::BTreeMap;
use std::path::Path;

use smah_core::errors::SmahError;
use smah_core::virtual_tools::is_valid_name;
use smah_core::wrappers::loader::{self, discover, WrapperDef};
use smah_core::wrappers::resolve::{self, ResolvedPlan};

use clap::Parser;

use crate::args::{Cli, WrapperCommand};

/// Dispatch `smah wrapper <cmd>`; returns the process exit code.
pub fn run_sub(cmd: &WrapperCommand) -> i32 {
    match cmd {
        WrapperCommand::List { json } => list(*json),
        WrapperCommand::Show { name } => show(name),
        WrapperCommand::Validate { name } => validate(name),
        WrapperCommand::Run { name, param } => run_flag(name, param),
    }
}

/// The `--wrapper <name>` flag path: resolve + print the deterministic plan.
pub fn run_flag(name: &str, params: &[String]) -> i32 {
    let logfile = "smah.log";
    match plan_for(name, params) {
        Ok(plan) => {
            print!("{}", resolve::render_plan(&plan));
            0
        }
        Err(err) => {
            eprintln!("{}", err.render(logfile));
            err.exit_code().code()
        }
    }
}

/// Load + resolve with W-series error mapping.
pub fn plan_for(name: &str, params: &[String]) -> Result<ResolvedPlan, SmahError> {
    let wrapper = load_wrapper(name)?;
    let supplied = parse_params(params)?;
    resolve::resolve(&wrapper, &supplied)
}

/// Load one wrapper by name, mapping loader reasons onto the W-series.
pub fn load_wrapper(name: &str) -> Result<WrapperDef, SmahError> {
    match loader::load_by_name(name) {
        Ok(def) => Ok(def),
        Err(reason) if reason.starts_with("not found:") => {
            let (valid, _) = discover();
            let mut names: Vec<String> = valid.into_iter().map(|w| w.name).collect();
            names.sort();
            Err(SmahError::WrapperNotFound {
                name: name.to_string(),
                installed: if names.is_empty() {
                    "(none)".to_string()
                } else {
                    names.join(", ")
                },
            })
        }
        Err(reason) => {
            // The loader names the file in fence errors; re-read the path.
            let path = loader::wrappers_dir().join(format!("{name}.md"));
            Err(SmahError::WrapperFileInvalid {
                path: path.display().to_string(),
                detail: reason,
            })
        }
    }
}

/// Parse `--param k=v` tokens into a map (dup last-wins).
fn parse_params(params: &[String]) -> Result<BTreeMap<String, String>, SmahError> {
    let mut map = BTreeMap::new();
    for token in params {
        let (k, v) = resolve::parse_param_token(token)?;
        map.insert(k, v);
    }
    Ok(map)
}

/// `smah wrapper list [--json]` — valid and invalid files both appear.
fn list(json: bool) -> i32 {
    let (mut wrappers, invalid) = discover();
    wrappers.sort_by(|a, b| a.name.cmp(&b.name));

    for bad in &invalid {
        eprintln!(
            "smah: invalid wrapper file {}: {}",
            bad.path.display(),
            bad.reason
        );
    }

    if json {
        let rows = wrappers
            .iter()
            .map(|w| {
                serde_json::json!({
                    "name": w.name,
                    "description": w.description,
                    "status": "valid",
                    "steps": w.steps.len(),
                    "params": w.params.iter().map(|p| p.name.clone()).collect::<Vec<_>>(),
                    "approved": w.approved.is_some(),
                })
            })
            .chain(invalid.iter().map(|b| {
                serde_json::json!({
                    "name": b.name,
                    "status": "invalid",
                    "reason": b.reason,
                })
            }))
            .collect::<Vec<_>>();
        println!(
            "{}",
            serde_json::to_string_pretty(&rows).unwrap_or_else(|_| "[]".to_string())
        );
        return 0;
    }

    if wrappers.is_empty() && invalid.is_empty() {
        eprintln!(
            "smah: no wrappers installed. Drop .md files into {} (see docs/ux/wrappers.md)",
            loader::wrappers_dir().display()
        );
        return 0;
    }

    println!("{:<22} {:<8} {:<7} DESCRIPTION", "NAME", "STATUS", "STEPS");
    for w in &wrappers {
        println!(
            "{:<22} {:<8} {:<7} {}",
            w.name,
            if w.approved.is_some() {
                "valid*"
            } else {
                "valid"
            },
            w.steps.len(),
            w.description
        );
    }
    for bad in &invalid {
        println!(
            "{:<22} {:<8} {:<7} {}",
            bad.name, "invalid", "-", bad.reason
        );
    }
    0
}

/// `smah wrapper show <name>` — print the definition file verbatim (what
/// you would edit); the plan printer is `--wrapper`/`run`.
fn show(name: &str) -> i32 {
    match load_wrapper(name) {
        Ok(wrapper) => match std::fs::read_to_string(&wrapper.path) {
            Ok(raw) => {
                print!("{raw}");
                0
            }
            Err(err) => {
                eprintln!("smah: cannot read {}: {err}", wrapper.path.display());
                3
            }
        },
        Err(err) => {
            eprintln!("{}", err.render("smah.log"));
            err.exit_code().code()
        }
    }
}

/// `smah wrapper validate <name>` — load + static-resolve (no params
/// supplied; missing required params are reported as info, not failures —
/// they are invocation-time inputs, not file defects).
fn validate(name: &str) -> i32 {
    let wrapper = match load_wrapper(name) {
        Ok(w) => w,
        Err(err) => {
            eprintln!("{}", err.render("smah.log"));
            return err.exit_code().code();
        }
    };
    let empty = BTreeMap::new();
    match resolve::resolve(&wrapper, &empty) {
        Ok(_) => {
            println!(
                "smah: wrapper '{}' is valid: {} step(s), {} param(s){}",
                wrapper.name,
                wrapper.steps.len(),
                wrapper.params.len(),
                if wrapper.approved.is_some() {
                    ", approved for execution"
                } else {
                    ", not approved for execution"
                }
            );
            0
        }
        // Missing required params are expected without --param: report and
        // stay green — the file itself is fine.
        Err(SmahError::WrapperParamMissing { params, .. }) => {
            println!(
                "smah: wrapper '{}' is valid: {} step(s); invocation needs --param for: {params}",
                wrapper.name,
                wrapper.steps.len()
            );
            0
        }
        Err(err) => {
            eprintln!("{}", err.render("smah.log"));
            err.exit_code().code()
        }
    }
}

/// The wrapper name an argv[0] carries, if any: `smah-<name>` where `<name>`
/// is a valid wrapper name and not the reserved `run` (that argv[0] is the
/// companion runner's own).
pub fn wrapper_name_from_argv0(argv0: &str) -> Option<String> {
    let base = Path::new(argv0).file_name()?.to_string_lossy().to_string();
    let name = base.strip_prefix("smah-")?;
    if name == "run" || name.is_empty() || !is_valid_name(name) {
        return None;
    }
    Some(name.to_string())
}

/// argv[0] postfix dispatch (`smah-<name>`): `Some(exit_code)` when argv[0]
/// named an installed wrapper and the call was handled here; `None` = fall
/// through to normal clap parsing (not our argv[0], or no such wrapper
/// file — the normal usage error is the honest answer then).
pub fn run_argv0_dispatch() -> Option<i32> {
    let argv: Vec<String> = std::env::args().collect();
    let name = wrapper_name_from_argv0(argv.first()?)?;
    // Dispatch only when a wrapper file actually exists; otherwise let
    // clap own the error (an unrelated `smah-*` binary is not ours).
    let candidate = loader::wrappers_dir().join(format!("{name}.md"));
    if !candidate.is_file() {
        return None;
    }
    let rest: Vec<String> = argv[1..].to_vec();
    let mut forwarded: Vec<String> =
        vec!["smah".to_string(), "--wrapper".to_string(), name.clone()];
    forwarded.extend(rest);
    let cli = Cli::try_parse_from(forwarded).unwrap_or_else(|err| err.exit());
    let name = cli
        .wrapper
        .clone()
        .expect("argv was rewritten with --wrapper");
    Some(run_flag(&name, &cli.param))
}

#[cfg(test)]
mod tests {
    use super::wrapper_name_from_argv0;

    #[test]
    fn argv0_names_extract_and_reject() {
        assert_eq!(
            wrapper_name_from_argv0("smah-demo").as_deref(),
            Some("demo")
        );
        assert_eq!(
            wrapper_name_from_argv0("/usr/local/bin/smah-demo").as_deref(),
            Some("demo")
        );
        assert_eq!(
            wrapper_name_from_argv0("smah-run"),
            None,
            "runner's own argv0"
        );
        assert_eq!(wrapper_name_from_argv0("smah-"), None);
        assert_eq!(wrapper_name_from_argv0("smah"), None);
        assert_eq!(
            wrapper_name_from_argv0("smah-Bad"),
            None,
            "invalid name chars"
        );
        assert_eq!(wrapper_name_from_argv0("/bin/ls"), None);
    }
}
