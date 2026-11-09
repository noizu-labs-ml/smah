//! `smah ask "<question>"` — the one-shot ask flow (RA-01): prompt assembly
//! with the operator profile + system context (per the egress toggles), one
//! streaming provider turn (prose live to stdout, `<exec>` bodies held and
//! rendered as numbered suggestion blocks), piped stdin attached as data
//! with a loud truncation notice.
//!
//! Console contract (§5): stdout carries content only (prose + blocks);
//! diagnostics (the standing non-execution notice, truncation, errors) go
//! to stderr. Print-only: nothing here executes anything.

use std::io::{IsTerminal, Read, Write};
use std::path::PathBuf;

use smah_ask::{
    render_suggestion_blocks, AskBackend, AskFlow, AskIo, EgressToggles, ThreadMessage, ThreadRole,
};
use smah_core::errors::SmahError;
use smah_core::settings::Settings;
use smah_core::virtual_tools::driver::ChatBackend;
use smah_core::virtual_tools::DEFAULT_INPUT_BYTES;

use crate::args::{AskArgs, Cli};
use crate::backend::{split_model_spec, ProviderBackend, DEFAULT_MODEL_ID, DEFAULT_PROVIDER};

/// Run `smah ask`; returns the process exit code.
pub fn run_ask(cli: &Cli, ask: &AskArgs) -> i32 {
    let logfile = init_logfile();
    match run_ask_inner(cli, ask) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err.render(&logfile.display().to_string()));
            err.exit_code().code()
        }
    }
}

fn init_logfile() -> PathBuf {
    smah_core::logging::init().unwrap_or_else(|_| PathBuf::from("smah.log"))
}

fn run_ask_inner(cli: &Cli, ask: &AskArgs) -> Result<i32, SmahError> {
    // Stdin: piped data is read fully (capped); a TTY is never read (§5).
    // Truncation is loud, not fatal (§5 rule 7).
    let cap = cli.max_input_chars.unwrap_or(DEFAULT_INPUT_BYTES);
    let (piped, truncation) = read_stdin(cap, std::io::stdin().is_terminal());
    if let Some((sent, total)) = truncation {
        eprintln!("input truncated: sent {sent} of {total} bytes (--max-input-chars to change)");
    }

    // The question itself. Piped data with no question is still a usage
    // error — the model is not sent data with no ask (§4 row 1 class).
    let query = ask.query.join(" ");
    if query.trim().is_empty() {
        if piped.is_some() {
            return Err(SmahError::Usage {
                message: "smah: piped input still needs a question — try: <command> | smah ask \"what does this output mean?\"".to_string(),
            });
        }
        return Err(SmahError::AskNeedsQuery);
    }

    // Settings (missing config = empty settings; the key preflight below
    // turns that into the actionable exit-4 path, same as emulate).
    let settings = match &cli.config {
        Some(path) => Settings::load_from(path)?,
        None => Settings::load()?,
    };

    // Extra instructions: -i file for this ask.
    let instructions = match &ask.instructions {
        Some(path) => match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(err) => {
                return Err(SmahError::Usage {
                    message: format!("smah: cannot read --instructions {}: {err}", path.display()),
                })
            }
        },
        None => None,
    };

    // Provider + model (same precedence as the emulate path).
    let model_spec = cli
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    let (provider_key, model_id) = split_model_spec(&model_spec, DEFAULT_PROVIDER);
    let provider = smah_providers::registry::get_provider(&provider_key, &settings, None)?;
    let has_creds = provider.has_credentials();
    let backend = ProviderAskBackend {
        inner: ProviderBackend::new(provider),
        model_id,
        provider_key,
        has_creds,
    };
    // Fail fast on a keyless provider: exit 4 with guidance, before any
    // network call.
    if !backend.has_credentials() {
        return Err(SmahError::MissingApiKey {
            provider: backend.provider_key().to_string(),
        });
    }

    let flow = AskFlow {
        backend: &backend,
        settings: &settings,
        toggles: EgressToggles::from_settings(&settings),
        instructions,
    };
    let mut io = ConsoleIo {
        out_tty: std::io::stdout().is_terminal(),
    };
    let outcome = flow.turn(&mut io, &[], &query, piped.as_deref(), truncation, true)?;
    render_suggestion_blocks(&mut io, &outcome);
    Ok(0)
}

/// Real console: content on stdout (flushed per write so streamed prose is
/// live), diagnostics on stderr.
struct ConsoleIo {
    out_tty: bool,
}

impl AskIo for ConsoleIo {
    fn out(&mut self, text: &str) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.flush();
    }

    fn out_line(&mut self, text: &str) {
        let mut stdout = std::io::stdout().lock();
        let _ = stdout.write_all(text.as_bytes());
        let _ = stdout.write_all(b"\n");
        let _ = stdout.flush();
    }

    fn err(&mut self, text: &str) {
        let mut stderr = std::io::stderr().lock();
        let _ = stderr.write_all(text.as_bytes());
        let _ = stderr.write_all(b"\n");
    }

    fn out_is_tty(&self) -> bool {
        self.out_tty
    }
}

/// [`AskBackend`] over the providers layer (the sole ask-path file that
/// knows the provider types; mirrors backend.rs).
struct ProviderAskBackend {
    inner: ProviderBackend,
    model_id: String,
    provider_key: String,
    has_creds: bool,
}

impl AskBackend for ProviderAskBackend {
    fn model(&self) -> &str {
        &self.model_id
    }

    fn provider_key(&self) -> &str {
        &self.provider_key
    }

    fn has_credentials(&self) -> bool {
        self.has_creds
    }

    fn stream(
        &self,
        system: &str,
        messages: &[ThreadMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError> {
        use smah_core::virtual_tools::driver::{ChatMessage, Role};
        let wire = messages
            .iter()
            .map(|m| ChatMessage {
                role: match m.role {
                    ThreadRole::User => Role::User,
                    ThreadRole::Assistant => Role::Assistant,
                },
                content: m.content.clone(),
            })
            .collect::<Vec<_>>();
        self.inner
            .stream_turn(&self.model_id, system, &wire, on_delta)
    }
}

/// Capped stdin read (same semantics as the emulate path): `None` piped
/// text on a TTY (never read); `Some((sent, total))` truncation when the
/// pipe exceeded the cap.
fn read_stdin(cap: usize, is_tty: bool) -> (Option<String>, Option<(usize, usize)>) {
    if is_tty {
        return (None, None);
    }
    let mut stdin = std::io::stdin().lock();
    let (text, total) = read_stdin_from(&mut stdin, cap);
    let truncation = (total > cap).then_some((text.len(), total));
    (Some(text), truncation)
}

/// Capped read of an arbitrary reader: (lossy-decoded capped text,
/// uncapped byte total).
fn read_stdin_from<R: Read>(reader: &mut R, cap: usize) -> (String, usize) {
    let mut raw = Vec::new();
    if reader.read_to_end(&mut raw).is_err() {
        return (String::new(), 0);
    }
    let total = raw.len();
    raw.truncate(cap);
    (String::from_utf8_lossy(&raw).to_string(), total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capped_read_reports_total() {
        let mut data = std::io::Cursor::new(b"0123456789".to_vec());
        let (text, total) = read_stdin_from(&mut data, 4);
        assert_eq!(text, "0123");
        assert_eq!(total, 10);
    }

    #[test]
    fn under_cap_read_is_whole() {
        let mut data = std::io::Cursor::new(b"hi".to_vec());
        let (text, total) = read_stdin_from(&mut data, 16);
        assert_eq!(text, "hi");
        assert_eq!(total, 2);
    }
}
