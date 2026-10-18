//! The **tui** virtual-tool driver (#18): the cli loop presented as an
//! interactive terminal UI.
//!
//! Contract parity: the loop, scanner usage, file-surface execution, audit
//! records, and result-line shapes mirror [`super::super::driver`]'s cli
//! driver byte-for-byte wherever stdout is observable (the non-TTY fallback
//! is exactly the cli presentation, plus `ui` op summaries on stderr). The
//! TTY path renders into [`smah_tui::vtool`] (leaf presentation crate) and
//! additionally accepts the driver extension verb
//!
//! ```text
//! @@smah ui {"op": "status"|"pane"|"menu", ...}
//! ```
//!
//! whose payload is validated (never executed — it is display data) and
//! rendered; invalid payloads become soft `[ERROR ui ...]` results.
//!
//! TTY gating: the ratatui presentation runs only when **both** stdin and
//! stdout are terminals; otherwise the run degrades to the line fallback so
//! pipes, CI, and tests stay byte-compatible with the cli driver.

use std::io::IsTerminal;
use std::time::Duration;

use smah_tui::vtool::{parse_ui_op, Flow, TtySession};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{Audit, AuditRecord};
use crate::virtual_tools::directives::{Directive, ScanError, Scanner};
use crate::virtual_tools::driver::{
    invocation_block, results_block, ChatBackend, ChatMessage, FsMode, RunOutcome, Sink,
};
use crate::virtual_tools::fs_surface::{FileSurface, Outcome as FsOutcome};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::{TURNS_MAX, WRITE_CONTENT_MAX};

/// Scanner verb set: the cli base verbs + the tui-only `ui` op.
const TUI_VERBS: &[&str] = &["read", "write", "append", "ui"];

/// `interface:` seed cap (the seed is prompt text, not executable).
const INTERFACE_SEED_MAX: usize = 8 * 1024;

/// Run one tui virtual tool to completion.
///
/// Signature matches the frozen `--emulate` dispatch in smah-cli's
/// `emulate.rs` exactly — drop-in for the stub it replaces.
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
    // Validate the optional UI layout seed before any terminal setup so a
    // malformed tool file fails the same way the cli driver fails.
    let interface = ui_seed(tool)?;

    let tty = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    if tty {
        match TtySession::new(&tool.name, model) {
            Ok(session) => {
                let mut presenter = TtyPresenter {
                    session,
                    quit_seen: false,
                };
                drive(
                    tool,
                    backend,
                    model,
                    args,
                    stdin,
                    stdin_partial,
                    fs_mode,
                    audit,
                    run_id,
                    max_turns,
                    &interface,
                    &mut presenter,
                )
            }
            // Terminal could not be configured: degrade, never fail.
            Err(err) => {
                eprintln!("smah(virtual): tui unavailable ({err}); falling back to line output");
                let mut presenter = LinePresenter { sink };
                drive(
                    tool,
                    backend,
                    model,
                    args,
                    stdin,
                    stdin_partial,
                    fs_mode,
                    audit,
                    run_id,
                    max_turns,
                    &interface,
                    &mut presenter,
                )
            }
        }
    } else {
        let mut presenter = LinePresenter { sink };
        drive(
            tool,
            backend,
            model,
            args,
            stdin,
            stdin_partial,
            fs_mode,
            audit,
            run_id,
            max_turns,
            &interface,
            &mut presenter,
        )
    }
}

/// The presentation seam between the driving loop and the terminal.
trait Present {
    /// Streamed answer piece.
    fn content(&mut self, piece: &str);
    /// A diagnostic line (directive results, refusals).
    fn notice(&mut self, line: &str);
    /// Render (TTY) or summarize (line) one `ui` op payload. `Err` carries a
    /// one-line reason that becomes a soft `[ERROR ui ...]` result.
    fn ui(&mut self, payload: &str) -> Result<(), String>;
    /// Update the visible turn counter.
    fn set_turn(&mut self, turn: usize);
    /// Did the user ask to quit?
    fn quit(&mut self) -> bool;
    /// Keep the final screen up until the user quits (TTY only; no-op on the
    /// line path).
    fn park(&mut self);
}

