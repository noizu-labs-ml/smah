//! The deferred-execution driver loop (design decision D7).
//!
//! ```text
//! msgs = [user(invocation block)]        // system prompt assembled once
//! for turn in 1..=TURNS_MAX:
//!    text = backend.stream_turn(model, system, msgs, scanner-fed stdout)
//!    protocol violation?  -> audit(denied), Err(DirectiveProtocol)
//!    no directives?       -> Ok                      // answer is complete
//!    execute each directive in emission order      // deferred, in order
//!    violation?           -> audit(denied), Err(DirectiveViolation)
//!    msgs += [assistant(text minus directives), user(results)]
//! Err(DirectiveLoopOverflow)                        // exit 5
//! ```
//!
//! Everything the model must not control — the operating contract, the caps,
//! the roots — is code-owned. The tool author adds a persona and optional
//! constraints; the contract is appended by the driver and cannot be weakened
//! from a tool file.
//!
//! Execution timing is **deferred and order-preserving**: the full turn
//! streams to stdout first (minus intercepted lines), then that turn's
//! directives execute in emission order, then the loop re-queries if any
//! `read` needs results. Uniform for read/write/append, trivially testable,
//! and stdout never interleaves with filesystem effects mid-sentence. The
//! cost — file effects land at turn end — is documented in the driver spec.
//!
//! Backend access goes through the local [`ChatBackend`] trait (not any
//! provider type), so the whole loop is unit-testable with a fake backend
//! and provider drift is a one-adapter fix (`smah-cli/src/backend.rs`).

use crate::errors::SmahError;
use crate::virtual_tools::audit::{Audit, AuditRecord};
use crate::virtual_tools::directives::{Directive, ScanError, Scanner};
use crate::virtual_tools::fs_surface::{FileSurface, Outcome};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::TURNS_MAX;

/// Role of one message in the driver's conversation thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// User turn (the invocation block, or file-surface results).
    User,
    /// Assistant turn (the model's previous answer, directives stripped).
    Assistant,
}

/// One message in the driver's conversation thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatMessage {
    /// Who produced the content.
    pub role: Role,
    /// Message text.
    pub content: String,
}

impl ChatMessage {
    /// A user turn.
    pub fn user(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::User,
            content: content.into(),
        }
    }

    /// An assistant turn.
    pub fn assistant(content: impl Into<String>) -> Self {
        ChatMessage {
            role: Role::Assistant,
            content: content.into(),
        }
    }
}

/// A chat-completion inference backend, streaming. Returns the full answer
/// text once the stream ends; content reached `on_delta` earlier.
pub trait ChatBackend {
    /// Stream one model turn. `on_delta` receives text pieces as they
    /// arrive; the returned `String` is the complete answer.
    fn stream_turn(
        &self,
        model: &str,
        system: &str,
        messages: &[ChatMessage],
        on_delta: &mut dyn FnMut(&str),
    ) -> Result<String, SmahError>;
}

/// Output sinks. Content goes to stdout (flushed per piece — §5 first-byte
/// discipline); notices go to stderr.
pub trait Sink {
    /// Content piece (stdout; raw bytes, no styling).
    fn content(&mut self, piece: &str);
    /// A diagnostic line (stderr).
    fn notice(&mut self, line: &str);
}

/// Real stdout/stderr sink.
pub struct StdSink;

impl Sink for StdSink {
    fn content(&mut self, piece: &str) {
        use std::io::Write;
        let mut out = std::io::stdout();
        let _ = out.write_all(piece.as_bytes());
        let _ = out.flush();
    }

    fn notice(&mut self, line: &str) {
        eprintln!("{line}");
    }
}

/// File-surface mode for the run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FsMode {
    /// The tool declares roots and they resolved; directives execute.
    Enabled(Vec<std::path::PathBuf>),
    /// The tool declares roots but the user passed `--no-fs`: directives
    /// become soft "file surface disabled" results.
    DisabledByFlag,
    /// The tool declares no roots: every directive is a violation (D1 — a
    /// tool with no declared surface asking for files is a design bug).
    NoneDeclared,
}

/// One completed run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunOutcome {
    /// Model turns consumed (1-based).
    pub turns: usize,
    /// Whether the run wrote anything to the file surface.
    pub wrote_files: bool,
    /// The full conversation (invocation + each assistant turn with
    /// directives stripped + result turns) — for the caller's transcript
    /// cache. Directives are stripped from assistant turns; result turns
    /// carry the `[FILE ...]`/`[OK ...]`/`[ERROR ...]` blocks verbatim.
    pub messages: Vec<ChatMessage>,
}

