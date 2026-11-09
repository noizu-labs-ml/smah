//! asciinema v2 `.cast` parsing (SW-04) — pure data transformation, no exec.
//!
//! The v2 format is JSON Lines: a header object
//! (`{"version": 2, "width": 80, "height": 24, "env": {...}}`) followed by
//! event arrays `[<time>, "<code>", "<data>"]` where the codes are
//! `o` (output written to the terminal), `i` (input), `r` (resize), and
//! `m` (marker). Only `o` events carry terminal output.
//!
//! Strictness: the header must be a JSON object with `version == 2`
//! (anything else is "not an asciinema v2 cast"); every subsequent
//! non-empty line must parse as a 3-element array whose second element is a
//! string; a violation names the 1-based line number. Unknown event codes
//! are skipped per the format spec (forward compatibility), and non-`o`
//! events never contribute text.

use serde_json::Value;

/// One parsed output event (time in seconds from the recording start).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputEvent {
    pub time: f64,
    pub text: String,
}

/// Parse an asciinema v2 cast into its ordered output events.
pub fn parse_cast(raw: &str) -> Result<Vec<OutputEvent>, String> {
    let mut events = Vec::new();
    let mut saw_header = false;
    for (idx, line) in raw.lines().enumerate() {
        let lineno = idx + 1;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let value: Value =
            serde_json::from_str(line).map_err(|err| format!("line {lineno}: not JSON: {err}"))?;
        if !saw_header {
            let obj = value
                .as_object()
                .ok_or_else(|| format!("line {lineno}: header must be a JSON object"))?;
            let version = obj
                .get("version")
                .and_then(Value::as_i64)
                .ok_or_else(|| format!("line {lineno}: header has no integer 'version'"))?;
            if version != 2 {
                return Err(format!(
                    "line {lineno}: cast version {version} is not 2 (only v2 casts are supported)"
                ));
            }
            saw_header = true;
            continue;
        }
        let arr = value
            .as_array()
            .ok_or_else(|| format!("line {lineno}: event must be an array"))?;
        if arr.len() != 3 {
            return Err(format!(
                "line {lineno}: event must have exactly 3 elements [time, code, data], got {}",
                arr.len()
            ));
        }
        let time = arr[0]
            .as_f64()
            .ok_or_else(|| format!("line {lineno}: event time must be a number"))?;
        let code = arr[1]
            .as_str()
            .ok_or_else(|| format!("line {lineno}: event code must be a string"))?;
        let data = arr[2]
            .as_str()
            .ok_or_else(|| format!("line {lineno}: event data must be a string"))?;
        if code == "o" {
            events.push(OutputEvent {
                time,
                text: data.to_string(),
            });
        }
        // i/r/m and unknown codes: never output text.
    }
    if !saw_header {
        return Err("empty cast: no header line".to_string());
    }
    Ok(events)
}

/// Strip ANSI escape sequences from terminal output text: CSI
/// (`ESC [ … <final 0x40–0x7E>`), OSC (`ESC ] … BEL` or `ESC \`), and
/// two-character escapes (`ESC <char>`). Control chars other than `\n`,
/// `\r`, and `\t` are dropped (they are terminal control, not content).
pub fn strip_ansi(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            push_content(&mut out, c);
            continue;
        }
        match chars.next() {
            // CSI: parameter/intermediate bytes until a final byte 0x40–0x7E.
            Some('[') => {
                for c2 in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&c2) {
                        break;
                    }
                }
            }
            // OSC: terminated by BEL or ST (ESC \).
            Some(']') => {
                let mut prev_esc = false;
                let mut done = false;
                for c2 in chars.by_ref() {
                    if prev_esc && c2 == '\\' {
                        done = true;
                        break;
                    }
                    prev_esc = c2 == '\u{1b}';
                    if c2 == '\u{7}' {
                        done = true;
                        break;
                    }
                }
                let _ = done;
            }
            // Two-character escape (ESC 7, ESC =, ESC M, …): swallowed.
            Some(_) => {}
            None => break,
        }
    }
    out
}

/// Content filter: keep newlines/CR/tabs, drop other C0 controls.
fn push_content(out: &mut String, c: char) {
    if c == '\n' || c == '\r' || c == '\t' || !c.is_control() {
        out.push(c);
    }
}

/// Render a cast to a plain-text transcript: the output events concatenated
/// in order (terminal writes are incremental — concatenation replays the
/// screen text) with ANSI escapes stripped.
pub fn cast_to_text(raw: &str) -> Result<String, String> {
    let events = parse_cast(raw)?;
    let mut text = String::new();
    for event in events {
        text.push_str(&strip_ansi(&event.text));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CAST: &str = concat!(
        r#"{"version": 2, "width": 80, "height": 24, "env": {"SHELL": "/bin/zsh"}}"#,
        "\n",
        r#"[0.1, "o", "$ "]"#,
        "\n",
        r#"[0.2, "i", "ls\r"]"#,
        "\n",
        r#"[0.3, "o", "\u001b[?1hfile-a\u001b[0m\n"]"#,
        "\n",
        r#"[0.4, "o", "file-b\n"]"#,
        "\n",
        r#"[0.5, "r", "100x30"]"#,
        "\n",
        r#"[0.6, "m", "checkpoint"]"#,
        "\n"
    );

    #[test]
    fn parses_header_and_output_events_only() {
        let events = parse_cast(CAST).expect("parses");
        assert_eq!(events.len(), 3);
        assert_eq!(events[0].text, "$ ");
        assert!((events[1].time - 0.3).abs() < 1e-9);
    }

    #[test]
    fn transcript_strips_ansi_and_skips_non_output() {
        let text = cast_to_text(CAST).expect("renders");
        assert_eq!(text, "$ file-a\nfile-b\n");
    }

    #[test]
    fn wrong_version_rejects() {
        let bad = concat!(r#"{"version": 1, "width": 80, "height": 24}"#, "\n");
        assert!(parse_cast(bad).unwrap_err().contains("version 1 is not 2"));
    }

    #[test]
    fn malformed_event_names_line() {
        let bad = concat!(r#"{"version": 2}"#, "\n", r#"{"oops": true}"#, "\n");
        assert!(parse_cast(bad).unwrap_err().contains("line 2"));
    }

    #[test]
    fn empty_input_rejects() {
        assert!(parse_cast("").unwrap_err().contains("no header"));
    }

    #[test]
    fn blank_lines_are_tolerated() {
        let cast = concat!(r#"{"version": 2}"#, "\n\n", r#"[0.0, "o", "hi"]"#, "\n");
        assert_eq!(parse_cast(cast).expect("parses").len(), 1);
    }

    #[test]
    fn strip_ansi_csi_osc_and_two_char_escapes() {
        assert_eq!(strip_ansi("\u{1b}[31mred\u{1b}[0m"), "red");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{7}after"), "after");
        assert_eq!(strip_ansi("\u{1b}]0;title\u{1b}\\x"), "x");
        assert_eq!(strip_ansi("\u{1b}7saved\u{1b}8"), "saved");
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\u{1b}"), "");
    }

    #[test]
    fn strip_ansi_drops_control_but_keeps_formatting_newlines() {
        assert_eq!(strip_ansi("a\u{7}b\nc\td\re"), "ab\nc\td\re");
    }

    #[test]
    fn incremental_writes_concatenate() {
        let cast = concat!(
            r#"{"version": 2}"#,
            "\n",
            r#"[0.01, "o", "smah"]"#,
            "\n",
            r#"[0.02, "o", "-run"]"#,
            "\n"
        );
        assert_eq!(cast_to_text(cast).expect("renders"), "smah-run");
    }
}
