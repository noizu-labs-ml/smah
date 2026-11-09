//! `smah chat` + `smah resume` (RA-03): the interactive loop over a
//! persisted session. Interactive-only — a non-TTY invocation is a usage
//! error pointing at `smah ask` (the print-only pipe path). Sessions live
//! in a SQLite store beside the config (`smah.db`, Python-schema
//! compatible); the [e]dit handoff writes under the smah cache dir.

use std::io::{BufRead as _, IsTerminal, Write};
use std::path::PathBuf;

use smah_ask::{outcome_from_history, AskFlow, AskIo, ChatLoop, EgressToggles, LoopIo};
use smah_core::errors::SmahError;
use smah_core::settings::Settings;

use crate::args::Cli;
use crate::ask_cmd::{build_backend, init_logfile, ConsoleIo};

/// Run `smah chat "<opening question>"`; returns the process exit code.
pub fn run_chat(cli: &Cli, query: &[String]) -> i32 {
    let logfile = init_logfile();
    match run_chat_inner(cli, query) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err.render(&logfile.display().to_string()));
            err.exit_code().code()
        }
    }
}

/// Run `smah resume [--last | <id>]`.
pub fn run_resume(cli: &Cli, last: bool, id: Option<i64>) -> i32 {
    let logfile = init_logfile();
    match run_resume_inner(cli, last, id) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("{}", err.render(&logfile.display().to_string()));
            err.exit_code().code()
        }
    }
}

fn require_terminal() -> Result<(), SmahError> {
    if !std::io::stdin().is_terminal() || !std::io::stdout().is_terminal() {
        return Err(SmahError::Usage {
            message: "smah chat/resume need an interactive terminal — for non-interactive use: <command> | smah ask \"question\""
                .to_string(),
        });
    }
    Ok(())
}

fn run_chat_inner(cli: &Cli, query: &[String]) -> Result<i32, SmahError> {
    require_terminal()?;
    let question = query.join(" ");
    if question.trim().is_empty() {
        return Err(SmahError::AskNeedsQuery);
    }
    let settings = load_settings(cli)?;
    let backend = build_backend(cli, &settings)?;
    let flow = AskFlow {
        backend: &backend,
        settings: &settings,
        toggles: EgressToggles::from_settings(&settings),
        instructions: None,
    };
    let store = smah_ask::SessionStore::open(&session_db_path(&settings))?;
    let session = store.create_session(&session_title(&question))?;
    store.set_last_session(session)?;
    let edit_dir = edit_dir();
    let mut io = ConsoleLoopIo::new();
    io.err(&format!("session {session} — {KEYS_SHORT}"));
    let mut history = Vec::new();
    let loop_ = ChatLoop {
        flow: &flow,
        store: &store,
        session,
        edit_dir: &edit_dir,
    };
    let outcome = loop_.start(&mut io, &mut history, &question)?;
    loop_.key_loop(&mut io, &mut history, outcome)?;
    Ok(0)
}

fn run_resume_inner(cli: &Cli, last: bool, id: Option<i64>) -> Result<i32, SmahError> {
    require_terminal()?;
    let settings = load_settings(cli)?;
    let backend = build_backend(cli, &settings)?;
    let flow = AskFlow {
        backend: &backend,
        settings: &settings,
        toggles: EgressToggles::from_settings(&settings),
        instructions: None,
    };
    let store = smah_ask::SessionStore::open(&session_db_path(&settings))?;
    let session = match (last, id) {
        (true, _) => store
            .latest_session()?
            .map(|row| row.id)
            .ok_or_else(|| SmahError::Usage {
                message:
                    "smah: no previous session to resume — start one with `smah chat \"question\"`"
                        .to_string(),
            })?,
        (false, Some(id)) => {
            let known = store.list_sessions(1000)?.iter().any(|row| row.id == id);
            if !known {
                return Err(SmahError::Usage {
                    message: format!(
                        "smah: no session {id} — list recent ids with `smah resume --last`"
                    ),
                });
            }
            id
        }
        (false, None) => {
            return Err(SmahError::Usage {
                message: "smah: resume needs a session id or --last".to_string(),
            })
        }
    };
    store.set_last_session(session)?;
    let history = store.load_session(session)?;
    let edit_dir = edit_dir();
    let mut io = ConsoleLoopIo::new();
    io.out_line(&format!(
        "resuming session {session} — {} messages",
        history.len()
    ));
    let loop_ = ChatLoop {
        flow: &flow,
        store: &store,
        session,
        edit_dir: &edit_dir,
    };
    let mut history = history;
    if history.is_empty() {
        let Some(first) = io.read_line("question: ") else {
            io.err(&smah_ask::resume_footer(session));
            return Ok(0);
        };
        if first.trim().is_empty() {
            io.err(&smah_ask::resume_footer(session));
            return Ok(0);
        }
        let outcome = loop_.start(&mut io, &mut history, first.trim())?;
        loop_.key_loop(&mut io, &mut history, outcome)?;
        return Ok(0);
    }
    let outcome = outcome_from_history(&history).ok_or_else(|| SmahError::Internal {
        detail: "resumed session has no assistant message".to_string(),
    })?;
    loop_.key_loop(&mut io, &mut history, outcome)?;
    Ok(0)
}

