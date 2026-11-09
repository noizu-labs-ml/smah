//! The interactive chat loop (RA-03): turns over a persisted session with
//! the standing keys — [c]opy, [e]dit, [r]efine, [n]ext, [q]uit. Every exit
//! path (quit, EOF, or an error surfaced by the caller) follows a footer
//! naming the resume command. Print-only throughout: [e]dit is a
//! file-handoff (the operator edits a file; smah re-reads it, re-classifies
//! locally, and asks the model to re-explain) — no `$EDITOR` is ever
//! spawned.

use std::path::{Path, PathBuf};

use smah_core::errors::SmahError;

use crate::ask::{outcome_from_text, render_suggestion_blocks, AskFlow, AskIo, TurnOutcome};
use crate::backend::{ThreadMessage, ThreadRole};
use crate::classify::classify;
use crate::clipboard::copy_to_clipboard;
use crate::prompt::{edit_reexplain_turn, query_turn, refine_turn};
use crate::render::render_block_plain;
use crate::session::SessionStore;
use crate::suggestion::Suggestion;

/// Console + input seam for the loop: everything [`AskIo`] does, plus one
/// blocking line read (prompt text is chrome — stderr-side).
pub trait LoopIo: AskIo {
    /// Read one line (newline trimmed). `None` = EOF.
    fn read_line(&mut self, prompt: &str) -> Option<String>;
}

/// The standing key help line (stderr chrome).
pub const KEYS_LINE: &str = "keys: [c]opy  [e]dit  [r]efine  [n]ext  [q]uit";

/// The exit footer, printed on every loop exit (quit and EOF included).
pub fn resume_footer(session: i64) -> String {
    format!("session {session} saved — resume with `smah resume {session}`")
}

/// The interactive loop over one stored session.
pub struct ChatLoop<'a> {
    /// The ask engine (backend + settings + toggles).
    pub flow: &'a AskFlow<'a>,
    /// The session store (messages persist as they happen).
    pub store: &'a SessionStore,
    /// The live session id.
    pub session: i64,
    /// Directory for [e]dit handoff files (created on demand).
    pub edit_dir: &'a Path,
}

impl<'a> ChatLoop<'a> {
    /// Where [e]dit writes the selected command.
    fn edit_file(&self) -> PathBuf {
        self.edit_dir
            .join(format!("smah-session-{}.cmd", self.session))
    }

    /// Run one model turn over `history` (appending both messages to the
    /// store and the in-memory thread), render the blocks, return the
    /// outcome. `user_text` is what the model saw — wrappers like
    /// [`refine_turn`] included.
    pub fn run_turn(
        &self,
        io: &mut dyn LoopIo,
        history: &mut Vec<ThreadMessage>,
        user_text: &str,
    ) -> Result<TurnOutcome, SmahError> {
        let outcome = self.flow.turn(io, history, user_text, None, None, true)?;
        let assistant = ThreadMessage::assistant(&outcome.raw);
        self.store
            .append_message(self.session, &ThreadMessage::user(user_text))?;
        self.store.append_message(self.session, &assistant)?;
        history.push(ThreadMessage::user(user_text));
        history.push(assistant);
        render_suggestion_blocks(io, &outcome);
        Ok(outcome)
    }

    /// First turn of a fresh session: the opening question.
    pub fn start(
        &self,
        io: &mut dyn LoopIo,
        history: &mut Vec<ThreadMessage>,
        query: &str,
    ) -> Result<TurnOutcome, SmahError> {
        self.run_turn(io, history, &query_turn(query, None, None))
    }

    /// Enter the key loop over an already-rendered turn (chat start, ask
    /// continuation, resume).
    pub fn key_loop(
        &self,
        io: &mut dyn LoopIo,
        history: &mut Vec<ThreadMessage>,
        mut outcome: TurnOutcome,
    ) -> Result<(), SmahError> {
        loop {
            io.err(KEYS_LINE);
            let Some(line) = io.read_line("> ") else {
                self.footer(io);
                return Ok(());
            };
            match line.trim() {
                "q" | "quit" | "exit" => {
                    self.footer(io);
                    return Ok(());
                }
                "c" => self.copy_flow(io, &outcome)?,
                "e" => outcome = self.edit_flow(io, history, outcome)?,
                "r" => {
                    let Some(follow_up) = io.read_line("refine: ") else {
                        self.footer(io);
                        return Ok(());
                    };
                    if follow_up.trim().is_empty() {
                        io.err("refine: empty — nothing sent");
                        continue;
                    }
                    outcome = self.run_turn(io, history, &refine_turn(follow_up.trim()))?;
                }
                "n" => {
                    let Some(next) = io.read_line("question: ") else {
                        self.footer(io);
                        return Ok(());
                    };
                    if next.trim().is_empty() {
                        io.err("next: empty — nothing sent");
                        continue;
                    }
                    outcome = self.start(io, history, next.trim())?;
                }
                "" => continue,
                other => {
                    io.err(&format!("unknown key '{other}' — {KEYS_LINE}"));
                }
            }
        }
    }

