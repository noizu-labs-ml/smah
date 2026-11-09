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
//!
//! Size caps (M2): a cast is untrusted input — a single line may not exceed
//! [`LINE_MAX_BYTES`] and the whole input may not exceed [`TOTAL_MAX_BYTES`];
//! exceeding either is a hard parse error naming the cap (a hostile cast
//! must not be able to make the parser allocate without bound).

use serde_json::Value;

/// Per-line cap for `.cast` parsing (10 MiB — far above any honest event
/// line; a header or event line beyond it is hostile input).
pub const LINE_MAX_BYTES: usize = 10 * 1024 * 1024;

/// Total-input cap for `.cast` parsing (64 MiB).
pub const TOTAL_MAX_BYTES: usize = 64 * 1024 * 1024;

/// One parsed output event (time in seconds from the recording start).
#[derive(Debug, Clone, PartialEq)]
pub struct OutputEvent {
    pub time: f64,
    pub text: String,
}

/// Parse an asciinema v2 cast into its ordered output events.
pub fn parse_cast(raw: &str) -> Result<Vec<OutputEvent>, String> {
    parse_cast_with_caps(raw, LINE_MAX_BYTES, TOTAL_MAX_BYTES)
}

/// [`parse_cast`] with explicit caps (the test seam — production always
/// passes [`LINE_MAX_BYTES`] / [`TOTAL_MAX_BYTES`]).
fn parse_cast_with_caps(
    raw: &str,
    line_max: usize,
    total_max: usize,
) -> Result<Vec<OutputEvent>, String> {
    if raw.len() > total_max {
        return Err(format!(
            "cast exceeds the total-size cap ({} > {total_max} bytes) — refusing hostile input",
            raw.len()
        ));
    }
    let mut events = Vec::new();
    let mut saw_header = false;
    for (idx, line) in raw.lines().enumerate() {
        let lineno = idx + 1;
        let line = line.trim();
        if line.len() > line_max {
            return Err(format!(
                "line {lineno}: exceeds the per-line cap ({} > {line_max} bytes) — refusing hostile input",
                line.len()
            ));
        }
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
/// (`ESC [ … <final 0x40–0x7E>`), OSC (`ESC ] … BEL` or `ESC \`) —
/// consumed **fully** — plus the string-introducer sequences DCS (`ESC P`),
/// APC (`ESC _`), PM (`ESC ^`), and SOS (`ESC X`), each consumed through
/// its terminator (`ESC \` or BEL). Two-character escapes (`ESC <char>`)
/// are swallowed. The output contains no ESC and no control bytes other
/// than `\n` and `\t` (H2: a malicious cast must not be able to smuggle
/// terminal control sequences — including inside DCS/APC/PM/SOS strings —
/// into the transcript).
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
            // String sequences — OSC, DCS, APC, PM, SOS: everything up to
            // the terminator (ESC \, or BEL for OSC-style leniency) is
            // control data, never content. An ESC inside the string only
            // terminates when followed by '\' (so ESC ESC \ still ends the
            // sequence, and a stray ESC keeps scanning).
            Some(']') | Some('P') | Some('_') | Some('^') | Some('X') => {
                let mut prev_esc = false;
                for c2 in chars.by_ref() {
                    if prev_esc && c2 == '\\' {
                        break;
                    }
                    prev_esc = c2 == '\u{1b}';
                    if c2 == '\u{7}' {
                        break;
                    }
                }
            }
            // Two-character escape (ESC 7, ESC =, ESC M, …): swallowed.
            Some(_) => {}
            None => break,
        }
    }
    out
}