fn load_settings(cli: &Cli) -> Result<Settings, SmahError> {
    match &cli.config {
        Some(path) => Settings::load_from(path),
        None => Settings::load(),
    }
}

/// Session store location: beside the active config (`~/.smah/smah.db`
/// with the default config path — Python-parity).
pub(crate) fn session_db_path(settings: &Settings) -> PathBuf {
    settings
        .path()
        .parent()
        .unwrap_or(std::path::Path::new("."))
        .join("smah.db")
}

/// Session title from the opening question (first line, capped).
pub(crate) fn session_title(query: &str) -> String {
    let first_line = query.lines().next().unwrap_or("").trim();
    let cut: String = first_line.chars().take(40).collect();
    if cut.is_empty() {
        "chat".to_string()
    } else {
        cut
    }
}

fn edit_dir() -> PathBuf {
    smah_core::virtual_tools::audit::cache_dir()
}

const KEYS_SHORT: &str = "keys: [c]opy [e]dit [r]efine [n]ext [q]uit";

/// The interactive console: AskIo over the real streams plus the blocking
/// line read (prompt printed to stderr — chrome, not content).
pub(crate) struct ConsoleLoopIo {
    inner: ConsoleIo,
}

impl ConsoleLoopIo {
    pub(crate) fn new() -> Self {
        ConsoleLoopIo {
            inner: ConsoleIo::new(),
        }
    }
}

impl AskIo for ConsoleLoopIo {
    fn out(&mut self, text: &str) {
        self.inner.out(text)
    }
    fn out_line(&mut self, text: &str) {
        self.inner.out_line(text)
    }
    fn err(&mut self, text: &str) {
        self.inner.err(text)
    }
    fn out_is_tty(&self) -> bool {
        self.inner.out_is_tty()
    }
}

impl LoopIo for ConsoleLoopIo {
    fn read_line(&mut self, prompt: &str) -> Option<String> {
        if !prompt.is_empty() {
            let mut stderr = std::io::stderr().lock();
            let _ = stderr.write_all(prompt.as_bytes());
            let _ = stderr.write_all(b" ");
            let _ = stderr.flush();
        }
        let mut line = String::new();
        match std::io::stdin().lock().read_line(&mut line) {
            Ok(0) => None,
            Ok(_) => Some(line.trim_end_matches(['\n', '\r']).to_string()),
            Err(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_take_the_first_forty_chars() {
        assert_eq!(session_title("free disk\nmore context"), "free disk");
        let long = "x".repeat(80);
        assert_eq!(session_title(&long), "x".repeat(40));
        assert_eq!(session_title("   "), "chat");
    }

    #[test]
    fn db_sits_beside_the_config() {
        let dir = tempfile::tempdir().expect("tmp");
        let path = dir.path().join("nested/config.yaml");
        std::fs::create_dir_all(path.parent().unwrap()).expect("dir");
        std::fs::write(&path, "vsn: \"0.0.1\"\n").expect("cfg");
        let settings = Settings::load_from(&path).expect("load");
        assert_eq!(
            session_db_path(&settings),
            dir.path().join("nested/smah.db")
        );
    }
}