/// The ratatui presentation: wraps the leaf-crate [`TtySession`].
struct TtyPresenter {
    session: TtySession,
    quit_seen: bool,
}

impl TtyPresenter {
    fn pump(&mut self) {
        if self.session.pump(Duration::from_millis(16)) == Flow::Quit {
            self.quit_seen = true;
        }
    }
}

impl Present for TtyPresenter {
    fn content(&mut self, piece: &str) {
        self.session.app().content(piece);
        self.pump();
    }

    fn notice(&mut self, line: &str) {
        self.session.app().notice(line);
        self.pump();
    }

    fn ui(&mut self, payload: &str) -> Result<(), String> {
        let result = self.session.app().ui_op(payload);
        self.session.force_redraw();
        result
    }

    fn set_turn(&mut self, turn: usize) {
        self.session.app().set_turn(turn);
    }

    fn quit(&mut self) -> bool {
        self.pump();
        self.quit_seen
    }

    fn park(&mut self) {
        // Keep the done screen readable until the user quits (10 ms poll is
        // invisible; pump redraws on its own throttle).
        loop {
            if self.session.pump(Duration::from_millis(80)) == Flow::Quit {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }
}

/// The line fallback: exactly the cli presentation on the passed-in sink
/// (byte-for-byte for base verbs); `ui` ops are validated but only
/// summarized on stderr — the text never echoes to a non-interactive
/// channel (T-017).
struct LinePresenter<'a> {
    sink: &'a mut dyn Sink,
}

impl Present for LinePresenter<'_> {
    fn content(&mut self, piece: &str) {
        self.sink.content(piece);
    }

    fn notice(&mut self, line: &str) {
        self.sink.notice(line);
    }

    fn ui(&mut self, payload: &str) -> Result<(), String> {
        let op = parse_ui_op(payload)?;
        let kind = match op {
            smah_tui::vtool::widgets::UiOp::Status { .. } => "status",
            smah_tui::vtool::widgets::UiOp::Pane { .. } => "pane",
            smah_tui::vtool::widgets::UiOp::Menu { .. } => "menu",
        };
        self.sink.notice(&format!(
            "[ui op {kind} ({} B) — no interactive terminal attached]",
            payload.len()
        ));
        Ok(())
    }

    fn set_turn(&mut self, _turn: usize) {}

    fn quit(&mut self) -> bool {
        false
    }

    fn park(&mut self) {}
}

