//! Suggestion rendering (interface-spec §3.2 + wireframes §10.1/10.2):
//! numbered blocks with title, command, purpose, model Risk/Touches lines,
//! and the local classifier's red `[!] DESTRUCTIVE` escalation banner.
//!
//! Two shapes:
//!
//! * **TTY** — a rule-delimited block on stdout with an indented command;
//!   ANSI color only when the stream is a TTY (the caller passes `tty`).
//! * **Plain** — markdown-ish lines (wireframe 10.2) for pipes/files/`--json`
//!   consumers; byte-stable, no borders.
//!
//! Prompt/handoff lines (`[c]opy …`, copy confirmations, truncation
//! notices, the standing non-execution notice) are **stderr chrome** and
//! live in the chat/ask layers, not here — stdout carries content only
//! (§5.1).

use serde_json::{json, Value};

use crate::classify::Classification;
use crate::suggestion::Suggestion;

/// Red SGR wrap (destructive escalation) when styling.
fn red(text: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[1;31m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// Bold wrap when styling.
fn bold(text: &str, tty: bool) -> String {
    if tty {
        format!("\x1b[1m{text}\x1b[0m")
    } else {
        text.to_string()
    }
}

/// The standing disclosure (stderr chrome, printed by the CLI layers).
pub const NON_EXECUTION_NOTICE: &str =
    "SMAH does not run commands. Review each suggestion and run it yourself, only if you choose to.";

/// Destructive banner text (owner decision: the local classifier always
/// escalates irreversible-class patterns, labelled guidance).
pub const DESTRUCTIVE_BANNER: &str =
    "[!] DESTRUCTIVE — irreversible-class operation (local static classifier; guidance, not a guarantee)";

/// One rendered suggestion block.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedBlock {
    /// The block itself (content — stdout).
    pub block: String,
    /// The escalation banner line, when the classifier hit (content — it
    /// belongs with the block; stderr styling is the caller's choice).
    pub banner: Option<String>,
}

/// Render suggestion `index` (0-based) of `total` as a TTY block.
pub fn render_block_tty(
    suggestion: &Suggestion,
    index: usize,
    total: usize,
    class: &Classification,
    tty: bool,
) -> RenderedBlock {
    let rule = "─".repeat(72);
    let mut lines: Vec<String> = Vec::new();
    lines.push(rule.clone());
    let header = format!(
        " SUGGESTED COMMAND {}/{} · NOT EXECUTED BY SMAH",
        index + 1,
        total
    );
    lines.push(bold(&header, tty));
    if class.is_destructive() {
        // Escalation line right under the header — impossible to scroll past.
        lines.push(red(&format!(" {}", DESTRUCTIVE_BANNER), tty));
    }
    lines.push(format!(" title:   {}", suggestion.title));
    if let Some(risk) = &suggestion.risk {
        lines.push(format!(
            " risk:    {risk} (model guidance, not a guarantee)"
        ));
    }
    if let Some(touches) = &suggestion.touches {
        lines.push(format!(" touches: {touches}"));
    }
    lines.push(format!(" does:    {}", suggestion.purpose));
    lines.push(String::new());
    for command_line in suggestion.command.lines() {
        lines.push(format!("   {command_line}"));
    }
    lines.push(String::new());
    lines.push(rule);
    RenderedBlock {
        block: lines.join("\n"),
        banner: None,
    }
}

/// Render suggestion `index` (0-based) of `total` as plain text
/// (wireframe 10.2 — pipes, files, redirect consumers).
pub fn render_block_plain(
    suggestion: &Suggestion,
    index: usize,
    total: usize,
    class: &Classification,
) -> RenderedBlock {
    let mut lines: Vec<String> = Vec::new();
    lines.push(format!("## Suggested command {} of {}", index + 1, total));
    if class.is_destructive() {
        lines.push(format!("warning: {DESTRUCTIVE_BANNER}"));
    }
    lines.push(format!("title: {}", suggestion.title));
    if let Some(risk) = &suggestion.risk {
        lines.push(format!("risk: {risk} (model guidance, not a guarantee)"));
    }
    if let Some(touches) = &suggestion.touches {
        lines.push(format!("touches: {touches}"));
    }
    lines.push(format!("does: {}", suggestion.purpose));
    lines.push("command:".to_string());
    lines.push(suggestion.command.clone());
    RenderedBlock {
        block: lines.join("\n"),
        banner: None,
    }
}

/// Machine shape for `--json` (RA-04).
pub fn suggestion_json(suggestion: &Suggestion, class: &Classification) -> Value {
    json!({
        "title": suggestion.title,
        "command": suggestion.command,
        "purpose": suggestion.purpose,
        "risk": suggestion.risk,
        "touches": suggestion.touches,
        "shell": suggestion.shell,
        "classification": {
            "level": match class.level {
                crate::classify::RiskLevel::Benign => "benign",
                crate::classify::RiskLevel::Destructive => "destructive",
            },
            "matched": class.matched,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::classify;

    fn sample() -> Suggestion {
        Suggestion {
            title: "Purge journal logs".to_string(),
            purpose: "Free about 2.1 GB.".to_string(),
            command: "journalctl --vacuum-time=7d".to_string(),
            risk: Some("DELETES FILES — irreversible for the matched set".to_string()),
            touches: Some("/var/log/journal/**".to_string()),
            shell: "bash".to_string(),
        }
    }

    #[test]
    fn plain_block_shape() {
        let class = classify("journalctl --vacuum-time=7d");
        let rendered = render_block_plain(&sample(), 0, 2, &class);
        let expected = "## Suggested command 1 of 2\n\
                        title: Purge journal logs\n\
                        risk: DELETES FILES — irreversible for the matched set (model guidance, not a guarantee)\n\
                        touches: /var/log/journal/**\n\
                        does: Free about 2.1 GB.\n\
                        command:\n\
                        journalctl --vacuum-time=7d";
        assert_eq!(rendered.block, expected);
        assert!(!rendered.block.contains('\x1b'));
    }

    #[test]
    fn destructive_plain_gets_banner() {
        let mut s = sample();
        s.command = "rm -rf /".to_string();
        let class = classify(&s.command);
        assert!(class.is_destructive());
        let rendered = render_block_plain(&s, 0, 1, &class);
        assert!(rendered.block.contains("## Suggested command 1 of 1"));
        assert!(rendered.block.contains("warning: [!] DESTRUCTIVE"));
        assert!(rendered.block.contains("guidance, not a guarantee"));
    }

    #[test]
    fn tty_block_styled_only_when_tty() {
        let mut s = sample();
        s.command = "rm -rf /".to_string();
        let class = classify(&s.command);
        let styled = render_block_tty(&s, 0, 1, &class, true);
        assert!(styled.block.contains("\x1b[1;31m"));
        let plain = render_block_tty(&s, 0, 1, &class, false);
        assert!(!plain.block.contains('\x1b'));
        assert!(plain
            .block
            .contains("SUGGESTED COMMAND 1/1 · NOT EXECUTED BY SMAH"));
        assert!(plain.block.contains("   rm -rf /"));
    }

    #[test]
    fn model_guidance_label_always_present_on_risk() {
        let class = classify("ls");
        let rendered = render_block_plain(&sample(), 0, 1, &class);
        assert!(rendered.block.contains("(model guidance, not a guarantee)"));
    }

    #[test]
    fn json_shape() {
        let class = classify("rm -rf /");
        let value = suggestion_json(&sample(), &class);
        assert_eq!(value["classification"]["level"], "destructive");
        assert_eq!(
            value["classification"]["matched"][0],
            "rm -rf against / or a protected path"
        );
        assert_eq!(value["command"], "journalctl --vacuum-time=7d");
    }

    #[test]
    fn multiline_command_indented_in_tty() {
        let mut s = sample();
        s.command = "for f in *.log; do\n  gzip \"$f\"\ndone".to_string();
        let class = classify(&s.command);
        let rendered = render_block_tty(&s, 0, 1, &class, false);
        assert!(rendered
            .block
            .contains("\n   for f in *.log; do\n     gzip \"$f\"\n   done"));
    }
}
