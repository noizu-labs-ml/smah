//! The `@@smah` directive line protocol (design decision D2).
//!
//! A directive is a single whole line:
//!
//! ```text
//! @@smah write {"path": "...", "content": "..."}
//! @@smah append {"path": "...", "content": "..."}
//! @@smah read {"path": "..."}
//! ```
//!
//! Line protocol, not fenced blocks: single lines are incrementally
//! parseable while streaming, the sentinel is grep-able and near-impossible
//! in prose, and JSON escaping makes arbitrary newlines/quotes unambiguous.
//!
//! The scanner is fed stream deltas; it emits *content* pieces (everything
//! that is not a directive) through a callback as it arrives — at most the
//! first bytes of a line are ever buffered, and only while they could still
//! turn out to be the sentinel — and holds complete directive lines back.
//! The `@@smah ` prefix is **reserved**: any line-initial sentinel is
//! intercepted and never reaches stdout. A sentinel line that does not parse
//! (`[directives::ScanError::Protocol]`) or a well-formed directive the
//! executor refuses (`[directives::ScanError::Violation]`) aborts the run
//! (both exit 3; see [`crate::errors::SmahError::DirectiveProtocol`] and
//! [`crate::errors::SmahError::DirectiveViolation`]).
//!
//! Verb-set seam (PR-0): the base verb set is [`KNOWN_VERBS`]
//! (`read`/`write`/`append`, each carrying a strict file-operation schema).
//! [`Scanner::with_verbs`] lets a driver-family member accept *extension*
//! verbs (e.g. the tui driver's `ui`); those parse as
//! [`Directive::Extension`] with the raw payload carried through for the
//! owning driver to validate and interpret. The base verbs and their
//! schemas never change shape here.

use serde_json::Value;
use std::fmt;

use crate::virtual_tools::{DIRECTIVE_LINE_MAX, KNOWN_VERBS};

/// The reserved line-initial sentinel (with its trailing space in use).
pub const SENTINEL: &str = "@@smah";

/// One parsed directive.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Directive {
    /// `@@smah read {"path": "..."}` — fetch a file's text.
    Read {
        /// Requested path (absolute or relative; resolved by the executor).
        path: String,
    },
    /// `@@smah write {...}` — create/overwrite a file.
    Write {
        /// Requested path.
        path: String,
        /// Full file contents (JSON-unescaped).
        content: String,
    },
    /// `@@smah append {...}` — append to an existing file.
    Append {
        /// Requested path.
        path: String,
        /// Text to append.
        content: String,
    },
    /// A driver-extension directive: `@@smah <verb> <json>` whose verb the
    /// scanner accepts via [`Scanner::with_verbs`] but has no base schema
    /// for. The payload is carried **raw** (unvalidated here) — the owning
    /// driver parses and enforces its own contract (the
    /// [`DIRECTIVE_LINE_MAX`] line cap is the only host-level limit).
    /// Never produced by the cli driver's scanners.
    Extension {
        /// The extension verb (e.g. the tui driver's `ui`).
        verb: String,
        /// Raw post-verb payload text (possibly empty).
        payload: String,
    },
}

impl Directive {
    /// The verb word.
    pub fn verb(&self) -> &str {
        match self {
            Directive::Read { .. } => "read",
            Directive::Write { .. } => "write",
            Directive::Append { .. } => "append",
            Directive::Extension { verb, .. } => verb,
        }
    }

    /// The requested path (empty for [`Directive::Extension`] — extension
    /// directives carry no file path; the owning driver gives meaning to
    /// its own payload).
    pub fn path(&self) -> &str {
        match self {
            Directive::Read { path }
            | Directive::Write { path, .. }
            | Directive::Append { path, .. } => path,
            Directive::Extension { .. } => "",
        }
    }

    /// Payload size in bytes (0 for read; the raw extension payload for
    /// extension directives).
    pub fn payload_bytes(&self) -> usize {
        match self {
            Directive::Read { .. } => 0,
            Directive::Write { content, .. } | Directive::Append { content, .. } => content.len(),
            Directive::Extension { payload, .. } => payload.len(),
        }
    }
}

/// Why a sentinel line could not be honored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanError {
    /// The line does not parse as `@@smah <verb> <json>` with the right
    /// schema, or blew the [`DIRECTIVE_LINE_MAX`] line cap. Protocol abuse —
    /// exit 3 via `DirectiveProtocol`.
    Protocol {
        /// One-line description.
        detail: String,
    },
    /// A well-formed line with a verb this scanner does not accept. Exit 3
    /// via `DirectiveViolation` (D2: unknown verbs are a violation, not a
    /// negotiation).
    UnknownVerb {
        /// The verb that was not recognized.
        verb: String,
        /// The verb set this scanner was configured with (the base set for
        /// [`Scanner::new`], the extended set for [`Scanner::with_verbs`] —
        /// so extended scanners report their own accepted verbs).
        known: &'static [&'static str],
    },
}

