//! The one-shot ask turn: prompt assembly (behavior contract + context per
//! the egress toggles), a streaming provider call through the
//! [`StreamFilter`] (prose to the console as it arrives, exec bodies held
//! for block rendering), classification, and rendering of the numbered
//! suggestion blocks.
//!
//! All console writes go through [`AskIo`] — the CLI wires the real std
//! streams; tests capture buffers. stdout carries content only (prose +
//! blocks); stderr carries diagnostics (§5).

use smah_core::errors::SmahError;
use smah_core::settings::Settings;

use crate::backend::{AskBackend, ThreadMessage};
use crate::classify::{classify, Classification};
use crate::context::{context_message, EgressToggles};
use crate::prompt::{query_turn, system_prompt};
use crate::render::{render_block_plain, render_block_tty, suggestion_json, NON_EXECUTION_NOTICE};
use crate::stream::{StreamEvent, StreamFilter};
use crate::suggestion::{parse_response, Suggestion};

/// Console seam: content on out, diagnostics on err, TTY-aware rendering.
pub trait AskIo {
    /// Raw stdout write (streaming prose — no newline munging).
    fn out(&mut self, text: &str);
    /// Stdout line (block rendering).
    fn out_line(&mut self, text: &str);
    /// Stderr diagnostic line.
    fn err(&mut self, text: &str);
    /// Whether stdout is a TTY (selects the block style).
    fn out_is_tty(&self) -> bool;
}

/// One completed turn.
#[derive(Debug, Clone)]
pub struct TurnOutcome {
    /// Normalized prose (tags removed, blank runs collapsed).
    pub prose: String,
    /// Suggestions with their local classifications, in order.
    pub suggestions: Vec<(Suggestion, Classification)>,
    /// The full raw model text (tags included) — session persistence keeps
    /// the thread byte-faithful for later turns.
    pub raw: String,
}

/// Rebuild a [`TurnOutcome`] from full model text (used when resuming a
/// session and re-deriving the last turn's blocks).
pub fn outcome_from_text(text: &str) -> TurnOutcome {
    let parsed = parse_response(text);
    let suggestions = parsed
        .suggestions
        .iter()
        .map(|s| (s.clone(), classify(&s.command)))
        .collect();
    TurnOutcome {
        prose: parsed.prose,
        suggestions,
        raw: text.to_string(),
    }
}

impl TurnOutcome {
    /// Count of suggestions carrying a non-empty command (the clipboard
    /// auto-copy rule keys on this).
    pub fn command_count(&self) -> usize {
        self.suggestions
            .iter()
            .filter(|(s, _)| !s.command.trim().is_empty())
            .count()
    }
}

/// The ask engine: backend + settings + egress toggles.
pub struct AskFlow<'a> {
    /// Streaming backend (one provider + model).
    pub backend: &'a dyn AskBackend,
    /// Loaded settings (operator profile / system block source).
    pub settings: &'a Settings,
    /// Egress toggles (`privacy.share_profile` / `privacy.share_system_stats`).
    pub toggles: EgressToggles,
    /// Extra operator instructions (config `inference.instructions` or `-i`).
    pub instructions: Option<String>,
}

