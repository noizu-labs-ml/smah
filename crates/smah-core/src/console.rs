//! stdout/stderr console split with render-edge styling.
//!
//! Messages are stored **plain** ([`Message`]: stream + tone + text) and
//! styled only at the write edge: ANSI escape codes are applied iff the
//! target stream is a TTY ([`std::io::IsTerminal`]). A non-TTY stream
//! receives the exact plain bytes — pipes, files, and captures never see
//! escape sequences.

use std::io::{IsTerminal, StderrLock, StdoutLock, Write};

/// Which standard stream a message targets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// Standard output — answer payload only (§5 contract).
    Out,
    /// Standard error — diagnostics, warnings, errors.
    Err,
}

/// Semantic tone of a message; mapped to ANSI styling at the render edge.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    /// No styling.
    Plain,
    /// Emphasized.
    Bold,
    /// De-emphasized (hints, echo of internal state).
    Dim,
    /// Error text (red).
    Error,
    /// Warning text (yellow).
    Warning,
    /// Affirmation text (green).
    Success,
    /// Informational text (cyan).
    Info,
}

impl Tone {
    /// ANSI SGR parameter(s) for this tone; empty for [`Tone::Plain`].
    fn sgr(self) -> &'static str {
        match self {
            Tone::Plain => "",
            Tone::Bold => "1",
            Tone::Dim => "2",
            Tone::Error => "31",
            Tone::Warning => "33",
            Tone::Success => "32",
            Tone::Info => "36",
        }
    }
}

/// A single user-facing message, stored plain and styled at render time.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Message {
    /// Target stream.
    pub stream: Stream,
    /// Semantic tone (styling hint).
    pub tone: Tone,
    /// Plain text — never carries escape codes at rest.
    pub text: String,
}

impl Message {
    /// Plain stdout message.
    pub fn out(text: impl Into<String>) -> Self {
        Message {
            stream: Stream::Out,
            tone: Tone::Plain,
            text: text.into(),
        }
    }

    /// Plain stderr message.
    pub fn err(text: impl Into<String>) -> Self {
        Message {
            stream: Stream::Err,
            tone: Tone::Plain,
            text: text.into(),
        }
    }

    /// Stderr message with a semantic tone (diagnostics channel).
    pub fn err_tone(tone: Tone, text: impl Into<String>) -> Self {
        Message {
            stream: Stream::Err,
            tone,
            text: text.into(),
        }
    }
}

/// Render `msg` for a stream known to be (`tty == true`) or not to be a TTY.
///
/// TTY: the tone's SGR sequence wraps the text and is reset after — plain
/// bytes otherwise. A trailing newline is appended in both cases.
pub fn render(msg: &Message, tty: bool) -> String {
    if tty {
        match msg.tone.sgr() {
            "" => format!("{}\n", msg.text),
            sgr => format!("\x1b[{}m{}\x1b[0m\n", sgr, msg.text),
        }
    } else {
        format!("{}\n", msg.text)
    }
}

/// Console handle pair writing through locked std streams.
#[derive(Debug, Clone, Copy, Default)]
pub struct Console;

impl Console {
    /// Create the console (std streams are process-global).
    pub fn new() -> Self {
        Console
    }

    /// Whether the given stream is currently a TTY.
    pub fn is_tty(stream: Stream) -> bool {
        match stream {
            Stream::Out => std::io::stdout().is_terminal(),
            Stream::Err => std::io::stderr().is_terminal(),
        }
    }

    /// Emit one message: styled on a TTY, plain bytes otherwise.
    pub fn emit(&self, msg: &Message) {
        let tty = Self::is_tty(msg.stream);
        let line = render(msg, tty);
        match msg.stream {
            Stream::Out => {
                let stdout = std::io::stdout();
                let mut lock: StdoutLock<'_> = stdout.lock();
                let _ = lock.write_all(line.as_bytes());
                let _ = lock.flush();
            }
            Stream::Err => {
                let stderr = std::io::stderr();
                let mut lock: StderrLock<'_> = stderr.lock();
                let _ = lock.write_all(line.as_bytes());
                let _ = lock.flush();
            }
        }
    }

    /// Plain line to stdout.
    pub fn out(&self, text: impl AsRef<str>) {
        self.emit(&Message::out(text.as_ref()));
    }

    /// Plain line to stderr.
    pub fn err(&self, text: impl AsRef<str>) {
        self.emit(&Message::err(text.as_ref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tty_render_applies_tone_ansi() {
        let msg = Message::err_tone(Tone::Error, "boom");
        assert_eq!(render(&msg, true), "\x1b[31mboom\x1b[0m\n");
        let warn = Message::err_tone(Tone::Warning, "careful");
        assert_eq!(render(&warn, true), "\x1b[33mcareful\x1b[0m\n");
    }

    #[test]
    fn non_tty_render_is_plain_bytes() {
        let msg = Message::err_tone(Tone::Error, "boom");
        assert_eq!(render(&msg, false), "boom\n");
        assert_eq!(render(&Message::out("hi"), false), "hi\n");
        assert!(!render(&msg, false).contains('\x1b'));
    }

    #[test]
    fn plain_tone_never_styled_even_on_tty() {
        let msg = Message::out("plain");
        assert_eq!(render(&msg, true), "plain\n");
    }
}