impl fmt::Display for ScanError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScanError::Protocol { detail } => write!(f, "{detail}"),
            ScanError::UnknownVerb { verb, known } => {
                write!(
                    f,
                    "unknown directive verb '{verb}'; known verbs: {}",
                    known.join(", ")
                )
            }
        }
    }
}

/// Incremental line scanner. Feed stream deltas; content pieces come out
/// through the callback the moment they are unambiguous.
///
/// Holding discipline: a line is buffered only while the bytes so far are a
/// prefix of the sentinel (≤ 7 bytes) or while the line *is* a sentinel
/// directive (bounded by [`DIRECTIVE_LINE_MAX`]). Everything else streams
/// through immediately, flushed per delta (§5 first-byte discipline).
pub struct Scanner {
    /// Current (unterminated) line under consideration.
    line: String,
    /// True while `line` might still become a directive (starts with the
    /// sentinel or a prefix of it).
    deciding: bool,
    /// True once the directive line exceeded [`DIRECTIVE_LINE_MAX`] (the
    /// rest of the line is dropped until the newline).
    overflowing: bool,
    /// Complete content emitted so far (everything passed to the callback).
    content: String,
    /// Complete directives, in emission order.
    directives: Vec<Directive>,
    /// First protocol/verb error seen, if any.
    error: Option<ScanError>,
    /// The accepted verb set (base verbs via [`Scanner::new`]; extended by
    /// [`Scanner::with_verbs`]).
    verbs: &'static [&'static str],
}

impl Scanner {
    /// A fresh scanner with the base verb set ([`KNOWN_VERBS`]).
    pub fn new() -> Self {
        Scanner::with_verbs(&KNOWN_VERBS)
    }

    /// A fresh scanner accepting an extended verb set (the driver-family
    /// seam): base verbs keep their strict schemas; any other accepted verb
    /// parses as [`Directive::Extension`] with its raw payload. Unknown
    /// verbs (outside `verbs`) stay a [`ScanError::UnknownVerb`] violation
    /// whose message reports *this* set.
    pub fn with_verbs(verbs: &'static [&'static str]) -> Self {
        Scanner {
            line: String::new(),
            deciding: true,
            overflowing: false,
            content: String::new(),
            directives: Vec::new(),
            error: None,
            verbs,
        }
    }
}

impl Default for Scanner {
    fn default() -> Self {
        Scanner::new()
    }
}

impl Scanner {
    /// Feed one stream delta. `emit` receives content pieces (possibly
    /// empty-per-call splits; concatenation of all pieces is the full
    /// content with directive lines removed).
    pub fn feed(&mut self, delta: &str, emit: &mut dyn FnMut(&str)) {
        for piece in delta.split_inclusive('\n') {
            self.consume(piece, emit);
        }
    }

    /// Flush any unterminated trailing line at end of stream. A trailing
    /// line that actually starts with the sentinel is treated as a directive
    /// attempt (D2 test matrix); a short prefix like `@` or `@@s` is just
    /// content (an at-sign at end of stream is ordinary text).
    pub fn finish(&mut self, emit: &mut dyn FnMut(&str)) {
        if self.line.is_empty() {
            return;
        }
        let line = std::mem::take(&mut self.line);
        let deciding = self.deciding;
        self.deciding = true;
        self.overflowing = false;
        if deciding && line.starts_with(SENTINEL) {
            self.handle_directive_line(&line);
        } else {
            self.emit_content(&line, emit);
        }
    }

    /// All parsed directives, in emission order (drains the scanner).
    pub fn take_directives(&mut self) -> Vec<Directive> {
        std::mem::take(&mut self.directives)
    }

    /// The full content emitted so far (directive lines removed).
    pub fn content(&self) -> &str {
        &self.content
    }

    /// First protocol/verb error, if any.
    pub fn error(&self) -> Option<&ScanError> {
        self.error.as_ref()
    }

