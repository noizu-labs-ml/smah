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

pub(crate) fn init_logfile() -> PathBuf {
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
    let mut io = ConsoleIo::new();
    if ask.json {
        // Machine mode (§5): no streaming prose, no block rendering — one
        // JSON object on stdout (still content-only; diagnostics stderr).
        let outcome = flow.turn(&mut io, &[], &query, piped.as_deref(), truncation, false)?;
        io.out_line(
            &serde_json::to_string_pretty(&smah_ask::turn_json(
                &outcome,
                backend.model(),
                backend.provider_key(),
            ))
            .map_err(|err| SmahError::Internal {
                detail: format!("json encode: {err}"),
            })?,
        );
        return Ok(0);
    }
    let outcome = flow.turn(&mut io, &[], &query, piped.as_deref(), truncation, true)?;
    render_suggestion_blocks(&mut io, &outcome);
    // Clipboard handoff (owner decision: auto iff exactly one command;
    // --copy/--no-copy override; confirmations on stderr). JSON mode skips
    // it — machine output has no side effects.
    let config_auto = smah_ask::config_auto(&settings);
    let decision = smah_ask::CopyDecision::resolve(ask.copy, ask.no_copy, config_auto);
    eprintln!("{}", smah_ask::apply_copy(decision, &outcome).stderr_line());

    // On a real terminal the one-shot answer continues into the chat loop
    // (RA-03): the exchange is persisted and the standing keys take over.
    // Piped/JSON runs stay one-shot (§5). Session-store trouble after a
    // delivered answer is a stderr notice, not a failed ask.
    if !ask.json && std::io::stdin().is_terminal() && io.out_is_tty() {
        if let Err(err) = continue_into_loop(
            &flow,
            &settings,
            &query,
            piped.as_deref(),
            truncation,
            &outcome,
        ) {
            eprintln!("smah: interactive follow-up unavailable: {err}");
        }
    }
    Ok(0)
}

/// Persist the delivered exchange and hand control to the chat loop.
fn continue_into_loop(
    flow: &AskFlow<'_>,
    settings: &Settings,
    query: &str,
    piped: Option<&str>,
    truncation: Option<(usize, usize)>,
    outcome: &smah_ask::TurnOutcome,
) -> Result<(), SmahError> {
    let store = smah_ask::SessionStore::open(&crate::chat_cmd::session_db_path(settings))?;
    let session = store.create_session(&crate::chat_cmd::session_title(query))?;
    store.set_last_session(session)?;
    let user_text = smah_ask::query_turn(query, piped, truncation);
    store.append_message(session, &ThreadMessage::user(&user_text))?;
    store.append_message(session, &ThreadMessage::assistant(&outcome.raw))?;
    let mut history = vec![
        ThreadMessage::user(user_text),
        ThreadMessage::assistant(outcome.raw.clone()),
    ];
    let edit_dir = smah_core::virtual_tools::audit::cache_dir();
    let mut io = crate::chat_cmd::ConsoleLoopIo::new();
    let loop_ = smah_ask::ChatLoop {
        flow,
        store: &store,
        session,
        edit_dir: &edit_dir,
    };
    loop_.key_loop(&mut io, &mut history, outcome.clone())
}

/// Settings + provider + model + the key preflight (shared by ask/chat/
/// resume — same precedence as the emulate path). Fails fast with exit 4
/// on a keyless provider, before any network call.
pub(crate) fn build_backend(
    cli: &Cli,
    settings: &Settings,
) -> Result<ProviderAskBackend, SmahError> {
    let model_spec = cli
        .model
        .clone()
        .unwrap_or_else(|| DEFAULT_MODEL_ID.to_string());
    let (provider_key, model_id) = split_model_spec(&model_spec, DEFAULT_PROVIDER);
    let provider = smah_providers::registry::get_provider(&provider_key, settings, None)?;
    let has_creds = provider.has_credentials();
    let backend = ProviderAskBackend {
        inner: ProviderBackend::new(provider),
        model_id,
        provider_key,
        has_creds,
    };
    if !backend.has_credentials() {
        return Err(SmahError::MissingApiKey {
            provider: backend.provider_key().to_string(),
        });
    }
    Ok(backend)
}

/// Real console: content on stdout (flushed per write so streamed prose is
/// live), diagnostics on stderr.
pub(crate) struct ConsoleIo {
    out_tty: bool,
}

impl ConsoleIo {
    pub(crate) fn new() -> Self {
        ConsoleIo {
            out_tty: std::io::stdout().is_terminal(),
        }
    }
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
pub(crate) struct ProviderAskBackend {
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
