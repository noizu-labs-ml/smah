//! Clipboard handoff (owner decision 2026-11-09): the execute affordance
//! in a print-only tool. Auto-copy **iff** the answer contains exactly one
//! command; `--copy`/`--no-copy` override; multi-command answers are not
//! bulk-copied (per-block `[c]opy` lives in the interactive loop, RA-03).
//!
//! Pure-Rust `arboard` — never `pbcopy`/`xclip` subprocesses (the
//! no-shell gate). Confirmations are stderr chrome; the copied text is the
//! command itself (never a secret).

use crate::ask::TurnOutcome;
use smah_core::settings::Settings;

/// `clipboard.auto` from config (default on).
pub fn config_auto(settings: &Settings) -> bool {
    use serde_yaml_ng::Value;
    settings
        .value()
        .get(Value::String("clipboard".to_string()))
        .and_then(|v| v.get(Value::String("auto".to_string())))
        .and_then(|v| v.as_bool())
        .unwrap_or(true)
}

/// Whether to attempt a copy this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CopyDecision {
    /// Config default (`clipboard.auto`, default on): copy iff exactly one
    /// command.
    Auto,
    /// `--copy`: force the attempt (still single-command only — a bulk copy
    /// of several commands is a paste hazard, not a convenience).
    Force,
    /// `--no-copy`: never.
    Never,
}

impl CopyDecision {
    /// Resolve the CLI flags + config default. `--no-copy` wins over
    /// `--copy` (the cautious flag outranks the eager one); both win over
    /// the config.
    pub fn resolve(copy: bool, no_copy: bool, config_auto: bool) -> Self {
        if no_copy {
            CopyDecision::Never
        } else if copy {
            CopyDecision::Force
        } else if config_auto {
            CopyDecision::Auto
        } else {
            CopyDecision::Never
        }
    }
}

/// What the copy step decided, for the stderr confirmation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CopyResult {
    /// The command now on the clipboard.
    Copied(String),
    /// Nothing copied, and why (stderr chrome).
    NotCopied(&'static str),
}

impl CopyResult {
    /// The stderr line for this outcome (content never on stdout).
    pub fn stderr_line(&self) -> String {
        match self {
            CopyResult::Copied(command) => format!("copied to clipboard: {command}"),
            CopyResult::NotCopied(reason) => format!("clipboard: {reason}"),
        }
    }
}

/// Decide + perform the copy for a finished turn. The clipboard write
/// itself is best-effort: a failure is a stderr note, never a changed exit
/// code (§5 — the answer already succeeded).
pub fn apply_copy(decision: CopyDecision, outcome: &TurnOutcome) -> CopyResult {
    let commands: Vec<&str> = outcome
        .suggestions
        .iter()
        .filter(|(s, _)| !s.command.trim().is_empty())
        .map(|(s, _)| s.command.trim())
        .collect();
    match decision {
        CopyDecision::Never => CopyResult::NotCopied("disabled for this run (--no-copy)"),
        CopyDecision::Auto | CopyDecision::Force => match commands.len() {
            0 => CopyResult::NotCopied("no commands in this answer to copy"),
            1 => match copy_to_clipboard(commands[0]) {
                Ok(()) => CopyResult::Copied(commands[0].to_string()),
                Err(reason) => CopyResult::NotCopied(reason),
            },
            _ => CopyResult::NotCopied(
                "answer has multiple commands — use the interactive loop (smah chat) for per-block copy",
            ),
        },
    }
}

/// Write `text` to the system clipboard (arboard; no subprocess).
pub fn copy_to_clipboard(text: &str) -> Result<(), &'static str> {
    let mut board = arboard::Clipboard::new().map_err(|_| "clipboard unavailable")?;
    board
        .set_text(text.to_string())
        .map_err(|_| "clipboard write failed")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;
    use crate::suggestion::{parse_response, Suggestion};

    fn outcome_for(answer: &str) -> TurnOutcome {
        let parsed = parse_response(answer);
        TurnOutcome {
            prose: parsed.prose,
            suggestions: parsed
                .suggestions
                .iter()
                .map(|s: &Suggestion| (s.clone(), classify(&s.command)))
                .collect(),
        }
    }

    #[test]
    fn no_copy_outranks_copy() {
        assert_eq!(CopyDecision::resolve(true, true, true), CopyDecision::Never);
        assert_eq!(
            CopyDecision::resolve(true, false, false),
            CopyDecision::Force
        );
        assert_eq!(
            CopyDecision::resolve(false, false, true),
            CopyDecision::Auto
        );
        assert_eq!(
            CopyDecision::resolve(false, false, false),
            CopyDecision::Never
        );
        assert_eq!(
            CopyDecision::resolve(true, true, false),
            CopyDecision::Never
        );
    }

    #[test]
    fn auto_with_single_command_attempts_copy() {
        let outcome = outcome_for("here:\n<exec><command>ls -la</command></exec>");
        match apply_copy(CopyDecision::Auto, &outcome) {
            CopyResult::Copied(cmd) => assert_eq!(cmd, "ls -la"),
            CopyResult::NotCopied(reason) => {
                // A headless test host may have no clipboard — the reason
                // must be the availability note, not a policy one.
                assert!(reason.contains("clipboard"), "reason: {reason}");
            }
        }
    }

    #[test]
    fn multiple_commands_are_never_bulk_copied() {
        let answer =
            "a:\n<exec><command>ls</command></exec>\nb:\n<exec><command>pwd</command></exec>";
        let outcome = outcome_for(answer);
        let result = apply_copy(CopyDecision::Force, &outcome);
        assert_eq!(
            result,
            CopyResult::NotCopied(
                "answer has multiple commands — use the interactive loop (smah chat) for per-block copy"
            )
        );
        assert!(result.stderr_line().contains("multiple commands"));
    }

    #[test]
    fn no_commands_means_no_copy() {
        let outcome = outcome_for("just prose, nothing to run");
        let result = apply_copy(CopyDecision::Auto, &outcome);
        assert_eq!(
            result,
            CopyResult::NotCopied("no commands in this answer to copy")
        );
    }

    #[test]
    fn never_is_silent_about_commands() {
        let outcome = outcome_for("x:\n<exec><command>ls</command></exec>");
        let result = apply_copy(CopyDecision::Never, &outcome);
        assert_eq!(
            result,
            CopyResult::NotCopied("disabled for this run (--no-copy)")
        );
    }

    #[test]
    fn empty_command_blocks_do_not_count() {
        let outcome = outcome_for("a:\n<exec><command>ls</command></exec>b:\n<exec></exec>");
        assert_eq!(outcome.command_count(), 1);
        match apply_copy(CopyDecision::Auto, &outcome) {
            CopyResult::Copied(cmd) => assert_eq!(cmd, "ls"),
            CopyResult::NotCopied(reason) => {
                assert!(reason.contains("clipboard"), "reason: {reason}")
            }
        }
    }
}
