//! Model-response parsing: `<exec>` suggestion tags in, structured
//! [`Suggestion`]s out, prose preserved (Python `response_parser.py`
//! grammar, extended with the owner-approved `<risk>`/`<touches>` children).
//!
//! Grammar (whitespace-tolerant, hand-rolled — no XML dependency):
//!
//! ```text
//! <exec shell="bash">
//!   <title>Purge journal logs</title>
//!   <purpose>…</purpose>
//!   <command>journalctl --vacuum-time=7d</command>
//!   <risk>DELETES FILES — irreversible for the matched set</risk>
//!   <touches>/var/log/journal/**</touches>
//! </exec>
//! ```
//!
//! * `<risk>`/`<touches>` are optional (models may omit them); `title`,
//!   `purpose`, `command` default to honest placeholders when absent.
//! * `<cot …>…</cot>` thought blocks are dropped entirely (Python
//!   `strip-cot` parity).
//! * Unclosed or malformed tags degrade to prose — a malformed suggestion
//!   never aborts the answer.
//! * Everything between managed tags is prose and renders in order.

/// One model-proposed command (never executed — print-only invariant).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    /// Short human title.
    pub title: String,
    /// What it does / expected impact, per the model.
    pub purpose: String,
    /// The bare command text (no fences, no borders).
    pub command: String,
    /// Model-generated risk line (guidance, never a guarantee).
    pub risk: Option<String>,
    /// Model-generated touches line (paths/processes affected).
    pub touches: Option<String>,
    /// Shell the model named for the command (`bash` default).
    pub shell: String,
}

impl Suggestion {
    /// Placeholder defaults per Python `ExecTag` (`Run Command`, `No Purpose
    /// Provided`).
    pub fn placeholders() -> Self {
        Suggestion {
            title: "Run Command".to_string(),
            purpose: "No purpose provided".to_string(),
            command: String::new(),
            risk: None,
            touches: None,
            shell: "bash".to_string(),
        }
    }
}

/// A parsed model response: prose (rendered in order) and suggestions.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedResponse {
    /// Prose with managed tag bodies removed.
    pub prose: String,
    /// Suggestions in arrival order.
    pub suggestions: Vec<Suggestion>,
}

/// Child-tag names parsed inside `<exec>` (lowercase, trimmed).
const EXEC_CHILDREN: [&str; 5] = ["title", "purpose", "command", "risk", "touches"];

/// Tags swallowed whole (never rendered).
const DROPPED_TAGS: [&str; 1] = ["cot"];

/// Parse a full model response.
pub fn parse_response(text: &str) -> ParsedResponse {
    let mut out = ParsedResponse::default();
    let mut prose = String::new();

    let mut rest = text;
    while let Some(tag_start) = rest.find('<') {
        let (before, from_tag) = rest.split_at(tag_start);
        prose.push_str(before);
        match split_managed_tag(from_tag) {
            Some((name, attrs, attrs_end)) => {
                let (body, after) = match tag_body_and_after(from_tag, attrs_end, &name) {
                    Some(parts) => parts,
                    // Unclosed managed tag: degrade the remainder to prose.
                    None => {
                        prose.push_str(from_tag);
                        rest = "";
                        break;
                    }
                };
                if name == "exec" {
                    out.suggestions.push(parse_exec(&attrs, body));
                }
                rest = after;
            }
            None => {
                // Not a tag we manage: the literal '<' stays prose.
                prose.push('<');
                rest = &from_tag[1..];
            }
        }
    }
    prose.push_str(rest);

    out.prose = normalize_prose(&prose);
    out
}

/// A managed tag split: name (lowercase), raw attribute text, and the byte
/// offset just past the opening tag's `>`. `None` = not a managed tag
/// (unknown name, missing `>`, or a closing tag — callers treat the `<` as
/// prose).
fn split_managed_tag(from_tag: &str) -> Option<(String, String, usize)> {
    let after_lt = &from_tag[1..];
    if after_lt.starts_with('/') {
        return None;
    }
    let name_len = after_lt
        .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
        .unwrap_or(after_lt.len());
    let name = after_lt[..name_len].to_ascii_lowercase();
    if name != "exec" && !DROPPED_TAGS.contains(&name.as_str()) {
        return None;
    }
    let open_end = from_tag.find('>')? + 1;
    let attrs = from_tag[1 + name_len..open_end - 1].to_string();
    Some((name, attrs, open_end))
}