    fn consume(&mut self, piece: &str, emit: &mut dyn FnMut(&str)) {
        let (has_newline, line_part) = match piece.strip_suffix('\n') {
            Some(stripped) => (true, stripped),
            None => (false, piece),
        };
        if self.overflowing {
            // Directive line already condemned; swallow until the newline.
            if has_newline {
                self.line.clear();
                self.deciding = true;
                self.overflowing = false;
            }
            return;
        }
        self.line.push_str(line_part);
        if !self.deciding {
            // Mid-content line: stream through as it arrives.
            let chunk = std::mem::take(&mut self.line);
            self.emit_content(&chunk, emit);
        } else if !Scanner::may_become_directive(&self.line) {
            // Mismatch past the sentinel: the whole buffered head is content.
            self.deciding = false;
            let chunk = std::mem::take(&mut self.line);
            self.emit_content(&chunk, emit);
        }
        if has_newline {
            let line = std::mem::take(&mut self.line);
            let deciding = self.deciding;
            self.deciding = true;
            // CRLF: strip the trailing CR before the sentinel check/parse.
            let trimmed = line.strip_suffix('\r').unwrap_or(&line);
            if deciding && trimmed.starts_with(SENTINEL) {
                self.handle_directive_line(trimmed);
            } else {
                // Content line already emitted except its trailing newline;
                // emit it (minus a CR that belonged to the directive check —
                // content lines keep their bytes verbatim). While still
                // `deciding` the buffered head never streamed — flush it
                // first (a blank line, or a lone sentinel prefix like `@`,
                // is ordinary prose, not a directive: the same guard
                // `finish()` applies). Without it every blank line in a
                // streamed answer aborted post-stream with a protocol
                // error, and paragraph breaks were silently dropped.
                if deciding {
                    self.emit_content(&line, emit);
                }
                self.emit_content("\n", emit);
            }
        }
    }

    /// Could `line` still be (or already be) a sentinel directive?
    fn may_become_directive(line: &str) -> bool {
        let sentinel = SENTINEL;
        if line.len() < sentinel.len() {
            return sentinel.starts_with(line);
        }
        if !line.starts_with(sentinel) {
            return false;
        }
        let rest = &line[sentinel.len()..];
        // "@@smah" alone, or "@@smah <verb> ..." — both are directive lines.
        rest.is_empty() || rest.starts_with(' ') || rest.starts_with('\t')
    }

    fn handle_directive_line(&mut self, line: &str) {
        if line.len() > DIRECTIVE_LINE_MAX {
            self.record(ScanError::Protocol {
                detail: format!(
                    "directive line exceeds the {} KiB line cap",
                    DIRECTIVE_LINE_MAX / 1024
                ),
            });
            return;
        }
        match parse_line(line, self.verbs) {
            Ok(Some(directive)) => self.directives.push(directive),
            Ok(None) => self.record(ScanError::Protocol {
                detail: "sentinel line carries no directive".to_string(),
            }),
            Err(err) => self.record(err),
        }
    }

    fn record(&mut self, err: ScanError) {
        if self.error.is_none() {
            self.error = Some(err);
        }
    }

    fn emit_content(&mut self, piece: &str, emit: &mut dyn FnMut(&str)) {
        self.content.push_str(piece);
        emit(piece);
    }
}

/// Parse one complete sentinel line (no trailing newline) against `verbs`.
/// `Ok(None)` means the line was just the bare sentinel; the caller records
/// the protocol error. Base verbs (read/write/append) keep their strict
/// schemas; any other verb accepted by `verbs` becomes a raw
/// [`Directive::Extension`].
fn parse_line(line: &str, verbs: &'static [&'static str]) -> Result<Option<Directive>, ScanError> {
    let rest = match line.strip_prefix(SENTINEL) {
        Some(rest) => rest,
        None => return Ok(None), // caller guarantees sentinel-prefixed
    };
    let rest = rest.trim_start();
    let (verb, payload) = match rest.find(char::is_whitespace) {
        Some(idx) => (&rest[..idx], rest[idx..].trim_start()),
        None => (rest, ""),
    };
    if verb.is_empty() {
        return Ok(None);
    }
    if !verbs.contains(&verb) {
        return Err(ScanError::UnknownVerb {
            verb: verb.to_string(),
            known: verbs,
        });
    }
    match verb {
        "read" | "write" | "append" => parse_file_directive(verb, payload).map(Some),
        other => Ok(Some(Directive::Extension {
            verb: other.to_string(),
            payload: payload.to_string(),
        })),
    }
}

/// Parse the three base file-operation verbs (strict JSON-object schema —
/// the contract the cli driver has always enforced).
fn parse_file_directive(verb: &str, payload: &str) -> Result<Directive, ScanError> {
    let payload: Value = serde_json::from_str(payload).map_err(|err| ScanError::Protocol {
        detail: format!(
            "directive JSON did not parse: {}",
            one_line(&err.to_string())
        ),
    })?;
    let Value::Object(map) = payload else {
        return Err(ScanError::Protocol {
            detail: "directive payload must be a JSON object".to_string(),
        });
    };
    let take_str = |key: &str| -> Result<String, ScanError> {
        match map.get(key) {
            Some(Value::String(s)) => Ok(s.clone()),
            _ => Err(ScanError::Protocol {
                detail: format!("directive payload field '{key}' must be a string"),
            }),
        }
    };
    let path = take_str("path")?;
    match verb {
        "read" => Ok(Directive::Read { path }),
        "write" => {
            let content = take_str("content")?;
            Ok(Directive::Write { path, content })
        }
        "append" => {
            let content = take_str("content")?;
            Ok(Directive::Append { path, content })
        }
        _ => unreachable!("base-verb set checked by parse_line"),
    }
}

/// Collapse whitespace to one line.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
