//! Virtual tools: user-authored markdown tools the CLI driver *emulates*
//! (ticket #17). The model never runs anything — it emits directive lines;
//! the driver executes them against a restricted file surface and streams
//! everything else to stdout.
//!
//! Modules:
//! - [`loader`]: parse/validate/discover `~/.config/smah/virtual-tools/*.md`
//! - [`directives`]: the `@@smah` line-protocol scanner
//! - [`fs_surface`]: allowlist-root containment + directive execution
//! - [`audit`]: append-only JSONL audit log
//! - [`driver`]: the multi-turn loop, [`driver::ChatBackend`] and
//!   [`driver::Sink`] seams, prompt assembly
//!
//! Every size cap lives in this module's [`limits`](self) constants so the
//! driver spec doc can be cross-checked against one place.

pub mod audit;
pub mod directives;
pub mod driver;
pub mod fs_surface;
pub mod loader;

/// A directive line longer than this is protocol abuse, not content.
pub const DIRECTIVE_LINE_MAX: usize = 512 * 1024;

/// Per-directive write/append payload cap. Over-cap writes are refused back
/// to the model as a soft error (it may retry smaller); the file is untouched.
pub const WRITE_CONTENT_MAX: usize = 128 * 1024;

/// Per-read cap; oversize reads are a soft error result.
pub const READ_FILE_MAX: usize = 1024 * 1024;

/// Sum of all bytes written in one run; breach aborts the run (violation).
pub const RUN_AGGREGATE_WRITE_MAX: usize = 1024 * 1024;

/// Model turns per run. A tool that keeps asking for file operations past
/// this is non-convergent → [`crate::errors::SmahError::DirectiveLoopOverflow`].
pub const TURNS_MAX: usize = 8;

/// Default piped-stdin cap in bytes. Over-cap input is *truncated loudly*
/// (§5 rule 7): the head is sent, the model is told its input is partial,
/// and a stderr notice names `--max-input-chars`.
pub const DEFAULT_INPUT_BYTES: usize = 256 * 1024;

/// Tool names and subcommand words a virtual tool may not claim (current +
/// planned M5 surface — prevents `smah hooks`/`smah tools` ambiguity and the
/// `-c/--config` confusion class).
pub const RESERVED_NAMES: [&str; 12] = [
    "ask", "chat", "history", "resume", "config", "models", "doctor", "hooks", "tools", "emulate",
    "help", "version",
];

/// The only directive verbs the executor knows. Anything else in a
/// well-formed directive line is a violation (unknown future verbs belong to
/// later drivers and must not run as file operations).
pub const KNOWN_VERBS: [&str; 3] = ["read", "write", "append"];

/// Tool-name rule, implemented as a validator (no regex dependency):
/// `^[a-z][a-z0-9_-]{0,63}$`.
pub fn is_valid_name(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() => {}
        _ => return false,
    }
    let rest = chars.count();
    rest <= 63
        && name
            .chars()
            .skip(1)
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}