/// Body text between the opening tag and its matching `</name>`, plus the
/// remainder after the close. `None` when the tag never closes.
fn tag_body_and_after<'a>(
    from_tag: &'a str,
    attrs_end: usize,
    name: &str,
) -> Option<(&'a str, &'a str)> {
    let close = format!("</{name}>");
    let body_region = &from_tag[attrs_end..];
    let idx = body_region.find(close.as_str())?;
    let body = &body_region[..idx];
    let after = &body_region[idx + close.len()..];
    Some((body, after))
}

/// Parse one `<exec>` body into a [`Suggestion`].
fn parse_exec(attrs: &str, body: &str) -> Suggestion {
    let mut suggestion = Suggestion::placeholders();
    suggestion.shell = shell_attr(attrs).unwrap_or_else(|| "bash".to_string());

    let mut rest = body;
    while let Some(tag_start) = rest.find('<') {
        let (_, from_tag) = rest.split_at(tag_start);
        let after_lt = &from_tag[1..];
        if after_lt.starts_with('/') {
            break;
        }
        let name_len = after_lt
            .find(|c: char| !c.is_ascii_alphanumeric() && c != '-' && c != '_')
            .unwrap_or(after_lt.len());
        let name = after_lt[..name_len].to_ascii_lowercase();
        let open_end = match from_tag.find('>') {
            Some(i) => i + 1,
            None => break,
        };
        if from_tag[..open_end].ends_with("/>") {
            // Self-closing empty child: nothing to capture.
            rest = &from_tag[open_end..];
            continue;
        }
        match tag_body_and_after(from_tag, open_end, &name) {
            Some((text, after)) => {
                if name == "exec" {
                    // Nested exec is malformed; keep scanning from its body.
                    rest = text;
                    continue;
                }
                if EXEC_CHILDREN.contains(&name.as_str()) {
                    let value = unescape(text.trim());
                    match name.as_str() {
                        "title" => suggestion.title = value,
                        "purpose" => suggestion.purpose = value,
                        "command" => suggestion.command = value,
                        "risk" => suggestion.risk = (!value.is_empty()).then_some(value),
                        "touches" => suggestion.touches = (!value.is_empty()).then_some(value),
                        _ => {}
                    }
                }
                rest = after;
            }
            None => break,
        }
    }
    suggestion
}

/// `shell="…"` (or `shell='…'`) attribute value from raw opening-tag text.
fn shell_attr(attrs: &str) -> Option<String> {
    let after_key = attrs.split("shell").nth(1)?;
    let first_quote = after_key.find(['"', '\''])?;
    let quote = after_key.as_bytes()[first_quote] as char;
    let value = &after_key[first_quote + 1..];
    let end = value.find(quote)?;
    Some(unescape(&value[..end]))
}

/// Minimal entity unescape for tag bodies (`&lt; &gt; &amp; &quot; &#39;`).
fn unescape(text: &str) -> String {
    text.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&amp;", "&")
}

/// Trim stray blank edges left by tag removal; keep interior paragraph
/// breaks (runs of 3+ newlines collapse to one paragraph gap — the tag
/// bodies usually contribute the extras).
fn normalize_prose(prose: &str) -> String {
    let joined = prose
        .lines()
        .map(str::trim_end)
        .collect::<Vec<_>>()
        .join("\n");
    let collapsed = collapse_newline_runs(&joined);
    collapsed.trim_matches('\n').to_string()
}