    fn footer(&self, io: &mut dyn LoopIo) {
        io.err(&resume_footer(self.session));
    }

    /// [c]opy: single command copies directly; multiple prompt for an
    /// index (never bulk — the multi-copy rule is owner-set).
    fn copy_flow(&self, io: &mut dyn LoopIo, outcome: &TurnOutcome) -> Result<(), SmahError> {
        let Some(index) = self.select_command(io, outcome, "copy") else {
            return Ok(());
        };
        let (suggestion, _) = &outcome.suggestions[index];
        let command = suggestion.command.clone();
        match copy_to_clipboard(&command) {
            Ok(()) => io.err(&format!("copied to clipboard: {command}")),
            Err(reason) => io.err(&format!("clipboard: {reason}")),
        }
        Ok(())
    }

    /// [e]dit: write the selected command to a file, wait for the operator,
    /// re-read, re-classify locally, and — when changed — run a model turn
    /// asking for a re-explanation of the edit.
    fn edit_flow(
        &self,
        io: &mut dyn LoopIo,
        history: &mut Vec<ThreadMessage>,
        outcome: TurnOutcome,
    ) -> Result<TurnOutcome, SmahError> {
        let Some(index) = self.select_command(io, &outcome, "edit") else {
            return Ok(outcome);
        };
        let original = outcome.suggestions[index].0.command.clone();
        let file = self.edit_file();
        std::fs::create_dir_all(self.edit_dir).map_err(|err| SmahError::Internal {
            detail: format!("edit handoff dir: {err}"),
        })?;
        // F6: the handoff file can carry a destructive command — write it
        // atomically with 0600 (unix) like every other private file.
        smah_core::secrets::write_private_file_atomic(&file, original.as_bytes()).map_err(
            |err| SmahError::Internal {
                detail: format!("edit handoff write: {err}"),
            },
        )?;
        io.err(&format!(
            "edit: {} — change the file, then press Enter",
            file.display()
        ));
        if io.read_line("").is_none() {
            self.footer(io);
            return Ok(outcome);
        }
        let edited = std::fs::read_to_string(&file)
            .map_err(|err| SmahError::Internal {
                detail: format!("edit handoff read: {err}"),
            })?
            .trim()
            .to_string();
        if edited == original.trim() {
            io.err("edit: unchanged");
            return Ok(outcome);
        }
        let class = classify(&edited);
        let banner = if class.is_destructive() {
            "[!] DESTRUCTIVE"
        } else {
            "within guidance"
        };
        io.err(&format!(
            "edit: re-classified as {banner} (guidance, not a guarantee)"
        ));
        let replacement = Suggestion {
            title: "Edited command".to_string(),
            purpose: "Operator-edited variant of the suggested command.".to_string(),
            command: edited.clone(),
            risk: outcome.suggestions[index].0.risk.clone(),
            touches: outcome.suggestions[index].0.touches.clone(),
            shell: outcome.suggestions[index].0.shell.clone(),
        };
        for line in render_block_plain(&replacement, index, 1, &class)
            .block
            .lines()
        {
            io.out_line(line);
        }
        io.out_line("");
        let mut next = self.run_turn(io, history, &edit_reexplain_turn(&original, &edited))?;
        if next.suggestions.is_empty() {
            // The model answered without a fresh exec tag; keep the edited
            // command visible as the operative suggestion.
            next.suggestions.push((replacement, class));
            render_suggestion_blocks(io, &next);
        }
        Ok(next)
    }

    /// Command selection shared by [c]opy and [e]dit: zero commands is a
    /// stderr note; one copies/edits directly; many prompt for 1-N.
    fn select_command(
        &self,
        io: &mut dyn LoopIo,
        outcome: &TurnOutcome,
        verb: &str,
    ) -> Option<usize> {
        let with_commands: Vec<usize> = outcome
            .suggestions
            .iter()
            .enumerate()
            .filter(|(_, (s, _))| !s.command.trim().is_empty())
            .map(|(i, _)| i)
            .collect();
        match with_commands.len() {
            0 => {
                io.err(&format!("{verb}: no commands in this answer"));
                None
            }
            1 => Some(with_commands[0]),
            _ => {
                io.err(&format!(
                    "{verb}: several commands — which one? (1-{}):",
                    with_commands.len()
                ));
                let line = io.read_line("")?;
                match line.trim().parse::<usize>() {
                    Ok(n) if (1..=with_commands.len()).contains(&n) => Some(with_commands[n - 1]),
                    _ => {
                        io.err(&format!("{verb}: not a valid pick"));
                        None
                    }
                }
            }
        }
    }
}

