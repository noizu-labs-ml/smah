//! `smah --emulate-cli <tool>` — the emulated-CLI wiring (design decisions
//! D2/D4/D5/D6): settings + tool load, stdin handling with loud truncation,
//! the file-surface disclosure line, provider resolution through the M2
//! adapter, exit-code discipline, and the write-only transcript cache.

use std::io::{IsTerminal, Read};
use std::path::PathBuf;

use smah_core::errors::SmahError;
use smah_core::settings::Settings;
use smah_core::virtual_tools::audit::{cache_dir, Audit};
use smah_core::virtual_tools::driver::{self, FsMode, StdSink};
use smah_core::virtual_tools::loader;
use smah_core::virtual_tools::DEFAULT_INPUT_BYTES;

use crate::args::Cli;
use crate::backend::{split_model_spec, ProviderBackend, DEFAULT_MODEL_ID, DEFAULT_PROVIDER};

/// Run the emulate path; returns the process exit code. `tool_args` is the
/// raw tail after `<tool>` (see [`split_emulate_tail`]).
pub fn run_emulate(cli: &Cli, tool_name: &str, tool_args: &[String]) -> i32 {
    let logfile = init_logfile();
    match run_emulate_inner(cli, tool_name, tool_args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err.render(&logfile.display().to_string()));
            err.exit_code().code()
        }
    }
}

fn run_emulate_inner(cli: &Cli, tool_name: &str, tool_args: &[String]) -> Result<i32, SmahError> {
    // Settings (a virtual tool needs the provider blocks).
    let settings = match &cli.config {
        Some(path) => Settings::load_from(path)?,
        None => Settings::load()?,
    };

    // Tool: direct invoke resolves to exactly one file (<dir>/<tool>.md).
    let tool = loader::load_by_name(tool_name)?;

    // Model precedence: invocation --model > frontmatter > spec default.
    let model_spec = cli
        .model
        .clone()
        .or_else(|| tool.model.clone())
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    let (provider_key, model_id) = split_model_spec(&model_spec, DEFAULT_PROVIDER);

    // Provider (credentials resolved at construction; handled keys go through
    // smah-core's secret register — smah-providers does that in credentials).
    let provider = smah_providers::registry::get_provider(&provider_key, &settings, None)?;

    // File-surface mode + the stderr roots disclosure (T-010 visibility).
    let fs_mode = if tool.allow_roots.is_empty() {
        FsMode::NoneDeclared
    } else if cli.no_fs {
        eprintln!("file surface: disabled (--no-fs)");
        FsMode::DisabledByFlag
    } else {
        let roots = loader::resolve_roots(&tool)?;
        let list = roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ");
        eprintln!("file surface: {list} (disable: --no-fs)");
        FsMode::Enabled(roots)
    };

    // Stdin: piped data is read fully; a TTY is never read (§5). Truncation
    // is loud, not fatal (§5 rule 7).
    let cap = cli.max_input_chars.unwrap_or(DEFAULT_INPUT_BYTES);
    let (stdin_text, truncation) = read_stdin(cap, std::io::stdin().is_terminal());
    if let Some((sent, total)) = truncation {
        eprintln!("input truncated: sent {sent} of {total} bytes (--max-input-chars to change)");
    }
    let stdin_partial = truncation.is_some();

    let run_id = uuid::Uuid::new_v4().to_string();
    let audit = Audit::open(&run_id);
    if let Some(reason) = audit.disabled_reason() {
        eprintln!("smah(virtual): audit log unavailable: {reason}");
    }
    let backend = ProviderBackend::new(provider);
    let mut sink = StdSink;

    match driver::run(
        &tool,
        &backend,
        &mut sink,
        &model_id,
        tool_args,
        stdin_text.as_deref(),
        stdin_partial,
        &fs_mode,
        &audit,
        &run_id,
    ) {
        Ok(outcome) => {
            write_transcript(&tool, &outcome.messages);
            Ok(0)
        }
        Err(err) => {
            write_transcript_marker(&tool);
            Err(err)
        }
    }
}