impl<'a> AskFlow<'a> {
    /// Run one turn over `history` + the new query (with optional piped
    /// data). When `stream_to_console` is set, prose is printed as it
    /// arrives; otherwise the caller renders the outcome itself
    /// (e.g. `--json`).
    pub fn turn(
        &self,
        io: &mut dyn AskIo,
        history: &[ThreadMessage],
        query: &str,
        piped: Option<&str>,
        truncated: Option<(usize, usize)>,
        stream_to_console: bool,
    ) -> Result<TurnOutcome, SmahError> {
        let mut system = system_prompt(self.instructions.as_deref());
        if let Some(context) = context_message(self.settings, &self.toggles) {
            system = format!("{system}\n\n{context}");
        }
        let mut messages = history.to_vec();
        messages.push(ThreadMessage::user(query_turn(query, piped, truncated)));

        let mut filter = StreamFilter::new();
        let mut exec_bodies: Vec<String> = Vec::new();
        let full = self.backend.stream(&system, &messages, &mut |delta| {
            for event in filter.feed(delta) {
                match event {
                    StreamEvent::Prose(text) => {
                        if stream_to_console {
                            io.out(&text);
                        }
                    }
                    StreamEvent::Exec(body) => exec_bodies.push(body),
                }
            }
        })?;
        for event in filter.finish() {
            if let StreamEvent::Prose(text) = event {
                if stream_to_console {
                    io.out(&text);
                }
            }
        }
        if stream_to_console && !full.trim().is_empty() {
            io.out("\n\n");
        }

        Ok(outcome_from_text(&full))
    }
}

/// Render the suggestion blocks of a turn to stdout (TTY-aware), with the
/// standing non-execution notice on stderr once when any suggestion exists.
pub fn render_suggestion_blocks(io: &mut dyn AskIo, outcome: &TurnOutcome) {
    let total = outcome.suggestions.len();
    if total == 0 {
        return;
    }
    io.err(NON_EXECUTION_NOTICE);
    for (index, (suggestion, class)) in outcome.suggestions.iter().enumerate() {
        let rendered = if io.out_is_tty() {
            render_block_tty(suggestion, index, total, class, true).block
        } else {
            render_block_plain(suggestion, index, total, class).block
        };
        for line in rendered.lines() {
            io.out_line(line);
        }
        io.out_line("");
    }
}