/// Rebuild the last turn's outcome from a stored assistant message (resume
/// path — the blocks are not re-rendered, but [c]opy/[e]dit operate on
/// them).
pub fn outcome_from_history(history: &[ThreadMessage]) -> Option<TurnOutcome> {
    history
        .iter()
        .rev()
        .find(|m| m.role == ThreadRole::Assistant)
        .map(|m| outcome_from_text(&m.content))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ask::AskFlow;
    use crate::backend::{ScriptedBackend, ThreadRole};
    use crate::context::EgressToggles;
    use smah_core::settings::Settings;

    /// Prompt hook: rewrite files (or anything else) at the moment a given
    /// prompt is read, then supply the line.
    type PromptHook = Box<dyn Fn(&str) -> Option<String> + 'static>;

    /// Scripted console: captured out/err plus a queue of input lines.
    struct ScriptedIo {
        out: Vec<String>,
        err: Vec<String>,
        lines: std::collections::VecDeque<String>,
        on_prompt: Option<PromptHook>,
    }

    impl ScriptedIo {
        fn new(lines: &[&str]) -> Self {
            ScriptedIo {
                out: Vec::new(),
                err: Vec::new(),
                lines: lines.iter().map(|s| s.to_string()).collect(),
                on_prompt: None,
            }
        }

        fn joined_err(&self) -> String {
            self.err.join("\n")
        }
    }

    impl AskIo for ScriptedIo {
        fn out(&mut self, text: &str) {
            self.out.push(text.to_string());
        }
        fn out_line(&mut self, text: &str) {
            self.out.push(format!("{text}\n"));
        }
        fn err(&mut self, text: &str) {
            self.err.push(text.to_string());
        }
        fn out_is_tty(&self) -> bool {
            false
        }
    }

    impl LoopIo for ScriptedIo {
        fn read_line(&mut self, prompt: &str) -> Option<String> {
            if let Some(hook) = &self.on_prompt {
                if let Some(line) = hook(prompt) {
                    return Some(line);
                }
            }
            self.lines.pop_front()
        }
    }

    fn harness(
        answer: &str,
        lines: &[&str],
    ) -> (
        ScriptedBackend,
        ScriptedIo,
        tempfile::TempDir,
        SessionStore,
        Settings,
    ) {
        let dir = tempfile::tempdir().expect("tmp");
        let store = SessionStore::open(&dir.path().join("smah.db")).expect("store");
        let config = dir.path().join("config.yaml");
        std::fs::write(&config, "vsn: \"0.0.1\"\n").expect("cfg");
        let settings = Settings::load_from(&config).expect("empty settings");
        (
            ScriptedBackend::once(answer),
            ScriptedIo::new(lines),
            dir,
            store,
            settings,
        )
    }

    fn flow<'b>(backend: &'b ScriptedBackend, settings: &'b Settings) -> AskFlow<'b> {
        AskFlow {
            backend,
            settings,
            toggles: EgressToggles::default(),
            instructions: None,
        }
    }

    const ANSWER: &str = "Clean journals like this:\n<exec>\n<title>Purge journals</title>\n<command>journalctl --vacuum-time=7d</command>\n</exec>\nThat reclaims space.";

    #[test]
    fn quit_after_first_turn_persists_and_footers() {
        let (backend, mut io, dir, store, settings) = harness(ANSWER, &["q"]);
        let session = store.create_session("disk").expect("create");
        store.set_last_session(session).expect("last");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        loop_
            .start(&mut io, &mut history, "free disk?")
            .expect("turn");
        let outcome = outcome_from_history(&history).unwrap();
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");

        let messages = store.load_session(session).expect("load");
        assert_eq!(messages.len(), 2, "user + assistant persisted");
        assert_eq!(messages[0].role, ThreadRole::User);
        assert!(
            messages[1].content.contains("journalctl"),
            "assistant raw kept"
        );
        assert!(
            io.joined_err().contains(&resume_footer(session)),
            "footer on quit"
        );
        assert_eq!(store.last_session().expect("pointer"), Some(session));
    }

    #[test]
    fn eof_exits_with_footer() {
        let (backend, mut io, dir, store, settings) = harness(ANSWER, &[]);
        let session = store.create_session("eof").expect("create");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        loop_.start(&mut io, &mut history, "q1").expect("turn");
        let outcome = outcome_from_history(&history).unwrap();
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");
        assert!(
            io.joined_err().contains(&resume_footer(session)),
            "footer on EOF"
        );
    }

    #[test]
    fn refine_sends_followup_over_the_same_thread() {
        let (backend, mut io, dir, store, settings) =
            harness(ANSWER, &["r", "and journald config?", "q"]);
        let session = store.create_session("refine").expect("create");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        loop_
            .start(&mut io, &mut history, "free disk?")
            .expect("turn");
        let outcome = outcome_from_history(&history).unwrap();
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");

        let threads = backend.seen_threads.lock().expect("threads").clone();
        assert_eq!(threads.len(), 2, "two model turns");
        let second = &threads[1];
        assert_eq!(second.len(), 3, "history rides the second turn");
        assert_eq!(second[0].role, ThreadRole::User);
        assert_eq!(second[1].role, ThreadRole::Assistant);
        assert!(
            second[1].content.contains("journalctl"),
            "assistant turn in thread"
        );
        // The follow-up is wrapped by refine_turn.
        assert!(second
            .last()
            .unwrap()
            .content
            .contains("and journald config?"));
        assert!(
            store.load_session(session).expect("load").len() == 4,
            "4 messages stored"
        );
    }

    #[test]
    fn copy_with_multiple_commands_prompts_for_an_index() {
        let answer = "two:\n<exec>\n<command>df -h</command>\n</exec>\nand:\n<exec>\n<command>du -sh .</command>\n</exec>\n";
        let (backend, mut io, dir, store, settings) = harness(answer, &["c", "2", "q"]);
        let session = store.create_session("multi").expect("create");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        let outcome = loop_.start(&mut io, &mut history, "disk?").expect("turn");
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");

        let err = io.joined_err();
        assert!(err.contains("several commands"), "index prompt: {err}");
        assert!(
            err.contains("du -sh .")
                || err.contains("clipboard unavailable")
                || err.contains("clipboard write failed"),
            "picked second command: {err}"
        );
    }

    #[test]
    fn edit_unchanged_notes_and_keeps_the_answer() {
        let (backend, mut io, dir, store, settings) = harness(ANSWER, &["e", "", "q"]);
        let session = store.create_session("edit-same").expect("create");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        let outcome = loop_
            .start(&mut io, &mut history, "free disk?")
            .expect("turn");
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");

        let err = io.joined_err();
        assert!(err.contains("edit:"), "edit chrome: {err}");
        assert!(err.contains("unchanged"), "unchanged note: {err}");
        assert!(loop_.edit_file().exists(), "handoff file written");
        assert_eq!(
            store.load_session(session).expect("load").len(),
            2,
            "no extra turns"
        );
    }

    #[test]
    fn edit_changed_reclassifies_and_asks_the_model_to_reexplain() {
        let (backend, mut io, dir, store, settings) = harness(ANSWER, &[]);
        let session = store.create_session("edit-changed").expect("create");
        let edit_dir = dir.path().join("edit");
        let file = edit_dir.join(format!("smah-session-{session}.cmd"));
        // On the "press Enter" prompt, rewrite the handoff file.
        io.on_prompt = Some(Box::new(move |prompt| {
            if prompt.is_empty() {
                std::fs::create_dir_all(file.parent().expect("parent")).expect("dir");
                std::fs::write(&file, "journalctl --vacuum-time=30d").expect("edited");
                Some(String::new())
            } else {
                None
            }
        }));
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        // First turn scripted inline: queue the [e] key by hand.
        let outcome = loop_
            .start(&mut io, &mut history, "free disk?")
            .expect("turn");
        io.lines.clear();
        io.lines.push_back("e".to_string());
        io.lines.push_back("q".to_string());
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");

        let err = io.joined_err();
        assert!(err.contains("re-classified"), "local re-classify: {err}");
        assert!(err.contains("guidance, not a guarantee"), "{err}");
        let threads = backend.seen_threads.lock().expect("threads").clone();
        assert_eq!(threads.len(), 2, "model re-explain turn ran");
        assert!(
            threads[1]
                .last()
                .unwrap()
                .content
                .contains("--vacuum-time=30d"),
            "the edit reached the model"
        );
    }

    #[test]
    fn unknown_key_is_rejected_with_the_keys_line() {
        let (backend, mut io, dir, store, settings) = harness(ANSWER, &["x", "q"]);
        let session = store.create_session("unknown").expect("create");
        let edit_dir = dir.path().join("edit");
        let flow = flow(&backend, &settings);
        let mut history = Vec::new();
        let loop_ = ChatLoop {
            flow: &flow,
            store: &store,
            session,
            edit_dir: &edit_dir,
        };
        let outcome = loop_.start(&mut io, &mut history, "q?").expect("turn");
        loop_
            .key_loop(&mut io, &mut history, outcome)
            .expect("loop");
        assert!(
            io.joined_err().contains("unknown key 'x'"),
            "guidance: {}",
            io.joined_err()
        );
    }
}