/// Run one virtual tool to completion.
///
/// `args` and `stdin` become the invocation block; `stdin` should already be
/// truncated and annotated by the caller (the driver only renders it).
#[allow(clippy::too_many_arguments)]
pub fn run(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
) -> Result<RunOutcome, SmahError> {
    run_with_turns(
        tool,
        backend,
        sink,
        model,
        args,
        stdin,
        stdin_partial,
        fs_mode,
        audit,
        run_id,
        TURNS_MAX,
    )
}

/// [`run`] with an explicit turn cap (tests exercise the overflow path fast).
#[allow(clippy::too_many_arguments)]
pub fn run_with_turns(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
    max_turns: usize,
) -> Result<RunOutcome, SmahError> {
    let system = system_prompt(tool, fs_mode);
    let mut messages = vec![ChatMessage::user(invocation_block(
        args,
        stdin,
        stdin_partial,
    ))];
    let mut surface = match fs_mode {
        FsMode::Enabled(roots) => Some(FileSurface::new(roots.clone())),
        FsMode::DisabledByFlag | FsMode::NoneDeclared => None,
    };
    let mut wrote_files = false;

    for turn in 1..=max_turns {
        let mut scanner = Scanner::new();
        let result = {
            let scanner_ref = &mut scanner;
            let sink_ref = &mut *sink;
            let mut emit = |piece: &str| {
                scanner_ref.feed(piece, &mut |content| sink_ref.content(content));
            };
            backend.stream_turn(model, &system, &messages, &mut emit)
        };
        match result {
            Ok(_) => {}
            Err(err) => return Err(err),
        }
        // Flush the unterminated trailing line (may itself be a directive).
        scanner.finish(&mut |piece| sink.content(piece));

        if let Some(err) = scanner.error() {
            let detail = scan_error_detail(err);
            audit_denied(
                audit,
                run_id,
                tool,
                "protocol",
                "(unparsable line)",
                None,
                None,
                turn - 1,
                &detail,
            );
            sink.notice(&format!(
                "smah(virtual): {} broke the directive protocol: {detail}",
                tool.name
            ));
            return Err(SmahError::DirectiveProtocol {
                tool: tool.name.clone(),
                detail,
            });
        }

        let directives = scanner.take_directives();
        if directives.is_empty() {
            return Ok(RunOutcome {
                turns: turn,
                wrote_files,
                messages,
            });
        }

        // Deferred, order-preserving execution of this turn's directives.
        let mut results: Vec<String> = Vec::new();
        for directive in &directives {
            audit_attempt(audit, run_id, tool, directive, turn - 1);
            match execute_directive(fs_mode, &mut surface, directive) {
                ExecOutcome::Allowed(line, bytes, resolved, wrote) => {
                    if wrote {
                        wrote_files = true;
                    }
                    let did = match directive {
                        Directive::Read { .. } => "read",
                        Directive::Append { .. } => "appended to",
                        Directive::Write { .. } => "wrote",
                        // Unreachable: extension directives are refused in
                        // execute_directive before anything returns Allowed.
                        Directive::Extension { .. } => "handled",
                    };
                    sink.notice(&format!(
                        "smah(virtual): {} {} {} {} {}",
                        tool.name,
                        did,
                        human_bytes(bytes),
                        if wrote { "→" } else { "from" },
                        resolved
                    ));
                    results.push(line);
                }
                ExecOutcome::SoftError(line) => {
                    sink.notice(&format!(
                        "smah(virtual): {} {} → refused: {}",
                        tool.name,
                        directive.verb(),
                        line
                    ));
                    results.push(format!(
                        "[ERROR {} {}: {line}]",
                        directive.verb(),
                        directive.path()
                    ));
                }
                ExecOutcome::Violation(detail) => {
                    audit_denied(
                        audit,
                        run_id,
                        tool,
                        directive.verb(),
                        directive.path(),
                        None,
                        Some(directive.payload_bytes()),
                        turn - 1,
                        &detail,
                    );
                    sink.notice(&format!(
                        "smah(virtual): {} {} → FORBIDDEN: {detail}",
                        tool.name,
                        directive.verb()
                    ));
                    return Err(SmahError::DirectiveViolation {
                        tool: tool.name.clone(),
                        detail,
                    });
                }
            }
        }

        // A follow-up turn is only needed when the model must react to
        // results: a read it asked for, or a refusal it may retry around.
        // Pure successful writes/updates finish the run here — the answer
        // already streamed (plan W3: "write-only turn, no loop").
        let needs_followup = directives
            .iter()
            .any(|d| matches!(d, Directive::Read { .. }))
            || results.iter().any(|line| line.starts_with("[ERROR"));
        if !needs_followup {
            return Ok(RunOutcome {
                turns: turn,
                wrote_files,
                messages,
            });
        }

        // Re-query with the previous answer (directives stripped) + results.
        messages.push(ChatMessage::assistant(scanner.content().to_string()));
        messages.push(ChatMessage::user(results_block(&results)));
    }

    Err(SmahError::DirectiveLoopOverflow {
        tool: tool.name.clone(),
        turns: max_turns,
    })
}

