//! `smah hooks` — shell-hook registration snippets and shim management
//! (design decision D3).
//!
//! - `zsh`/`bash`: a function per tool — **stdout only**; warnings about
//!   skipped invalid tool files go to stderr. Names are validated at load
//!   (`^[a-z][a-z0-9_-]{0,63}$`, reserved set), so the emission needs no
//!   quoting and cannot inject shell metacharacters (T-014).
//! - `envrc`: `PATH_add <state>/bin` + a stderr hint (direnv cannot export
//!   functions, so it points at shims).
//! - `shims`: **writes** one executable `smah.<name>` per tool, chmod 0755,
//!   and removes stale shims for tools no longer installed (sync). This is a
//!   write command: the summary goes to stderr, nothing to stdout.
//!
//! Missing tools dir / zero valid tools ⇒ an empty snippet with a stderr
//! warning, exit 0 — an empty install is not a failure. Document: source
//! hooks after personal aliases (last definition wins).

use std::path::{Path, PathBuf};

use smah_core::virtual_tools::audit::state_dir;
use smah_core::virtual_tools::loader::{discover_in, tools_dir, ToolDef};

use crate::args::HooksCommand;

/// Dispatch `smah hooks <cmd>`; returns the process exit code.
pub fn run(cmd: &HooksCommand) -> i32 {
    match cmd {
        HooksCommand::Zsh => emit_shell("zsh"),
        HooksCommand::Bash => emit_shell("bash"),
        HooksCommand::Envrc => emit_envrc(),
        HooksCommand::Shims { dir } => write_shims(dir.as_deref()),
    }
}

/// Pure snippet generator (snapshotted by the golden tests): one function
/// per tool under the `smah.` namespace. `generated_at` is stamped by the
/// caller so tests pin deterministic output.
pub fn shell_snippet(shell: &str, tools: &[ToolDef], generated_at: &str) -> String {
    let mut out = String::new();
    out.push_str(&format!(
        "# smah virtual tools — generated {generated_at} ({} tools)\n",
        tools.len()
    ));
    for tool in tools {
        // Tool names are regex-validated + reserved-set-checked at load:
        // lowercase letters/digits/'-'/'_' only, no shell metacharacters.
        out.push_str(&format!(
            "smah.{}() {{ smah --emulate-cli {} \"$@\"; }}\n",
            tool.name, tool.name
        ));
    }
    let _ = shell; // zsh and bash share the function form today
    out
}

fn emit_shell(shell: &str) -> i32 {
    let dir = tools_dir();
    let (tools, invalid) = discover_in(&dir);
    for bad in &invalid {
        eprintln!(
            "smah: skipping invalid tool file {}: {}",
            bad.path.display(),
            bad.reason
        );
    }
    if tools.is_empty() {
        eprintln!(
            "smah: no virtual tools installed in {} — nothing to hook yet (drop .md files there, or override SMAH_VIRTUAL_TOOLS_DIR)",
            dir.display()
        );
    }
    let generated_at = chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, false);
    print!("{}", shell_snippet(shell, &tools, &generated_at));
    0
}

fn emit_envrc() -> i32 {
    let (tools, invalid) = discover_in(&tools_dir());
    for bad in &invalid {
        eprintln!(
            "smah: skipping invalid tool file {}: {}",
            bad.path.display(),
            bad.reason
        );
    }
    let bin = state_dir().join("bin");
    println!("PATH_add {}", bin.display());
    if tools.is_empty() {
        eprintln!(
            "smah: no virtual tools installed yet — run 'smah hooks shims' after installing some"
        );
    } else {
        eprintln!(
            "smah: direnv cannot export shell functions; this adds the shim dir to PATH. Run 'smah hooks shims' to (re)generate the shims."
        );
    }
    0
}

fn write_shims(dir: Option<&Path>) -> i32 {
    let (tools, invalid) = discover_in(&tools_dir());
    for bad in &invalid {
        eprintln!(
            "smah: skipping invalid tool file {}: {}",
            bad.path.display(),
            bad.reason
        );
    }
    let bin: PathBuf = dir
        .map(Path::to_path_buf)
        .unwrap_or_else(|| state_dir().join("bin"));
    if let Err(err) = std::fs::create_dir_all(&bin) {
        eprintln!("smah: cannot create shim dir {}: {err}", bin.display());
        return 1;
    }

    let mut written = 0usize;
    for tool in &tools {
        let path = bin.join(format!("smah.{}", tool.name));
        let script = format!("#!/bin/sh\nexec smah --emulate-cli {} \"$@\"\n", tool.name);
        match std::fs::write(&path, script).and_then(|()| set_executable(&path)) {
            Ok(()) => written += 1,
            Err(err) => {
                eprintln!("smah: could not write shim {}: {err}", path.display());
                return 1;
            }
        }
    }

    // Sync: remove stale shims for tools no longer installed.
    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(&bin) {
        let live: Vec<String> = tools.iter().map(|t| format!("smah.{}", t.name)).collect();
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with("smah.")
                && !live.contains(&name)
                && std::fs::remove_file(entry.path()).is_ok()
            {
                removed += 1;
            }
        }
    }

    eprintln!(
        "smah: {} shim(s) written, {} stale removed in {} (direnv: PATH_add this dir)",
        written,
        removed,
        bin.display()
    );
    0
}

#[cfg(unix)]
fn set_executable(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms)
}

#[cfg(not(unix))]
fn set_executable(_path: &Path) -> std::io::Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Deterministic fixtures (pure function — no env, no clock).
    fn fixture_tools() -> Vec<ToolDef> {
        let parse = |name: &str| {
            let dir = tempfile::tempdir().expect("tmp");
            let path = dir.path().join(format!("{name}.md"));
            std::fs::write(
                &path,
                format!("---\nname: {name}\ndescription: fixture {name}\n---\nbody\n"),
            )
            .expect("fixture");
            let def = smah_core::virtual_tools::loader::load_from(&path).expect("loads");
            std::mem::forget(dir); // keep the leaked path valid for the run
            def
        };
        vec![parse("commit-msg"), parse("log-analyzer")]
    }

    const STAMP: &str = "2026-10-18T00:00:00-07:00";

    #[test]
    fn zsh_snippet_golden() {
        insta::assert_snapshot!(shell_snippet("zsh", &fixture_tools(), STAMP));
    }

    #[test]
    fn bash_snippet_golden() {
        insta::assert_snapshot!(shell_snippet("bash", &fixture_tools(), STAMP));
    }

    #[test]
    fn names_cannot_inject_shell_metacharacters() {
        // Loader-validated names only ever reach the snippet; assert the
        // invariant holds for the validator's whole accepted set.
        for name in ["a", "z9", "with-dash", "with_underscore"] {
            assert!(smah_core::virtual_tools::is_valid_name(name));
        }
        for name in ["bad;rm", "bad$(x)", "bad name", "bad'quote"] {
            assert!(!smah_core::virtual_tools::is_valid_name(name));
        }
    }
}