/// The error-render path wants a log-file path even if logging init failed.
fn init_logfile() -> PathBuf {
    smah_core::logging::init().unwrap_or_else(|_| PathBuf::from("smah.log"))
}

/// Real-stdin wrapper over [`read_stdin_from`]: `is_tty` short-circuits (a
/// TTY is never read — §5). `None` truncation means nothing was read (TTY or
/// read error); `Some((sent, total))` marks a loud truncation.
fn read_stdin(cap: usize, is_tty: bool) -> (Option<String>, Option<(usize, usize)>) {
    if is_tty {
        return (None, None);
    }
    let mut stdin = std::io::stdin().lock();
    let (text, total) = read_stdin_from(&mut stdin, cap);
    let truncation = (total > cap).then_some((text.len(), total));
    (Some(text), truncation)
}

/// Capped read of an arbitrary reader (unit-testable stdin seam). Returns
/// the (lossy-decoded, capped) text and the **uncapped** byte total.
fn read_stdin_from<R: Read>(reader: &mut R, cap: usize) -> (String, usize) {
    let mut raw = Vec::new();
    if reader.read_to_end(&mut raw).is_err() {
        return (String::new(), 0);
    }
    let total = raw.len();
    raw.truncate(cap);
    (String::from_utf8_lossy(&raw).to_string(), total)
}


/// sha256 hex of the tool file (the cache marker).
fn tool_sha256(tool: &loader::ToolDef) -> String {
    use sha2::{Digest, Sha256};
    match std::fs::read(&tool.path) {
        Ok(bytes) => {
            let digest = Sha256::digest(&bytes);
            digest.iter().map(|b| format!("{b:02x}")).collect()
        }
        Err(_) => String::new(),
    }
}

/// Write the regenerable transcript scratch (design decision D5):
/// `~/.cache/smah/virtual-tools/<tool>/last-conversation.json` + a
/// `tool.sha256` marker. Write-only in this PR — no consumer yet; failures
/// are stderr warnings, never fatal.
fn write_transcript(
    tool: &loader::ToolDef,
    messages: &[smah_core::virtual_tools::driver::ChatMessage],
) {
    let sha = tool_sha256(tool);
    let dir = cache_dir().join("virtual-tools").join(&tool.name);
    let json = serde_json::json!({
        "tool": tool.name,
        "tool.sha256": sha,
        "model": tool.model,
        "turns": messages.len(),
        "messages": messages.iter().map(|m| serde_json::json!({
            "role": match m.role {
                smah_core::virtual_tools::driver::Role::User => "user",
                smah_core::virtual_tools::driver::Role::Assistant => "assistant",
            },
            "content": m.content,
        })).collect::<Vec<_>>(),
    });
    let _ = std::fs::create_dir_all(&dir);
    if let Err(err) = std::fs::write(dir.join("last-conversation.json"), json.to_string()) {
        eprintln!("smah(virtual): transcript cache write failed: {err}");
    }
    if let Err(err) = std::fs::write(dir.join("tool.sha256"), &sha) {
        eprintln!("smah(virtual): transcript cache marker write failed: {err}");
    }
}

/// Error-path marker refresh (transcript itself is not written).
fn write_transcript_marker(tool: &loader::ToolDef) {
    let sha = tool_sha256(tool);
    let dir = cache_dir().join("virtual-tools").join(&tool.name);
    let _ = std::fs::create_dir_all(&dir);
    let _ = std::fs::write(dir.join("tool.sha256"), &sha);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_stdin_is_never_read() {
        assert_eq!(read_stdin(10, true), (None, None));
    }

    #[test]
    fn pipe_stdin_read_and_truncated_at_cap() {
        let data = "x".repeat(100);
        let (text, total) = read_stdin_from(&mut data.as_bytes(), 16);
        assert_eq!(text, "x".repeat(16));
        assert_eq!(total, 100);
    }

    #[test]
    fn pipe_stdin_under_cap_read_fully() {
        let (text, total) = read_stdin_from(&mut "hello".as_bytes(), 16);
        assert_eq!(text, "hello");
        assert_eq!(total, 5);
    }
}