/// Content filter: keep newlines and tabs; drop every other control byte
/// (C0, DEL, and the C1 range 0x80–0x9F — `char::is_control()` covers
/// U+0000–U+001F and U+007F–U+009F). `\r` is terminal mechanics, not
/// transcript content. The result never contains ESC.
fn push_content(out: &mut String, c: char) {
    if c == '\n' || c == '\t' || !c.is_control() {
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
    fn oversized_line_and_total_reject_with_clear_errors() {
        // M2: per-line cap — one giant event line refuses (named line).
        // Small caps through the test seam; production consts below.
        let cast = concat!(
            r#"{"version": 2}"#,
            "\n",
            r#"[0.0, "o", "aaaaaaaaaaaaaaaaaaaa"]"#,
            "\n"
        );
        let err = parse_cast_with_caps(cast, 16, 1024).unwrap_err();
        assert!(err.contains("line 2"), "{err}");
        assert!(err.contains("per-line cap"), "{err}");

        // Total cap — lines that are individually fine, total over the cap.
        let event = "[0.0, \"o\", \"hello\"]".repeat(20);
        let big = format!("{{\"version\": 2}}\n{event}\n");
        let err = parse_cast_with_caps(&big, 64, 100).unwrap_err();
        assert!(err.contains("total-size cap"), "{err}");

        // Boundary: exactly at the caps parses.
        assert!(parse_cast_with_caps(cast, 40, 512).is_ok());
        // The production caps are sane (checked at compile time).
        const { assert!(LINE_MAX_BYTES < TOTAL_MAX_BYTES) };
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
    fn strip_ansi_swallows_dcs_apc_pm_sos_strings() {
        // H2: the four string-introducer sequences consumed through ST/BEL —
        // their payloads never reach the transcript.
        let cases = [
            ('\u{50}', "DCS"), // ESC P
            ('\u{5f}', "APC"), // ESC _
            ('\u{5e}', "PM"),  // ESC ^
            ('\u{58}', "SOS"), // ESC X
        ];
        for (intro, label) in cases {
            let st = strip_ansi(&format!("\u{1b}{intro}hidden payload\u{1b}\\visible"));
            assert_eq!(st, "visible", "{label} through ST");
            let bel = strip_ansi(&format!("\u{1b}{intro}hidden\u{7}visible"));
            assert_eq!(bel, "visible", "{label} through BEL");
            let hostile = strip_ansi(&format!(
                "\u{1b}{intro}malicious \u{1b}[31m CSI inside\u{1b}\\plain"
            ));
            assert_eq!(hostile, "plain", "{label} hiding a CSI");
            // Unterminated: everything to end-of-input is consumed.
            assert_eq!(
                strip_ansi(&format!("start\u{1b}{intro}never-terminated")),
                "start",
                "{label} unterminated"
            );
        }
        // ESC ESC \ still terminates (the first ESC is data inside ST scan).
        assert_eq!(strip_ansi("\u{1b}_a\u{1b}\u{1b}\\b"), "b");
    }

    #[test]
    fn strip_ansi_drops_c1_and_keeps_only_newline_tab() {
        // C1 bytes 0x80–0x9F (as chars U+0080–U+009F) are control — dropped.
        assert_eq!(strip_ansi("a\u{9b}b\u{9c}c"), "abc");
        // ESC never survives (ESC 7 is a two-char escape — swallowed); \r
        // is dropped too (exceptions are \n and \t).
        assert_eq!(strip_ansi("a\u{1b}7\nc\td\re"), "a\nc\tde");
        assert_eq!(strip_ansi("a\u{7f}b"), "ab");
        assert_eq!(strip_ansi("a\u{84}b"), "ab");
    }

    #[test]
    fn strip_ansi_fuzz_invariant_no_control_bytes_out() {
        // Fuzz-style: adversarial sequences built from every introducer,
        // payload, terminator, and truncation point. The INVARIANT: the
        // output contains no ESC and no control bytes other than \n and \t.
        let pieces: Vec<String> = [
            "\u{1b}[",
            "\u{1b}]",
            "\u{1b}P",
            "\u{1b}_",
            "\u{1b}^",
            "\u{1b}X",
            "\u{1b}7",
            "\u{1b}",
            "\u{7}",
            "\u{1b}\\",
            "data",
            "\n",
            "\t",
            "\r",
            "\u{9b}",
            "\u{1b}[31;42m",
            "0;title",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        // Deterministic pseudo-random sampling (LCG) over 3-piece
        // concatenations, all 2-piece pairs, and every single-prefix
        // truncation.
        let mut state: u64 = 0x2545F4914F6CDD1D;
        let mut next = move || {
            state = state
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            (state >> 33) as usize
        };
        let mut samples: Vec<String> = Vec::new();
        for a in &pieces {
            for b in &pieces {
                samples.push(format!("{a}{b}"));
            }
        }
        for _ in 0..400 {
            let a = &pieces[next() % pieces.len()];
            let b = &pieces[next() % pieces.len()];
            let c = &pieces[next() % pieces.len()];
            samples.push(format!("{a}{b}{c}"));
        }
        for s in &pieces {
            for cut in 1..s.chars().count() {
                let truncated: String = s.chars().take(cut).collect();
                samples.push(truncated);
            }
        }
        for input in &samples {
            let out = strip_ansi(input);
            for ch in out.chars() {
                assert!(
                    ch == '\n' || ch == '\t' || !ch.is_control(),
                    "control byte {ch:?} leaked from {input:?} -> {out:?}"
                );
            }
        }
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