/// Machine output for `--json`: one object with the answer and every
/// suggestion (classification included).
pub fn turn_json(outcome: &TurnOutcome, model: &str, provider: &str) -> serde_json::Value {
    serde_json::json!({
        "model": model,
        "provider": provider,
        "answer": outcome.prose,
        "suggestions": outcome
            .suggestions
            .iter()
            .map(|(s, c)| suggestion_json(s, c))
            .collect::<Vec<_>>(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::ScriptedBackend;

    /// Buffer-backed console: records out/err separately.
    struct BufferIo {
        out: String,
        err: Vec<String>,
        tty: bool,
    }

    impl BufferIo {
        fn new(tty: bool) -> Self {
            BufferIo {
                out: String::new(),
                err: Vec::new(),
                tty,
            }
        }
    }

    impl AskIo for BufferIo {
        fn out(&mut self, text: &str) {
            self.out.push_str(text);
        }
        fn out_line(&mut self, text: &str) {
            self.out.push_str(text);
            self.out.push('\n');
        }
        fn err(&mut self, text: &str) {
            self.err.push(text.to_string());
        }
        fn out_is_tty(&self) -> bool {
            self.tty
        }
    }

    fn settings(text: &str) -> Settings {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.yaml");
        std::fs::write(&path, text).expect("write");
        Settings::load_from(&path).expect("load")
    }

    const CONFIG: &str = "vsn: \"0.0.1\"\nuser:\n  vsn: \"0.0.1\"\n  name: Dana\n  system_admin_experience: Novice\n  role: Developer\n  about: learning ops\n";

    const ANSWER: &str = "Purge old journals like this:\n<exec shell=\"bash\">\n<title>Purge journal logs</title>\n<purpose>Drop entries older than 7 days.</purpose>\n<command>journalctl --vacuum-time=7d</command>\n<risk>IRREVERSIBLE — deletes matched entries</risk>\n<touches>/var/log/journal/**</touches>\n</exec>";

    #[test]
    fn turn_streams_prose_and_collects_suggestions() {
        let backend = ScriptedBackend::once(ANSWER);
        let settings = settings(CONFIG);
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(true);
        let outcome = flow
            .turn(&mut io, &[], "free disk", None, None, true)
            .expect("turn");
        assert!(io.out.starts_with("Purge old journals like this:"));
        assert!(!io.out.contains("<exec"));
        assert_eq!(outcome.suggestions.len(), 1);
        let (s, c) = &outcome.suggestions[0];
        assert_eq!(s.command, "journalctl --vacuum-time=7d");
        assert_eq!(c.level, crate::classify::RiskLevel::Benign);
    }

    #[test]
    fn system_prompt_carries_context_when_shared() {
        let backend = ScriptedBackend::once("ok");
        let settings = settings(CONFIG);
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        flow.turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        let system = backend.seen_system.lock().unwrap()[0].clone();
        assert!(system.contains("name: Dana"));
        assert!(system.contains("# System — about this system"));
    }

    #[test]
    fn toggles_off_mean_no_context_egress() {
        let backend = ScriptedBackend::once("ok");
        let settings = settings(CONFIG);
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles {
                share_profile: false,
                share_system_stats: false,
            },
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        flow.turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        let system = backend.seen_system.lock().unwrap()[0].clone();
        assert!(!system.contains("name: Dana"));
        assert!(!system.contains("# System"));
        assert!(system.contains("<exec"));
    }

    #[test]
    fn piped_input_in_the_user_turn_with_marker() {
        let backend = ScriptedBackend::once("ok");
        let settings = settings("vsn: \"0.0.1\"\n");
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        flow.turn(&mut io, &[], "sum", Some("data"), Some((4, 100)), false)
            .expect("turn");
        let thread = &backend.seen_threads.lock().unwrap()[0];
        let turn = &thread[0].content;
        assert!(turn.contains("# Piped input"));
        assert!(turn.contains("only the first 4 of 100 bytes"));
    }

    #[test]
    fn blocks_render_with_notice_on_stderr() {
        let backend = ScriptedBackend::once(ANSWER);
        let settings = settings("vsn: \"0.0.1\"\n");
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        let outcome = flow
            .turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        render_suggestion_blocks(&mut io, &outcome);
        assert!(io.out.contains("## Suggested command 1 of 1"));
        assert!(io.out.contains("journalctl --vacuum-time=7d"));
        assert!(io.out.contains(
            "risk: IRREVERSIBLE — deletes matched entries (model guidance, not a guarantee)"
        ));
        assert_eq!(io.err, vec![NON_EXECUTION_NOTICE.to_string()]);
        assert!(!io.out.contains('\x1b'));
    }

    #[test]
    fn destructive_command_escalates_in_block() {
        let answer = "<exec><command>rm -rf /</command><risk>safe-ish</risk></exec>";
        let backend = ScriptedBackend::once(answer);
        let settings = settings("vsn: \"0.0.1\"\n");
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        let outcome = flow
            .turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        render_suggestion_blocks(&mut io, &outcome);
        assert!(io.out.contains("warning: [!] DESTRUCTIVE"));
    }

    #[test]
    fn json_shape_complete() {
        let backend = ScriptedBackend::once(ANSWER);
        let settings = settings("vsn: \"0.0.1\"\n");
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        let outcome = flow
            .turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        let value = turn_json(&outcome, "claude-opus-5-5", "anthropic");
        assert_eq!(value["provider"], "anthropic");
        assert_eq!(
            value["suggestions"][0]["command"],
            "journalctl --vacuum-time=7d"
        );
        assert_eq!(value["suggestions"][0]["classification"]["level"], "benign");
    }

    #[test]
    fn command_count_counts_nonempty_commands() {
        let backend = ScriptedBackend::once("a<exec><command>x</command></exec>b<exec></exec>");
        let settings = settings("vsn: \"0.0.1\"\n");
        let flow = AskFlow {
            backend: &backend,
            settings: &settings,
            toggles: EgressToggles::default(),
            instructions: None,
        };
        let mut io = BufferIo::new(false);
        let outcome = flow
            .turn(&mut io, &[], "q", None, None, false)
            .expect("turn");
        assert_eq!(outcome.command_count(), 1);
    }
}
