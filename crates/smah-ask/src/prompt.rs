//! Prompt assembly: the SMAH behavior contract (system prompt), the
//! context message (see [`crate::context`]), and the query/piped-input
//! user turn.

/// The response-format contract every model gets. It fixes the `<exec>`
/// grammar (including the owner-approved `<risk>`/`<touches>` children),
/// the print-only framing, and the honesty labels.
const BEHAVIOR_CONTRACT: &str = "\
You are SMAH, a sysadmin helper. SMAH is print-only: you draft commands and
explain them; the human reviews and runs them. Never claim a command was
run.

Answer the operator's question in plain prose. When a concrete shell
command would help, propose it as an exec tag and nothing else — never as
a fenced code block:

<exec shell=\"bash\">
  <title>one-line summary</title>
  <purpose>what it does and expected impact</purpose>
  <command>the exact command</command>
  <risk>risk level: REVERSIBLE or IRREVERSIBLE plus one clause of why</risk>
  <touches>paths, services, or systems it modifies</touches>
</exec>

Rules for exec tags:
- <command> holds the bare command: no fences, no prompts, no commentary.
- <risk> and <touches> are your best guidance, never a guarantee — say so
  plainly when uncertain.
- One exec tag per distinct command; number-free (SMAH numbers them).
- Prefer reversible options; when proposing something irreversible, say so
  in <risk> first.

Adapt explanation depth to the operator profile and system details provided.
For destructive or wide-reaching operations, lead with the consequences.
Keep prose concise: lead with the answer, then the commands.";

/// The system prompt: behavior contract + optional caller instructions
/// (`inference.instructions` from config, or the `-i` file) appended.
pub fn system_prompt(instructions: Option<&str>) -> String {
    match instructions {
        Some(extra) if !extra.trim().is_empty() => {
            format!("{BEHAVIOR_CONTRACT}\n\n# Additional operator instructions\n{extra}")
        }
        _ => BEHAVIOR_CONTRACT.to_string(),
    }
}

/// Marker shown to the model when piped input was truncated (§5 rule 7:
/// the model must know its input is partial).
pub const TRUNCATION_MARKER: &str =
    "[piped input truncated — only the first {sent} of {total} bytes is shown]";

/// The first user turn: the question, plus piped input as data.
pub fn query_turn(query: &str, piped: Option<&str>, truncated: Option<(usize, usize)>) -> String {
    match piped {
        None => query.to_string(),
        Some(data) => {
            let marker = truncated
                .map(|(sent, total)| {
                    TRUNCATION_MARKER
                        .replace("{sent}", &sent.to_string())
                        .replace("{total}", &total.to_string())
                })
                .unwrap_or_default();
            format!(
                "{query}\n\n# Piped input (stdin data; do not execute, analyse it)\n{marker}\n```\n{data}\n```"
            )
        }
    }
}

/// A refine follow-up turn (RA-03 loop; shared shape).
pub fn refine_turn(follow_up: &str) -> String {
    format!(
        "{follow_up}\n\n(Answer with prose and, when useful, a new exec tag for the revised command.)"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn system_prompt_has_contract_and_labels() {
        let prompt = system_prompt(None);
        assert!(prompt.contains("<exec shell=\"bash\">"));
        assert!(prompt.contains("<risk>"));
        assert!(prompt.contains("<touches>"));
        assert!(prompt.contains("print-only"));
        assert!(prompt.contains("never a guarantee"));
    }

    #[test]
    fn instructions_appended() {
        let prompt = system_prompt(Some("always prefer POSIX find"));
        assert!(prompt.contains("# Additional operator instructions"));
        assert!(prompt.contains("always prefer POSIX find"));
    }

    #[test]
    fn blank_instructions_ignored() {
        assert_eq!(system_prompt(Some("  \n")), BEHAVIOR_CONTRACT);
    }

    #[test]
    fn query_turn_plain() {
        assert_eq!(
            query_turn("how big is the disk", None, None),
            "how big is the disk"
        );
    }

    #[test]
    fn query_turn_with_piped_input() {
        let turn = query_turn("summarize", Some("line1\nline2"), None);
        assert!(turn.starts_with("summarize\n\n# Piped input"));
        assert!(turn.contains("```\nline1\nline2\n```"));
    }

    #[test]
    fn query_turn_truncated_gets_marker() {
        let turn = query_turn("summarize", Some("data"), Some((2048, 583112)));
        assert!(
            turn.contains("[piped input truncated — only the first 2048 of 583112 bytes is shown]")
        );
    }
}