enum ExecOutcome {
    /// Result line for the model, bytes, resolved path, whether it wrote.
    Allowed(String, usize, String, bool),
    SoftError(String),
    Violation(String),
}

fn execute_directive(
    fs_mode: &FsMode,
    surface: &mut Option<FileSurface>,
    directive: &Directive,
) -> ExecOutcome {
    // Driver-extension verbs are another driver's surface; the cli driver
    // never executes them. (Its scanners only accept the base verbs, so in
    // practice this arm is unreachable from `run` — it guards direct reuse.)
    if let Directive::Extension { verb, .. } = directive {
        return ExecOutcome::Violation(format!(
            "directive verb '{verb}' belongs to another driver; the cli driver cannot execute it"
        ));
    }
    match fs_mode {
        FsMode::NoneDeclared => ExecOutcome::Violation(format!(
            "tool declares no allow_roots, so it has no file surface; '{} {}' is not permitted",
            directive.verb(),
            directive.path()
        )),
        FsMode::DisabledByFlag => {
            ExecOutcome::SoftError("file surface disabled for this run (--no-fs)".to_string())
        }
        FsMode::Enabled(_) => {
            let surface = surface
                .as_mut()
                .expect("surface present when FsMode::Enabled");
            match surface.execute(directive) {
                Outcome::Allowed {
                    resolved,
                    bytes,
                    text,
                } => {
                    let resolved = resolved.display().to_string();
                    match directive {
                        Directive::Read { .. } => ExecOutcome::Allowed(
                            format!("[FILE {resolved}]\n{}\n[/FILE]", text.unwrap_or_default()),
                            bytes,
                            resolved,
                            false,
                        ),
                        Directive::Write { path, .. } | Directive::Append { path, .. } => {
                            ExecOutcome::Allowed(
                                format!("[OK {} {path}: wrote {bytes} bytes]", directive.verb()),
                                bytes,
                                resolved,
                                true,
                            )
                        }
                        // Unreachable: extension directives are refused
                        // before the surface executes anything.
                        Directive::Extension { verb, .. } => ExecOutcome::Violation(format!(
                            "directive verb '{verb}' belongs to another driver"
                        )),
                    }
                }
                Outcome::SoftError { detail } => ExecOutcome::SoftError(detail),
                Outcome::Violation { detail } => ExecOutcome::Violation(detail),
            }
        }
    }
}

/// The code-owned operating contract + persona + constraints. One string;
/// the sentinel reservation sits at the very top.
fn system_prompt(tool: &ToolDef, fs_mode: &FsMode) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "RESERVED TOKEN: the line prefix \"@@smah \" is reserved by the host program. \
Never emit it in normal output except as a complete directive line as described below; \
any line beginning with it is intercepted and never shown to the user.\n\n",
    );
    prompt.push_str(CONTRACT_HEAD);

    match fs_mode {
        FsMode::Enabled(roots) => {
            let roots_list = roots
                .iter()
                .map(|r| r.display().to_string())
                .collect::<Vec<_>>()
                .join(", ");
            prompt.push_str(&format!(
                "FILE SURFACE — this run allows file operations inside these roots only: {roots_list}\n\
Relative paths resolve inside the first root. Absolute paths must be inside one of the roots.\n\
To read or save files, emit a whole directive line (nothing else on the line):\n\
@@smah read {{\"path\": \"<path>\"}}\n\
@@smah write {{\"path\": \"<path>\", \"content\": \"<full file text>\"}}\n\
@@smah append {{\"path\": \"<path>\", \"content\": \"<text to append>\"}}\n\
The host executes the operation and replies with results in the next message. \
Read results come back wrapped as [FILE <path>]...[/FILE]; write/append confirmations as [OK ...]; \
refusals as [ERROR ...] (you may adapt and retry within the turn limit).\n\
Limits per operation: writes up to 128 KiB, reads up to 1 MiB, at most 1 MiB written per run. \
Paths containing '..', anything outside the roots, or a symlinked write target abort the run.\n"
            ));
        }
        FsMode::DisabledByFlag => {
            prompt.push_str(
                "FILE SURFACE — disabled for this run by the user (--no-fs). Do not emit \
@@smah directive lines; if you need a file, describe what is missing in your answer instead.\n",
            );
        }
        FsMode::NoneDeclared => {
            prompt.push_str(
                "FILE SURFACE — none. This tool has no file surface; never emit @@smah \
directive lines. Work only from the input you are given.\n",
            );
        }
    }

    prompt.push_str("\nOUTPUT RULES:\n");
    prompt.push_str(CONTRACT_OUTPUT);

    prompt.push_str("\n# Tool persona\n\n");
    prompt.push_str(&tool.body);
    prompt.push('\n');

    if !tool.constraints.is_empty() {
        prompt.push_str("\nHard constraints:\n");
        for constraint in &tool.constraints {
            prompt.push_str(&format!("- {constraint}\n"));
        }
    }
    prompt
}