/// Collapse every run of 3+ newlines to exactly 2.
fn collapse_newline_runs(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut run = 0;
    for ch in text.chars() {
        if ch == '\n' {
            run += 1;
            if run <= 2 {
                out.push(ch);
            }
        } else {
            run = 0;
            out.push(ch);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "Here is how to purge old journals:\n\n<exec shell=\"bash\">\n<title>Purge journal logs</title>\n<purpose>Free about 2.1 GB by dropping entries older than 7 days.</purpose>\n<command>journalctl --vacuum-time=7d</command>\n<risk>DELETES FILES — irreversible for the matched set</risk>\n<touches>/var/log/journal/**</touches>\n</exec>\n\nThat is all.";

    #[test]
    fn parses_full_sample() {
        let parsed = parse_response(SAMPLE);
        assert_eq!(
            parsed.prose,
            "Here is how to purge old journals:\n\nThat is all."
        );
        assert_eq!(parsed.suggestions.len(), 1);
        let s = &parsed.suggestions[0];
        assert_eq!(s.title, "Purge journal logs");
        assert_eq!(
            s.purpose,
            "Free about 2.1 GB by dropping entries older than 7 days."
        );
        assert_eq!(s.command, "journalctl --vacuum-time=7d");
        assert_eq!(
            s.risk.as_deref(),
            Some("DELETES FILES — irreversible for the matched set")
        );
        assert_eq!(s.touches.as_deref(), Some("/var/log/journal/**"));
        assert_eq!(s.shell, "bash");
    }

    #[test]
    fn shell_attribute_parsed_single_quotes() {
        let parsed = parse_response("<exec shell='zsh'><command>true</command></exec>");
        assert_eq!(parsed.suggestions[0].shell, "zsh");
    }

    #[test]
    fn multiple_suggestions_keep_order() {
        let text = "<exec><command>one</command></exec>middle<exec><command>two</command></exec>";
        let parsed = parse_response(text);
        assert_eq!(parsed.suggestions.len(), 2);
        assert_eq!(parsed.suggestions[0].command, "one");
        assert_eq!(parsed.suggestions[1].command, "two");
        assert_eq!(parsed.prose, "middle");
    }

    #[test]
    fn cot_dropped_entirely() {
        let text = "a<cot thinking=\"deep\">secret reasoning</cot>b";
        assert_eq!(parse_response(text).prose, "ab");
    }

    #[test]
    fn defaults_when_children_missing() {
        let parsed = parse_response("<exec><command>ls</command></exec>");
        let s = &parsed.suggestions[0];
        assert_eq!(s.title, "Run Command");
        assert_eq!(s.purpose, "No purpose provided");
        assert_eq!(s.risk, None);
        assert_eq!(s.touches, None);
        assert_eq!(s.shell, "bash");
    }

    #[test]
    fn empty_risk_child_stays_none() {
        let parsed = parse_response("<exec><command>ls</command><risk></risk></exec>");
        assert_eq!(parsed.suggestions[0].risk, None);
    }

    #[test]
    fn unclosed_exec_degrades_to_prose() {
        let parsed = parse_response("ok <exec><command>half");
        assert!(parsed.suggestions.is_empty());
        assert!(parsed.prose.contains("<exec>"));
    }

    #[test]
    fn unknown_tags_stay_prose() {
        assert_eq!(parse_response("a < b and c>d").prose, "a < b and c>d");
        // Unknown tags render verbatim (nothing is hidden from the user).
        assert_eq!(
            parse_response("x<smah-foo>bar</smah-foo>y").prose,
            "x<smah-foo>bar</smah-foo>y"
        );
    }

    #[test]
    fn stray_close_tag_is_prose() {
        assert_eq!(parse_response("a</exec>b").prose, "a</exec>b");
    }

    #[test]
    fn entities_unescape_in_bodies() {
        let parsed =
            parse_response("<exec><command>echo &amp;&amp; true &lt;file&gt;</command></exec>");
        assert_eq!(parsed.suggestions[0].command, "echo && true <file>");
    }

    #[test]
    fn multiline_commands_preserved() {
        let cmd = "for f in *.log; do\n  gzip \"$f\"\ndone";
        let parsed = parse_response(&format!("<exec><command>{cmd}</command></exec>"));
        assert_eq!(parsed.suggestions[0].command, cmd);
    }
}