/// The driving loop: cli-parity turn machine with the `ui` extension and the
/// presenter seam. Structure mirrors `driver::run_with_turns`.
#[allow(clippy::too_many_arguments)]
fn drive(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    model: &str,
    args: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    fs_mode: &FsMode,
    audit: &Audit,
    run_id: &str,
    max_turns: usize,
    interface: &Option<String>,
    presenter: &mut dyn Present,
) -> Result<RunOutcome, SmahError> {
    let system = system_prompt(tool, fs_mode, interface);
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
    presenter.set_turn(0);

    for turn in 1..=max_turns {
        // The user may quit between turns; the finished turns stand.
        if presenter.quit() {
            presenter.set_turn(turn - 1);
            return Ok(RunOutcome {
                turns: turn - 1,
                wrote_files,
                messages,
            });
        }
        presenter.set_turn(turn);

        let mut scanner = Scanner::with_verbs(TUI_VERBS);
        let result = {
            let scanner_ref = &mut scanner;
            let presenter_ref = &mut *presenter;
            let mut emit = |piece: &str| {
                scanner_ref.feed(piece, &mut |content| presenter_ref.content(content));
            };
            backend.stream_turn(model, &system, &messages, &mut emit)
        };
        match result {
            Ok(_) => {}
            Err(err) => return Err(err),
        }
        // Flush the unterminated trailing line (may itself be a directive).
        scanner.finish(&mut |piece| presenter.content(piece));

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
            presenter.notice(&format!(
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
            presenter.park();
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
            match directive {
                // The tui extension verb: display data, validated then
                // rendered. Bad payloads are soft errors, never violations.
                Directive::Extension { verb, payload } if verb == "ui" => {
                    if payload.len() > WRITE_CONTENT_MAX {
                        let line =
                            format!("[ERROR ui: payload exceeds {} bytes]", WRITE_CONTENT_MAX);
                        presenter.notice(&format!(
                            "smah(virtual): {} ui → refused: payload exceeds {} bytes",
                            tool.name, WRITE_CONTENT_MAX
                        ));
                        results.push(line);
                    } else {
                        match presenter.ui(payload) {
                            Ok(()) => {
                                presenter.notice(&format!(
                                    "smah(virtual): {} ui op applied ({} bytes)",
                                    tool.name,
                                    payload.len()
                                ));
                                results.push("[OK ui: rendered]".to_string());
                            }
                            Err(reason) => {
                                presenter.notice(&format!(
                                    "smah(virtual): {} ui → refused: {reason}",
                                    tool.name
                                ));
                                results.push(format!("[ERROR ui: {reason}]"));
                            }
                        }
                    }
                }
                // Extension verbs outside TUI_VERBS are impossible from this
                // scanner; guard direct reuse exactly like the cli driver.
                Directive::Extension { verb, .. } => {
                    let detail = format!(
                        "directive verb '{verb}' belongs to another driver; the tui driver cannot execute it"
                    );
                    audit_denied(audit, run_id, tool, verb, "", None, None, turn - 1, &detail);
                    presenter.notice(&format!(
                        "smah(virtual): {} {} → FORBIDDEN: {detail}",
                        tool.name, verb
                    ));
                    return Err(SmahError::DirectiveViolation {
                        tool: tool.name.clone(),
                        detail,
                    });
                }
                _ => match execute_directive(fs_mode, &mut surface, directive) {
                    ExecOutcome::Allowed(line, bytes, resolved, wrote) => {
                        if wrote {
                            wrote_files = true;
                        }
                        let did = match directive {
                            Directive::Read { .. } => "read",
                            Directive::Append { .. } => "appended to",
                            Directive::Write { .. } => "wrote",
                            Directive::Extension { .. } => unreachable!("handled above"),
                        };
                        presenter.notice(&format!(
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
                        presenter.notice(&format!(
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
                        presenter.notice(&format!(
                            "smah(virtual): {} {} → FORBIDDEN: {detail}",
                            tool.name,
                            directive.verb()
                        ));
                        return Err(SmahError::DirectiveViolation {
                            tool: tool.name.clone(),
                            detail,
                        });
                    }
                },
            }
        }

        // Cli-parity follow-up rule: the model must react to reads and to
        // any refusal; successful writes and ui acks finish the run.
        let needs_followup = directives
            .iter()
            .any(|d| matches!(d, Directive::Read { .. }))
            || results.iter().any(|line| line.starts_with("[ERROR"));
        if !needs_followup {
            presenter.park();
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

/// File-surface execution for the cli base verbs — mirrors
/// `driver::execute_directive` (shared public surface, local copy because
/// the original is private).
fn execute_directive(
    fs_mode: &FsMode,
    surface: &mut Option<FileSurface>,
    directive: &Directive,
) -> ExecOutcome {
    if let Directive::Extension { verb, .. } = directive {
        return ExecOutcome::Violation(format!(
            "directive verb '{verb}' belongs to another driver; the tui driver cannot execute it"
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
                FsOutcome::Allowed {
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
                        Directive::Extension { verb, .. } => ExecOutcome::Violation(format!(
                            "directive verb '{verb}' belongs to another driver"
                        )),
                    }
                }
                FsOutcome::SoftError { detail } => ExecOutcome::SoftError(detail),
                FsOutcome::Violation { detail } => ExecOutcome::Violation(detail),
            }
        }
    }
}

/// Validate the optional `interface:` UI layout seed. Present-but-empty is a
/// tool-file error (silent ignoring would hide author intent); anything
/// larger than [`INTERFACE_SEED_MAX`] is too. The seed is prompt text — it
/// is never parsed, executed, or rendered as markup.
fn ui_seed(tool: &ToolDef) -> Result<Option<String>, SmahError> {
    match &tool.interface {
        None => Ok(None),
        Some(seed) if seed.trim().is_empty() => Err(SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: "interface: is present but empty (a UI layout seed is expected)".to_string(),
        }),
        Some(seed) if seed.len() > INTERFACE_SEED_MAX => Err(SmahError::ToolFileInvalid {
            path: tool.path.display().to_string(),
            detail: format!(
                "interface: UI layout seed exceeds {} bytes",
                INTERFACE_SEED_MAX
            ),
        }),
        Some(seed) => Ok(Some(seed.clone())),
    }
}

/// The code-owned tui operating contract: persona + cli file-surface rules +
/// the tui-specific UI surface.
fn system_prompt(tool: &ToolDef, fs_mode: &FsMode, interface: &Option<String>) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "RESERVED TOKEN: the line prefix \"@@smah \" is reserved by the host program. \
Never emit it in normal output except as a complete directive line as described below; \
any line beginning with it is intercepted and never shown to the user.\n\n",
    );
    prompt.push_str(
        "You are running as an interactive terminal-UI tool inside the smah host program. \
You receive one invocation message describing the tool call: the arguments, \
one per line, and the piped input (stdin) if any, marked INPUT. \
Answer the user's task directly.\n\n",
    );

    prompt.push_str(
        "UI SURFACE — the user watches your answer stream into an answer pane, with a \
directive-results pane and a status line beneath it. You may drive small display \
surfaces by emitting, each on its own line:\n\
@@smah ui {\"op\": \"status\", \"text\": \"<status text>\"}\n\
@@smah ui {\"op\": \"pane\", \"title\": \"<optional title>\", \"text\": \"<pane body>\"}\n\
@@smah ui {\"op\": \"menu\", \"title\": \"<optional title>\", \"items\": [\"<item>\", ...]}\n\
Rules: one JSON object per payload; text fields at most 64 KiB; menus up to 64 items \
of 512 bytes each. The UI is display-only: it shows things to the user and never \
reports anything back to you. Invalid or oversized payloads come back as \
[ERROR ui ...] results (you may adapt or continue). Keep UI text plain — control \
characters and terminal escape sequences are stripped before rendering.\n",
    );

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
@@smah file directive lines; if you need a file, describe what is missing in your answer instead.\n",
            );
        }
        FsMode::NoneDeclared => {
            prompt.push_str(
                "FILE SURFACE — none. This tool has no file surface; never emit @@smah \
read/write/append directive lines. Work only from the input you are given.\n",
            );
        }
    }

    prompt.push_str("\nOUTPUT RULES:\n");
    prompt.push_str(
        "- Your answer text streams into the answer pane (or stdout when no terminal is \
attached) and may be piped into other programs: output only the answer content — no \
banners, no preamble like \"Sure\", no commentary about being an AI.\n\
- Never wrap the whole answer in a code fence unless the content itself is code.\n\
- If the invocation says the input was truncated, treat it as partial: work \
with what is shown and never claim to have seen the rest.\n",
    );

    if let Some(seed) = interface {
        prompt.push_str(
            "\nUI LAYOUT SEED (from the tool author — a description of the \
intended UI arrangement; honor it with `ui` ops where it helps the user, it binds \
no additional behavior):\n",
        );
        prompt.push_str(seed.trim_end());
        prompt.push('\n');
    }

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

fn scan_error_detail(err: &ScanError) -> String {
    match err {
        ScanError::Protocol { detail } => detail.clone(),
        ScanError::UnknownVerb { verb, known } => {
            format!(
                "unknown directive verb '{verb}' (known: {})",
                known.join(", ")
            )
        }
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_tools::driver::Role;
    use std::cell::RefCell;
    use std::rc::Rc;

    // ------------------------------------------------------------------
    // Test doubles
    // ------------------------------------------------------------------

    /// Sink collecting content + notices in order.
    #[derive(Default, Clone)]
    struct VecSink {
        lines: Rc<RefCell<Vec<(bool, String)>>>, // (is_content, text)
    }

    impl VecSink {
        fn content_text(&self) -> String {
            self.lines
                .borrow()
                .iter()
                .filter(|(c, _)| *c)
                .map(|(_, t)| t.as_str())
                .collect()
        }

        fn notices(&self) -> Vec<String> {
            self.lines
                .borrow()
                .iter()
                .filter(|(c, _)| !*c)
                .map(|(_, t)| t.clone())
                .collect()
        }
    }

    impl Sink for VecSink {
        fn content(&mut self, piece: &str) {
            self.lines.borrow_mut().push((true, piece.to_string()));
        }
        fn notice(&mut self, line: &str) {
            self.lines.borrow_mut().push((false, format!("{line}\n")));
        }
    }

    /// Scripted backend: replays one answer per turn, chunked so streaming
    /// feed paths are exercised; records (model, system, messages) calls.
    struct FakeBackend {
        turns: Vec<String>,
        calls: RefCell<Vec<Call>>,
    }

    struct Call {
        model: String,
        system: String,
        messages: Vec<(Role, String)>,
    }

    impl FakeBackend {
        fn new(turns: Vec<String>) -> Self {
            FakeBackend {
                turns,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn last_messages(&self) -> Vec<(Role, String)> {
            self.calls
                .borrow()
                .last()
                .map(|c| c.messages.clone())
                .unwrap_or_default()
        }

        fn system(&self) -> String {
            self.calls
                .borrow()
                .first()
                .map(|c| c.system.clone())
                .unwrap_or_default()
        }
    }

    impl ChatBackend for FakeBackend {
        fn stream_turn(
            &self,
            model: &str,
            system: &str,
            messages: &[ChatMessage],
            on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, SmahError> {
            let idx = self.calls.borrow().len();
            assert!(
                idx < self.turns.len(),
                "FakeBackend: more turns requested than scripted"
            );
            self.calls.borrow_mut().push(Call {
                model: model.to_string(),
                system: system.to_string(),
                messages: messages
                    .iter()
                    .map(|m| (m.role, m.content.clone()))
                    .collect(),
            });
            let answer = self.turns[idx].clone();
            // Emit in two chunks so the mid-line directive path is covered.
            let (a, b) = answer.split_at(answer.len() / 2);
            on_delta(a);
            on_delta(b);
            Ok(answer)
        }
    }

    fn tool(name: &str, interface: Option<String>, roots: Vec<String>) -> ToolDef {
        ToolDef {
            name: name.to_string(),
            description: "test".to_string(),
            model: None,
            driver: "tui".to_string(),
            interface,
            constraints: vec![],
            allow_roots: roots,
            body: "You are a test persona.".to_string(),
            path: std::path::PathBuf::from(format!("/tmp/{name}.md")),
        }
    }

    fn tmp_audit(tag: &str) -> (tempfile::TempDir, Audit) {
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join(format!("{tag}.jsonl")), "run");
        (dir, audit)
    }

    fn audit_lines(path: &std::path::Path) -> Vec<String> {
        std::fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|l| l.to_string())
            .collect()
    }

    const NO_FS: FsMode = FsMode::NoneDeclared;

    // ------------------------------------------------------------------
    // Tests
    // ------------------------------------------------------------------

    #[test]
    fn final_answer_only_is_one_turn() {
        let (dir, audit) = tmp_audit("final");
        let backend = FakeBackend::new(vec!["Hello from the model.".into()]);
        let mut sink = VecSink::default();
        let t = tool("final", None, vec![]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &["query".to_string()],
            None,
            false,
            &NO_FS,
            &audit,
            "run-1",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 1);
        assert!(!out.wrote_files);
        assert_eq!(sink.content_text(), "Hello from the model.");
        assert_eq!(out.messages.len(), 1);
        assert!(audit_lines(&dir.path().join("final.jsonl")).is_empty());
    }

    #[test]
    fn ui_op_renders_audits_and_needs_no_followup() {
        let (dir, audit) = tmp_audit("ui");
        let backend = FakeBackend::new(vec![
            "watch this\n@@smah ui {\"op\":\"status\",\"text\":\"ready\"}\ndone".to_string(),
        ]);
        let mut sink = VecSink::default();
        let t = tool("ui", None, vec![]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-2",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 1);
        assert_eq!(sink.content_text(), "watch this\ndone");
        // Line fallback summarizes the op on stderr (metadata only — no text).
        let notices = sink.notices().join("");
        assert!(notices.contains("[ui op status"), "notices: {notices}");
        assert!(
            !notices.contains("ready"),
            "ui text must not echo: {notices}"
        );
        // Model sees the OK result... but there was no follow-up turn, so no
        // result block was ever sent — messages stay at 1.
        assert_eq!(out.messages.len(), 1);
        // Audit: one allowed attempt, op=ui, path empty.
        let lines = audit_lines(&dir.path().join("ui.jsonl"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"op\":\"ui\""));
        assert!(lines[0].contains("\"decision\":\"allowed\""));
        assert!(lines[0].contains("\"path\":\"\""));
    }

    #[test]
    fn bad_ui_payload_is_soft_error_and_follows_up() {
        let (dir, audit) = tmp_audit("badui");
        let backend = FakeBackend::new(vec!["@@smah ui not-json".to_string(), "fixed".to_string()]);
        let mut sink = VecSink::default();
        let t = tool("badui", None, vec![]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-3",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 2);
        // The model received the [ERROR ui] result in turn 2's messages.
        let msgs = backend.last_messages();
        assert_eq!(msgs.len(), 3);
        assert_eq!(msgs[2].0, Role::User);
        assert!(msgs[2].1.contains("[ERROR ui"));
        assert!(msgs[2].1.contains("not valid JSON"));
        // Attempt recorded as allowed (the directive itself parsed); the
        // refusal is the soft result line.
        let lines = audit_lines(&dir.path().join("badui.jsonl"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"op\":\"ui\""));
    }

    #[test]
    fn oversized_ui_payload_is_soft_error() {
        let (_dir, audit) = tmp_audit("bigui");
        let payload = format!(
            "@@smah ui {{\"op\":\"status\",\"text\":\"{}\"}}",
            "x".repeat(WRITE_CONTENT_MAX + 1)
        );
        let backend = FakeBackend::new(vec![payload, "ok".to_string()]);
        let mut sink = VecSink::default();
        let t = tool("bigui", None, vec![]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-4",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 2);
        let msgs = backend.last_messages();
        assert!(msgs[2].1.contains("payload exceeds"));
    }

    #[test]
    fn unknown_verb_is_protocol_error() {
        let (_dir, audit) = tmp_audit("zap");
        let backend = FakeBackend::new(vec!["@@smah zap {}".to_string()]);
        let mut sink = VecSink::default();
        let t = tool("zap", None, vec![]);
        let err = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-5",
            4,
        )
        .expect_err("protocol error");
        match err {
            SmahError::DirectiveProtocol { tool, detail } => {
                assert_eq!(tool, "zap");
                assert!(detail.contains("unknown directive verb 'zap'"), "{detail}");
            }
            other => panic!("expected DirectiveProtocol, got {other:?}"),
        }
    }

    #[test]
    fn read_directive_follows_up_with_results_block() {
        let (dir, audit) = tmp_audit("read");
        let fsroot = tempfile::tempdir().expect("tempdir");
        std::fs::write(fsroot.path().join("data.txt"), "file body").expect("write");
        let backend = FakeBackend::new(vec![
            "@@smah read {\"path\": \"data.txt\"}".to_string(),
            "The file says: file body".to_string(),
        ]);
        let mut sink = VecSink::default();
        let t = tool("read", None, vec![fsroot.path().display().to_string()]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &FsMode::Enabled(vec![fsroot.path().to_path_buf()]),
            &audit,
            "run-6",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 2);
        let msgs = backend.last_messages();
        assert!(msgs[2].1.contains("[FILE "));
        assert!(msgs[2].1.contains("file body"));
        assert!(sink.content_text().ends_with("The file says: file body"));
        let lines = audit_lines(&dir.path().join("read.jsonl"));
        assert_eq!(lines.len(), 1);
        assert!(lines[0].contains("\"op\":\"read\""));
        assert!(lines[0].contains("\"decision\":\"allowed\""));
    }

    #[test]
    fn write_only_turn_finishes_without_followup() {
        let fsroot = tempfile::tempdir().expect("tempdir");
        let (_dir, audit) = tmp_audit("write");
        let backend = FakeBackend::new(vec![
            "Writing it now.\n@@smah write {\"path\": \"out.txt\", \"content\": \"hi\"}"
                .to_string(),
        ]);
        let mut sink = VecSink::default();
        let t = tool("write", None, vec![fsroot.path().display().to_string()]);
        let out = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &FsMode::Enabled(vec![fsroot.path().to_path_buf()]),
            &audit,
            "run-7",
            4,
        )
        .expect("run");
        assert_eq!(out.turns, 1);
        assert!(out.wrote_files);
        assert_eq!(
            std::fs::read_to_string(fsroot.path().join("out.txt")).expect("read"),
            "hi"
        );
    }

    #[test]
    fn none_declared_read_is_violation() {
        let (_dir, audit) = tmp_audit("violation");
        let backend = FakeBackend::new(vec!["@@smah read {\"path\": \"x\"}".to_string()]);
        let mut sink = VecSink::default();
        let t = tool("violation", None, vec![]);
        let err = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-8",
            4,
        )
        .expect_err("violation");
        match err {
            SmahError::DirectiveViolation { tool, detail } => {
                assert_eq!(tool, "violation");
                assert!(detail.contains("no allow_roots"), "{detail}");
            }
            other => panic!("expected DirectiveViolation, got {other:?}"),
        }
    }

    #[test]
    fn interface_seed_enters_system_prompt_and_caps() {
        let (_dir, audit) = tmp_audit("seed");
        let backend = FakeBackend::new(vec!["ok".to_string()]);
        let mut sink = VecSink::default();
        let t = tool(
            "seed",
            Some("two panes: answer left, log right".into()),
            vec![],
        );
        let _ = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-9",
            4,
        )
        .expect("run");
        let system = backend.system();
        assert!(system.contains("UI LAYOUT SEED"), "{system}");
        assert!(system.contains("two panes: answer left, log right"));

        let oversized = tool("seedbig", Some("x".repeat(INTERFACE_SEED_MAX + 1)), vec![]);
        let err = run_with_turns(
            &oversized,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-10",
            4,
        )
        .expect_err("seed cap");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));

        let empty = tool("seedempty", Some(String::new()), vec![]);
        let err = run_with_turns(
            &empty,
            &backend,
            &mut sink,
            "m",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-11",
            4,
        )
        .expect_err("empty seed");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    }

    /// The non-TTY line fallback is byte-for-byte the cli presentation on
    /// base verbs: same answer text, same result-block contract.
    #[test]
    fn non_tty_fallback_matches_cli_shape() {
        let fsroot = tempfile::tempdir().expect("tempdir");
        std::fs::write(fsroot.path().join("a.txt"), "AAA").expect("write");
        let script = vec![
            "@@smah read {\"path\": \"a.txt\"}".to_string(),
            "answer: AAA".to_string(),
        ];

        let (_d1, audit1) = tmp_audit("parity-tui");
        let b1 = FakeBackend::new(script.clone());
        let mut s1 = VecSink::default();
        let t = tool("parity", None, vec![fsroot.path().display().to_string()]);
        let out_tui = run_with_turns(
            &t,
            &b1,
            &mut s1,
            "m",
            &[],
            None,
            false,
            &FsMode::Enabled(vec![fsroot.path().to_path_buf()]),
            &audit1,
            "run-t",
            4,
        )
        .expect("tui run");

        let (_d2, audit2) = tmp_audit("parity-cli");
        let b2 = FakeBackend::new(script.clone());
        let mut s2 = VecSink::default();
        let t_cli = ToolDef {
            driver: "cli".to_string(),
            ..tool("parity", None, vec![fsroot.path().display().to_string()])
        };
        let out_cli = crate::virtual_tools::driver::run_with_turns(
            &t_cli,
            &b2,
            &mut s2,
            "m",
            &[],
            None,
            false,
            &FsMode::Enabled(vec![fsroot.path().to_path_buf()]),
            &audit2,
            "run-c",
            4,
        )
        .expect("cli run");

        assert_eq!(s1.content_text(), s2.content_text());
        assert_eq!(out_tui, out_cli);
    }

    #[test]
    fn tool_file_model_overrides_run_model() {
        let (_dir, audit) = tmp_audit("model");
        let backend = FakeBackend::new(vec!["ok".to_string()]);
        let mut sink = VecSink::default();
        let t = tool("model", None, vec![]);
        let _ = run_with_turns(
            &t,
            &backend,
            &mut sink,
            "alibaba.qwen3.8-flash",
            &[],
            None,
            false,
            &NO_FS,
            &audit,
            "run-12",
            4,
        )
        .expect("run");
        assert_eq!(backend.calls.borrow()[0].model, "alibaba.qwen3.8-flash");
    }
}