/// The driver-owned contract: stdin description + result formats. Cannot be
/// weakened from a tool file (the author may only add rules).
const CONTRACT_HEAD: &str = "\
You are running as a command-line tool inside the smah host program. \
You receive one invocation message describing the tool call: the arguments, \
one per line, and the piped input (stdin) if any, marked INPUT. \
Answer the user's task directly.\n\n";

/// Stdout/content rules + the stdin-truncation acknowledgment duty.
const CONTRACT_OUTPUT: &str = "\
- Your answer text streams to stdout and may be piped into other programs: \
output only the answer content — no banners, no preamble like \"Sure\", no \
commentary about being an AI.\n\
- Never wrap the whole answer in a code fence unless the content itself is code.\n\
- If the invocation says the input was truncated, treat it as partial: work \
with what is shown and never claim to have seen the rest.\n";

/// The invocation user message: tool, args one-per-line, stdin. Shared with
/// the driver family (`pub(crate)`): drivers that keep the invocation-block
/// contract reuse it instead of re-inventing it.
pub(crate) fn invocation_block(
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
) -> String {
    let mut block = String::from("Tool invocation:\n\n");
    if args.is_empty() {
        block.push_str("arguments: (none)\n");
    } else {
        block.push_str("arguments (one per line):\n");
        for arg in args {
            block.push_str(&format!("  {arg}\n"));
        }
    }
    block.push_str("\nINPUT:\n");
    match stdin {
        Some(text) if !text.is_empty() => {
            block.push_str(text);
            if !text.ends_with('\n') {
                block.push('\n');
            }
            if stdin_partial {
                block.push_str(
                    "\n(the input above is TRUNCATED — only part of the user's data is shown)\n",
                );
            }
        }
        _ => block.push_str("(none)\n"),
    }
    block
}

/// File-surface results user message. Shared with the driver family
/// (`pub(crate)`) for drivers that keep the results-block contract.
pub(crate) fn results_block(results: &[String]) -> String {
    let mut block = String::from("File surface results for your directives:\n\n");
    for line in results {
        block.push_str(line);
        block.push('\n');
    }
    block.push_str(
        "\nContinue: emit more directives if needed, or give your final answer \
(remember: stdout is the user's result).",
    );
    block
}

fn scan_error_detail(err: &ScanError) -> String {
    match err {
        ScanError::Protocol { detail } => detail.clone(),
        ScanError::UnknownVerb { verb, .. } => format!("unknown directive verb '{verb}'"),
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_attempt(audit: &Audit, run_id: &str, tool: &ToolDef, directive: &Directive, turn: usize) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: run_id.to_string(),
        tool: tool.name.clone(),
        op: directive.verb().to_string(),
        path: directive.path().to_string(),
        resolved: None,
        bytes: Some(directive.payload_bytes()),
        decision: "allowed",
        reason: String::new(),
        turn,
    };
    if let Err(reason) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

#[allow(clippy::too_many_arguments)]
fn audit_denied(
    audit: &Audit,
    run_id: &str,
    tool: &ToolDef,
    op: &str,
    path: &str,
    resolved: Option<String>,
    bytes: Option<usize>,
    turn: usize,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: run_id.to_string(),
        tool: tool.name.clone(),
        op: op.to_string(),
        path: path.to_string(),
        resolved,
        bytes,
        decision: "denied",
        reason: reason.to_string(),
        turn,
    };
    if let Err(reason) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

fn human_bytes(n: usize) -> String {
    if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{n} B")
    }
}
