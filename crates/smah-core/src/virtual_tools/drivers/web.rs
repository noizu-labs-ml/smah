//! The **web** virtual-tool driver (#19): interface-first fabricated pages.
//!
//! Division of labor (family plan §2 #19): the Rust side owns the LLM seam
//! — it fabricates the page content as a strict strings-only `content.json`
//! under the tool's cache dir (validate-then-freeze, sha-pinned to the tool
//! file, T-021) and then WATCHES `regen.request`; the sibling Go service
//! (`services/web-vtool/`) is presentation/glue only — it serves the page
//! (auto-escaped templates, T-019) and touches `regen.request` when the
//! page's refresh button is used. No provider, no model calls in Go.
//!
//! Output discipline: stdout stays EMPTY; every driver message — content
//! path and serve instructions included — goes through the notice channel
//! (stderr). The driver itself binds nothing (T-020 is the Go service's
//! loopback bind; the Rust side only writes the cache).

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use serde_json::Value;
use smah_vtool_http::contract::{self};
use smah_vtool_http::pins;
use smah_vtool_http::server::ShutdownSignal;
use smah_vtool_http::sleeper::{Sleeper, Wake};

use crate::errors::SmahError;
use crate::virtual_tools::audit::{cache_dir, Audit, AuditRecord};
use crate::virtual_tools::driver::{
    invocation_block, ChatBackend, ChatMessage, FsMode, RunOutcome, Sink,
};
use crate::virtual_tools::loader::ToolDef;
use crate::virtual_tools::{TURNS_MAX, WRITE_CONTENT_MAX};

/// Generation attempts for the initial content (bounded provider spend).
const CONTENT_ATTEMPTS: usize = 2;

/// Keys allowed in one content object.
const CONTENT_KEYS_MAX: usize = 32;

/// Bytes allowed per string value.
const CONTENT_VALUE_MAX: usize = 64 * 1024;

/// Default watch poll interval (seconds).
const WATCH_INTERVAL_DEFAULT: u64 = 2;

/// Parsed driver flags.
struct Flags {
    watch: bool,
    interval: u64,
    request: Vec<String>,
}

/// Run the web driver.
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
    run_with_dirs(
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
        None,
        None,
    )
}

/// [`run`] with injectable cache root + stop signal (tests; production
/// passes `None`/`None` — ctrl-c ends the watch).
#[allow(clippy::too_many_arguments)]
pub fn run_with_dirs(
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
    cache_root: Option<PathBuf>,
    stop: Option<Arc<ShutdownSignal>>,
) -> Result<RunOutcome, SmahError> {
    let _ = fs_mode; // the web driver has no file surface
    let flags = parse_args(args)?;

    // Interface-first: the tool file must describe the page.
    let interface =
        match &tool.interface {
            Some(text) if !text.trim().is_empty() => text.clone(),
            Some(_) => return Err(SmahError::ToolFileInvalid {
                path: tool.path.display().to_string(),
                detail:
                    "interface: is present but empty — the web driver requires a page description"
                        .to_string(),
            }),
            None => {
                return Err(SmahError::ToolFileInvalid {
                    path: tool.path.display().to_string(),
                    detail: "the web driver requires an `interface:` key describing the page"
                        .to_string(),
                })
            }
        };

    // Content lifecycle: cache + sha-pin to the tool file (T-021).
    let base = cache_root.unwrap_or_else(cache_dir);
    let web_dir = base.join("virtual-tools").join(&tool.name).join("web");
    let content_path = web_dir.join("content.json");
    let regen_path = web_dir.join("regen.request");
    let tool_sha = pins::tool_sha256(&tool.path);

    let mut turns_used: usize = 0;
    let mut cache_hit = false;
    if contract::read_marker(&content_path).as_deref() == Some(tool_sha.as_str()) {
        let cached = std::fs::read_to_string(&content_path)
            .map_err(|err| err.to_string())
            .and_then(|text| {
                validate_content(&text)
                    .map(|_| ())
                    .map_err(|e| e.to_string())
            });
        match cached {
            Ok(()) => {
                cache_hit = true;
                sink.notice(&format!(
                    "smah(virtual): {} content cache hit (sha-pinned to the tool file)",
                    tool.name
                ));
            }
            // Poisoned/marker-forged cache: regenerate — validate-on-load is
            // the gate, the cache is never trusted blindly (T-021).
            Err(reason) => {
                sink.notice(&format!(
                    "smah(virtual): {} content cache rejected ({reason}); regenerating",
                    tool.name
                ));
            }
        }
    }
    if !cache_hit {
        let (used, outcome) = generate_content(
            tool,
            backend,
            sink,
            model,
            &interface,
            &flags.request,
            stdin,
            stdin_partial,
            audit,
            run_id,
            &content_path,
            &tool_sha,
            CONTENT_ATTEMPTS,
        );
        turns_used += used;
        outcome?;
    }

    print_serve_instructions(tool, sink, &content_path);

    if !flags.watch {
        sink.notice(&format!(
            "smah(virtual): {} done (--no-watch: not watching {})",
            tool.name,
            regen_path.display()
        ));
        return Ok(RunOutcome {
            turns: turns_used,
            wrote_files: false,
            messages: Vec::new(),
        });
    }

    watch_loop(
        tool,
        backend,
        sink,
        model,
        &interface,
        audit,
        run_id,
        &content_path,
        &regen_path,
        &mut turns_used,
        max_turns,
        flags.interval,
        stop,
    )?;

    Ok(RunOutcome {
        turns: turns_used,
        wrote_files: false,
        messages: Vec::new(),
    })
}

/// The watch loop: poll `regen.request`'s mtime; each change is one bounded
/// regeneration turn (budget-capped), atomic write, audit. Ends on ctrl-c
/// (or the injected stop) or when the turn budget is exhausted.
#[allow(clippy::too_many_arguments)]
fn watch_loop(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    interface: &str,
    audit: &Audit,
    run_id: &str,
    content_path: &Path,
    regen_path: &Path,
    turns_used: &mut usize,
    max_turns: usize,
    interval: u64,
    stop: Option<Arc<ShutdownSignal>>,
) -> Result<(), SmahError> {
    let sleeper = Sleeper::new().map_err(|err| SmahError::ToolFileInvalid {
        path: content_path.display().to_string(),
        detail: format!("cannot start the watch loop: {err}"),
    })?;
    // A regen request that predates this run is consumed by the fresh
    // content written above — seed from the current mtime.
    let mut last_regen = regen_mtime(regen_path);
    audit_record(
        audit,
        run_id,
        &tool.name,
        "web.serve",
        &content_path.display().to_string(),
        None,
        "allowed",
        "",
    );
    sink.notice(&format!(
        "smah(virtual): {} watching {} (interval {interval}s, ctrl-c to stop)",
        tool.name,
        regen_path.display()
    ));

    loop {
        if sleeper.wait_for(interval, stop.as_deref()) == Wake::Stop {
            break;
        }
        let current = regen_mtime(regen_path);
        if current == last_regen {
            continue;
        }
        last_regen = current;
        if *turns_used >= max_turns {
            audit_record(
                audit,
                run_id,
                &tool.name,
                "web.push",
                &content_path.display().to_string(),
                None,
                "denied",
                &format!("turn budget exhausted ({max_turns} turns)"),
            );
            sink.notice(&format!(
                "smah(virtual): {} regen requested but the turn budget ({max_turns}) is exhausted; stop and rerun to continue",
                tool.name
            ));
            break;
        }
        sink.notice(&format!(
            "smah(virtual): {} regen requested; regenerating content",
            tool.name
        ));
        // One attempt per regen: on failure the previous content stays
        // served (never blank the page on a bad model turn).
        let (used, outcome) = generate_content(
            tool,
            backend,
            sink,
            model,
            interface,
            &[],
            None,
            false,
            audit,
            run_id,
            content_path,
            &pins::tool_sha256(&tool.path),
            1,
        );
        *turns_used += used;
        match outcome {
            Ok(()) => {
                audit_record(
                    audit,
                    run_id,
                    &tool.name,
                    "web.push",
                    &content_path.display().to_string(),
                    None,
                    "allowed",
                    "",
                );
                sink.notice(&format!(
                    "smah(virtual): {} content regenerated (refresh the page to see it)",
                    tool.name
                ));
            }
            Err(err) => {
                sink.notice(&format!(
                    "smah(virtual): {} regen failed ({err}); keeping the previous content",
                    tool.name
                ));
            }
        }
    }
    Ok(())
}

/// Regeneration prompt pieces shared by initial + regen turns.
#[allow(clippy::too_many_arguments)]
fn generate_content(
    tool: &ToolDef,
    backend: &dyn ChatBackend,
    sink: &mut dyn Sink,
    model: &str,
    interface: &str,
    request: &[String],
    stdin: Option<&str>,
    stdin_partial: bool,
    audit: &Audit,
    run_id: &str,
    content_path: &Path,
    tool_sha: &str,
    attempts: usize,
) -> (usize, Result<(), SmahError>) {
    let system = generation_prompt(tool, interface);
    let user = if request.is_empty() && stdin.is_none() {
        "Author the page content now.".to_string()
    } else {
        invocation_block(request, stdin, stdin_partial)
    };
    let mut messages = vec![ChatMessage::user(user)];
    let mut last_reason = String::new();

    for attempt in 1..=attempts {
        if attempt > 1 {
            sink.notice(&format!(
                "smah(virtual): {} content generation attempt {attempt}/{attempts}",
                tool.name
            ));
        }
        let answer = match backend.stream_turn(model, &system, &messages, &mut |_piece| {
            // Content turns stream nowhere (stdout stays clean).
        }) {
            Ok(answer) => answer,
            Err(err) => return (attempt, Err(err)),
        };
        let candidate = pins::strip_fences(&answer);
        match validate_content(candidate) {
            Ok(()) => {
                if let Err(err) = contract::write_atomic(content_path, candidate) {
                    return (
                        attempt,
                        Err(SmahError::ToolFileInvalid {
                            path: content_path.display().to_string(),
                            detail: format!("cannot write content cache: {err}"),
                        }),
                    );
                }
                if let Err(err) = contract::write_marker(content_path, tool_sha) {
                    return (
                        attempt,
                        Err(SmahError::ToolFileInvalid {
                            path: content_path.display().to_string(),
                            detail: format!("cannot write content marker: {err}"),
                        }),
                    );
                }
                audit_record(
                    audit,
                    run_id,
                    &tool.name,
                    "web.generate",
                    &content_path.display().to_string(),
                    Some(candidate.len()),
                    "allowed",
                    "",
                );
                sink.notice(&format!(
                    "smah(virtual): {} content written: {}",
                    tool.name,
                    content_path.display()
                ));
                return (attempt, Ok(()));
            }
            Err(reason) => {
                last_reason = reason.to_string();
                messages.push(ChatMessage::assistant(answer.clone()));
                messages.push(ChatMessage::user(format!(
                    "The content failed validation: {last_reason}\n\
Respond again with the complete corrected content — exactly one JSON \
object whose values are all strings, raw JSON only, no code fences."
                )));
            }
        }
    }

    audit_record(
        audit,
        run_id,
        &tool.name,
        "web.generate",
        &content_path.display().to_string(),
        None,
        "denied",
        &format!("validation failed after {attempts} attempts: {last_reason}"),
    );
    (
        attempts,
        Err(SmahError::DirectiveLoopOverflow {
            tool: tool.name.clone(),
            turns: attempts,
        }),
    )
}

/// The code-owned content-generation contract.
fn generation_prompt(tool: &ToolDef, interface: &str) -> String {
    let mut prompt = String::new();
    prompt.push_str(
        "You generate page content for a local mock web UI. Respond with \
exactly one JSON object and nothing else — raw JSON, no code fences, no \
commentary, no markdown.\n\n\
Rules: every value in the object MUST be a string; keys are page sections \
(include a \"title\" for the page heading). Sections render as plain text — \
HTML/markup is escaped and shown literally, so do not include tags. At most \
32 keys, each value at most 65536 bytes, the whole object under 131072 \
bytes. When regenerating an existing page, keep the same section keys and \
stay consistent with what the page already says.\n\n\
PAGE DESCRIPTION (from the tool author — the source of truth for what this \
page shows):\n",
    );
    prompt.push_str(interface.trim_end());
    prompt.push_str("\n\n# Tool persona\n\n");
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

/// Strict content validation: a JSON object, every value a string, within
/// the key/value/total caps. The validated raw bytes are what lands on disk
/// and what the Go service serves — never re-serialized.
fn validate_content(text: &str) -> Result<(), ContentError> {
    if text.len() > WRITE_CONTENT_MAX {
        return Err(ContentError::TooLarge {
            bytes: text.len(),
            max: WRITE_CONTENT_MAX,
        });
    }
    let value: Value =
        serde_json::from_str(text).map_err(|err| ContentError::InvalidJson(err.to_string()))?;
    let obj = value.as_object().ok_or(ContentError::NotAnObject)?;
    if obj.len() > CONTENT_KEYS_MAX {
        return Err(ContentError::TooManyKeys {
            keys: obj.len(),
            max: CONTENT_KEYS_MAX,
        });
    }
    for (key, value) in obj {
        let text = value
            .as_str()
            .ok_or_else(|| ContentError::ValueNotAString { key: key.clone() })?;
        if text.len() > CONTENT_VALUE_MAX {
            return Err(ContentError::ValueTooLarge {
                key: key.clone(),
                bytes: text.len(),
                max: CONTENT_VALUE_MAX,
            });
        }
    }
    Ok(())
}

/// Content-validation failures (cache poisoning guard, T-021).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
enum ContentError {
    #[error("content is not valid JSON: {0}")]
    InvalidJson(String),
    #[error("content root must be a JSON object")]
    NotAnObject,
    #[error("content has {keys} keys (max {max})")]
    TooManyKeys { keys: usize, max: usize },
    #[error("value for '{key}' must be a string")]
    ValueNotAString { key: String },
    #[error("value for '{key}' is {bytes} bytes (max {max})")]
    ValueTooLarge {
        key: String,
        bytes: usize,
        max: usize,
    },
    #[error("content is {bytes} bytes (max {max})")]
    TooLarge { bytes: usize, max: usize },
}

/// Where the Go service (or anything) asks for a regeneration.
fn regen_mtime(regen_path: &Path) -> Option<SystemTime> {
    std::fs::metadata(regen_path)
        .and_then(|meta| meta.modified())
        .ok()
}

fn parse_args(args: &[String]) -> Result<Flags, SmahError> {
    let mut watch = true;
    let mut interval = WATCH_INTERVAL_DEFAULT;
    let mut request: Vec<String> = Vec::new();
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == "--no-watch" {
            watch = false;
        } else if arg == "--watch-interval" {
            let value = iter.next().ok_or_else(|| SmahError::Usage {
                message: "--watch-interval requires a value (e.g. --watch-interval 5)".to_string(),
            })?;
            interval = value.parse().map_err(|_| SmahError::Usage {
                message: format!("--watch-interval expects seconds, got '{value}'"),
            })?;
        } else if let Some(value) = arg.strip_prefix("--watch-interval=") {
            interval = value.parse().map_err(|_| SmahError::Usage {
                message: format!("--watch-interval expects seconds, got '{value}'"),
            })?;
        } else {
            request.push(arg.clone());
        }
    }
    Ok(Flags {
        watch,
        interval,
        request,
    })
}

/// The serve instructions: content path + how to run the sibling service +
/// the loopback URL (loud disclosure; nothing binds here).
fn print_serve_instructions(tool: &ToolDef, sink: &mut dyn Sink, content_path: &Path) {
    sink.notice(&format!(
        "smah(virtual): {} content ready at {}",
        tool.name,
        content_path.display()
    ));
    sink.notice(&format!(
        "smah(virtual): {} serve it: go run ./services/web-vtool --content {} --tool {}",
        tool.name,
        content_path.display(),
        tool.name
    ));
    sink.notice(&format!(
        "smah(virtual): {} then open http://127.0.0.1:8090 (loopback only, fabricated data)",
        tool.name
    ));
}

/// Metadata-only audit line (best-effort like the other drivers).
#[allow(clippy::too_many_arguments)]
fn audit_record(
    audit: &Audit,
    run_id: &str,
    tool_name: &str,
    op: &str,
    path: &str,
    bytes: Option<usize>,
    decision: &'static str,
    reason: &str,
) {
    let rec = AuditRecord {
        ts: chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false),
        run: run_id.to_string(),
        tool: tool_name.to_string(),
        op: op.to_string(),
        path: path.to_string(),
        resolved: None,
        bytes,
        decision,
        reason: reason.to_string(),
        turn: 0,
    };
    if let Err(reason) = audit.record(&rec) {
        eprintln!("smah(virtual): audit log write failed: {reason}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::virtual_tools::driver::Role;
    use std::cell::RefCell;
    use std::rc::Rc;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    // ------------------------------------------------------------------
    // Doubles
    // ------------------------------------------------------------------

    #[derive(Default, Clone)]
    struct VecSink {
        lines: Rc<RefCell<Vec<String>>>,
    }

    impl VecSink {
        fn notices(&self) -> Vec<String> {
            self.lines.borrow().clone()
        }

        fn find(&self, needle: &str) -> Option<String> {
            self.lines
                .borrow()
                .iter()
                .find(|l| l.contains(needle))
                .cloned()
        }
    }

    impl Sink for VecSink {
        fn content(&mut self, piece: &str) {
            self.lines.borrow_mut().push(piece.to_string());
        }
        fn notice(&mut self, line: &str) {
            self.lines.borrow_mut().push(format!("{line}\n"));
        }
    }

    /// Scripted backend replaying answers (pops from the END).
    struct Scripted {
        answers: std::sync::Mutex<Vec<String>>,
        calls: std::sync::Mutex<Vec<Vec<(Role, String)>>>,
        systems: std::sync::Mutex<Vec<String>>,
    }

    impl Scripted {
        fn new(answers: Vec<String>) -> Self {
            Scripted {
                answers: std::sync::Mutex::new(answers),
                calls: std::sync::Mutex::new(Vec::new()),
                systems: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn last_messages(&self) -> Vec<(Role, String)> {
            self.calls
                .lock()
                .expect("lock")
                .last()
                .cloned()
                .unwrap_or_default()
        }
    }

    impl ChatBackend for Scripted {
        fn stream_turn(
            &self,
            model: &str,
            system: &str,
            messages: &[ChatMessage],
            _on_delta: &mut dyn FnMut(&str),
        ) -> Result<String, SmahError> {
            let answer = self
                .answers
                .lock()
                .expect("lock")
                .pop()
                .expect("Scripted: more model turns requested than scripted");
            self.systems.lock().expect("lock").push(system.to_string());
            self.calls.lock().expect("lock").push(
                messages
                    .iter()
                    .map(|m| (m.role, m.content.clone()))
                    .collect(),
            );
            let _ = model;
            Ok(answer)
        }
    }

    /// Sink forwarding lines over a channel (for the watching thread).
    struct ChanSink(mpsc::Sender<String>);

    impl Sink for ChanSink {
        fn content(&mut self, _piece: &str) {}
        fn notice(&mut self, line: &str) {
            let _ = self.0.send(format!("{line}\n"));
        }
    }

    fn tool() -> ToolDef {
        ToolDef {
            name: "mock-page".to_string(),
            description: "test".to_string(),
            model: None,
            driver: "web".to_string(),
            interface: Some("A one-page dashboard for the widget workshop: title, a status headline, three bullet sections.".to_string()),
            constraints: vec![],
            allow_roots: vec![],
            body: "Widget workshop persona.".to_string(),
            path: PathBuf::from("mock-page.md"),
        }
    }

    fn write_tool_file(dir: &Path, name: &str) -> PathBuf {
        let path = dir.join(name);
        std::fs::write(&path, "tool file bytes for sha").expect("write");
        path
    }

    const GOOD_CONTENT: &str = r#"{"title":"Widget Workshop","status":"All systems nominal","notes":"three bullets here"}"#;

    fn tmp_audit(tag: &str) -> (tempfile::TempDir, Audit) {
        let dir = tempfile::tempdir().expect("tempdir");
        let audit = Audit::open_at(&dir.path().join(format!("{tag}.jsonl")), "run");
        (dir, audit)
    }

    fn audit_text(dir: &tempfile::TempDir, tag: &str) -> String {
        std::fs::read_to_string(dir.path().join(format!("{tag}.jsonl"))).unwrap_or_default()
    }

    fn content_path(cache: &Path, tool_name: &str) -> PathBuf {
        cache
            .join("virtual-tools")
            .join(tool_name)
            .join("web")
            .join("content.json")
    }

    fn run_no_watch(
        t: &ToolDef,
        backend: &Scripted,
        cache: &Path,
        audit: &Audit,
        run_id: &str,
        max_turns: usize,
    ) -> (RunOutcome, VecSink) {
        let mut sink = VecSink::default();
        let out = run_with_dirs(
            t,
            backend,
            &mut sink,
            "m",
            &["--no-watch".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            audit,
            run_id,
            max_turns,
            Some(cache.to_path_buf()),
            None,
        )
        .expect("no-watch run");
        (out, sink)
    }

    // ------------------------------------------------------------------
    // Unit
    // ------------------------------------------------------------------

    #[test]
    fn parse_args_flags_and_rest() {
        let flags = parse_args(&[
            "--no-watch".to_string(),
            "--watch-interval".to_string(),
            "5".to_string(),
            "make".to_string(),
            "it".to_string(),
        ])
        .expect("parse");
        assert!(!flags.watch);
        assert_eq!(flags.interval, 5);
        assert_eq!(flags.request, vec!["make".to_string(), "it".to_string()]);

        let flags = parse_args(&["--watch-interval=0".to_string()]).expect("parse");
        assert!(flags.watch);
        assert_eq!(flags.interval, 0);

        let flags = parse_args(&[]).expect("parse");
        assert!(flags.watch);
        assert_eq!(flags.interval, WATCH_INTERVAL_DEFAULT);

        assert!(parse_args(&["--watch-interval".to_string()]).is_err());
        assert!(parse_args(&["--watch-interval".to_string(), "soon".to_string()]).is_err());
    }

    #[test]
    fn content_validation_is_strings_only_and_capped() {
        assert!(validate_content(GOOD_CONTENT).is_ok());
        assert!(validate_content("{\"title\":\"x\"}").is_ok());

        // Not an object / not JSON.
        assert!(validate_content("[\"a\"]").is_err());
        assert!(validate_content("not json").is_err());
        // Non-string value.
        assert_eq!(
            validate_content("{\"title\": 3}"),
            Err(ContentError::ValueNotAString {
                key: "title".to_string()
            })
        );
        assert_eq!(
            validate_content("{\"title\": {\"nested\": true}}"),
            Err(ContentError::ValueNotAString {
                key: "title".to_string()
            })
        );
        // Too many keys.
        let many: Vec<String> = (0..CONTENT_KEYS_MAX + 1)
            .map(|i| format!("\"k{i}\":\"v\""))
            .collect();
        assert_eq!(
            validate_content(&format!("{{{}}}", many.join(","))),
            Err(ContentError::TooManyKeys {
                keys: CONTENT_KEYS_MAX + 1,
                max: CONTENT_KEYS_MAX
            })
        );
        // Oversized value.
        let big = "x".repeat(CONTENT_VALUE_MAX + 1);
        assert_eq!(
            validate_content(&format!("{{\"title\":\"{big}\"}}")),
            Err(ContentError::ValueTooLarge {
                key: "title".to_string(),
                bytes: CONTENT_VALUE_MAX + 1,
                max: CONTENT_VALUE_MAX
            })
        );
        // Oversized total (valid JSON, many large-but-legal values).
        let chunk = "y".repeat(CONTENT_VALUE_MAX);
        let parts: Vec<String> = (0..3).map(|i| format!("\"k{i}\":\"{chunk}\"")).collect();
        assert!(matches!(
            validate_content(&format!("{{{}}}", parts.join(","))),
            Err(ContentError::TooLarge { .. })
        ));
    }

    #[test]
    fn requires_interface() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.interface = None;
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let (_d, audit) = tmp_audit("iface");
        let backend = Scripted::new(vec![]);
        let mut sink = VecSink::default();
        let err = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "m",
            &["--no-watch".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect_err("interface required");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));

        let mut t_empty = tool();
        t_empty.interface = Some("   ".to_string());
        let err = run_with_dirs(
            &t_empty,
            &backend,
            &mut sink,
            "m",
            &["--no-watch".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect_err("empty interface");
        assert!(matches!(err, SmahError::ToolFileInvalid { .. }));
    }

    // ------------------------------------------------------------------
    // Generation + cache lifecycle (--no-watch)
    // ------------------------------------------------------------------

    #[test]
    fn generates_content_writes_cache_and_audits() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let (_d, audit) = tmp_audit("gen");
        let backend = Scripted::new(vec![GOOD_CONTENT.to_string()]);

        let (out, sink) = run_no_watch(&t, &backend, cache.path(), &audit, "run-gen", 4);
        assert_eq!(out.turns, 1);
        let path = content_path(cache.path(), "mock-page");
        assert_eq!(
            std::fs::read_to_string(&path).expect("content"),
            GOOD_CONTENT
        );
        assert_eq!(
            contract::read_marker(&path).as_deref(),
            Some(pins::tool_sha256(&t.path).as_str())
        );
        assert!(sink.find("content ready at").is_some());
        assert!(sink.find("http://127.0.0.1:8090").is_some());
        assert!(sink.find("--no-watch: not watching").is_some());
        let text = audit_text(&_d, "gen");
        assert!(text.contains("\"op\":\"web.generate\""), "{text}");
        assert!(text.contains("\"decision\":\"allowed\""), "{text}");
        assert!(!text.contains("\"op\":\"web.serve\""), "{text}");

        // The generation turn carried the interface seed + persona.
        let msgs = backend.last_messages();
        assert!(
            msgs[0].1.contains("Author the page content now"),
            "{:?}",
            msgs[0].1
        );
        // (system prompt asserted via the backend's systems log)
        assert!(backend
            .systems
            .lock()
            .expect("lock")
            .last()
            .unwrap()
            .contains("PAGE DESCRIPTION"));
    }

    #[test]
    fn cache_hit_skips_generation() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let (_d, audit) = tmp_audit("hit");
        {
            let backend = Scripted::new(vec![GOOD_CONTENT.to_string()]);
            run_no_watch(&t, &backend, cache.path(), &audit, "run-1", 4);
        }
        // Second run with an EMPTY backend: any model call panics the test.
        let backend = Scripted::new(vec![]);
        let (out, sink) = run_no_watch(&t, &backend, cache.path(), &audit, "run-2", 4);
        assert!(sink.find("content cache hit").is_some());
        assert_eq!(out.turns, 0);
        let text = audit_text(&_d, "hit");
        assert_eq!(text.matches("\"op\":\"web.generate\"").count(), 1);
    }

    #[test]
    fn poisoned_cache_is_regenerated() {
        let cache = tempfile::tempdir().expect("cache");
        let path = content_path(cache.path(), "mock-page");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, "{\"title\": 42}").expect("poison");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        // Marker pinned to the real sha: marker-current + corrupt body must
        // be caught by validate-on-load (T-021).
        contract::write_marker(&path, &pins::tool_sha256(&t.path)).expect("marker");

        let (_d, audit) = tmp_audit("poison");
        let backend = Scripted::new(vec![GOOD_CONTENT.to_string()]);
        let (out, sink) = run_no_watch(&t, &backend, cache.path(), &audit, "run-poison", 4);
        assert!(sink.find("content cache rejected").is_some());
        assert_eq!(std::fs::read_to_string(&path).expect("read"), GOOD_CONTENT);
        assert_eq!(out.turns, 1);
    }

    #[test]
    fn stale_tool_file_regenerates() {
        let cache = tempfile::tempdir().expect("cache");
        let path = content_path(cache.path(), "mock-page");
        std::fs::create_dir_all(path.parent().expect("parent")).expect("dirs");
        std::fs::write(&path, GOOD_CONTENT).expect("content");
        contract::write_marker(&path, "stale-sha").expect("marker");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");

        let (_d, audit) = tmp_audit("stale");
        let backend = Scripted::new(vec![GOOD_CONTENT.to_string()]);
        let (out, _sink) = run_no_watch(&t, &backend, cache.path(), &audit, "run-stale", 4);
        assert_eq!(out.turns, 1);
        assert_eq!(
            contract::read_marker(&path).as_deref(),
            Some(pins::tool_sha256(&t.path).as_str())
        );
    }

    #[test]
    fn invalid_content_retries_then_fails() {
        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let (_d, audit) = tmp_audit("fail");
        let bad = "{\"title\": 42}".to_string();
        let backend = Scripted::new(vec![bad.clone(), bad]);
        let err = run_with_dirs(
            &t,
            &backend,
            &mut VecSink::default(),
            "m",
            &["--no-watch".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-fail",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect_err("generation failed");
        assert!(
            matches!(err, SmahError::DirectiveLoopOverflow { .. }),
            "{err:?}"
        );
        let text = audit_text(&_d, "fail");
        assert!(text.contains("\"decision\":\"denied\""), "{text}");
        assert!(
            text.contains("validation failed after 2 attempts"),
            "{text}"
        );
        // No content file was written.
        assert!(!content_path(cache.path(), "mock-page").exists());
    }

    // ------------------------------------------------------------------
    // Watch loop
    // ------------------------------------------------------------------

    /// Run the driver (watch mode) on a worker thread; the test thread
    /// drives regen.request and collects notices over a channel.
    struct Watched {
        audit_dir: tempfile::TempDir,
        content_path: PathBuf,
        regen_path: PathBuf,
        stop: Arc<ShutdownSignal>,
        lines: mpsc::Receiver<String>,
    }

    impl Watched {
        fn touch_regen(&self) {
            std::fs::write(&self.regen_path, b"regen").expect("touch");
        }

        fn wait_for_notice(&self, needle: &str, timeout: Duration) -> String {
            let deadline = Instant::now() + timeout;
            loop {
                if let Ok(line) = self
                    .lines
                    .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                {
                    if line.contains(needle) {
                        return line;
                    }
                }
                if Instant::now() > deadline {
                    panic!("notice containing '{needle}' never arrived");
                }
            }
        }

        fn stop(self) {
            self.stop.notify_one();
        }
    }

    fn watch_with(answers: Vec<String>, cache: &Path, max_turns: usize) -> Watched {
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let t = Arc::new(t);
        let audit_dir = tempfile::tempdir().expect("auditdir");
        let backend = Arc::new(Scripted::new(answers));

        let content_path = content_path(cache, "mock-page");
        let regen_path = cache
            .join("virtual-tools")
            .join("mock-page")
            .join("web")
            .join("regen.request");
        let (line_tx, line_rx) = mpsc::channel::<String>();
        let stop = Arc::new(ShutdownSignal::new());

        let t_run = t;
        let backend_run = backend.clone();
        let audit_path = audit_dir.path().join("watch.jsonl");
        let cache_root = cache.to_path_buf();
        let stop_run = stop.clone();
        std::thread::spawn(move || {
            let audit = Audit::open_at(&audit_path, "run");
            let mut sink = ChanSink(line_tx);
            let _ = run_with_dirs(
                &t_run,
                backend_run.as_ref(),
                &mut sink,
                "m",
                &[],
                None,
                false,
                &FsMode::NoneDeclared,
                &audit,
                "run-watch",
                max_turns,
                Some(cache_root),
                Some(stop_run),
            );
        });

        Watched {
            audit_dir,
            content_path,
            regen_path,
            stop,
            lines: line_rx,
        }
    }

    #[test]
    fn watch_regenerates_on_request_and_audits_push() {
        let cache = tempfile::tempdir().expect("cache");
        let regenerated =
            r#"{"title":"Widget Workshop","status":"Regenerated!","notes":"new notes"}"#;
        let watched = watch_with(
            vec![regenerated.to_string(), GOOD_CONTENT.to_string()],
            cache.path(),
            8,
        );

        // Initial content generated before the watch starts.
        watched.wait_for_notice("watching", Duration::from_secs(15));
        assert_eq!(
            std::fs::read_to_string(&watched.content_path).expect("content"),
            GOOD_CONTENT
        );

        // Request a regen; the loop picks it up within the interval.
        watched.touch_regen();
        watched.wait_for_notice("content regenerated", Duration::from_secs(15));
        assert_eq!(
            std::fs::read_to_string(&watched.content_path).expect("regenerated"),
            regenerated
        );

        let text = audit_text(&watched.audit_dir, "watch");
        assert!(text.contains("\"op\":\"web.serve\""), "{text}");
        assert!(text.contains("\"op\":\"web.push\""), "{text}");
        // allowed: initial generate + serve + regen generate + push.
        assert_eq!(text.matches("\"decision\":\"allowed\"").count(), 4);

        watched.stop();
    }

    #[test]
    fn watch_budget_exhaustion_stops_the_loop() {
        let cache = tempfile::tempdir().expect("cache");
        // Budget 1: the initial generation consumes it; the regen request
        // must be denied and end the watch (not spin forever).
        let watched = watch_with(vec![GOOD_CONTENT.to_string()], cache.path(), 1);
        watched.wait_for_notice("watching", Duration::from_secs(15));
        watched.touch_regen();
        watched.wait_for_notice("turn budget", Duration::from_secs(15));

        // The previous content is untouched; the loop ended (join proves it).
        assert_eq!(
            std::fs::read_to_string(&watched.content_path).expect("content"),
            GOOD_CONTENT
        );
        let text = audit_text(&watched.audit_dir, "watch");
        assert!(text.contains("\"decision\":\"denied\""), "{text}");

        watched.stop();
    }

    #[test]
    fn watch_failed_regen_keeps_previous_content() {
        let cache = tempfile::tempdir().expect("cache");
        // Answers popped END-first: initial GOOD, regen INVALID (one
        // attempt, kept old content, watch continues).
        let watched = watch_with(
            vec!["{\"title\": 42}".to_string(), GOOD_CONTENT.to_string()],
            cache.path(),
            8,
        );
        watched.wait_for_notice("watching", Duration::from_secs(15));
        watched.touch_regen();
        watched.wait_for_notice("regen failed", Duration::from_secs(15));
        assert_eq!(
            std::fs::read_to_string(&watched.content_path).expect("content"),
            GOOD_CONTENT
        );
        let text = audit_text(&watched.audit_dir, "watch");
        assert!(text.contains("\"decision\":\"denied\""), "{text}");

        watched.stop();
    }

    // ------------------------------------------------------------------
    // Real-model e2e (env-gated, #[ignore], never in CI)
    // ------------------------------------------------------------------

    /// ```text
    /// SMAH_ALIBABA_API_KEY=... [ALIBABA_BASE_URL=...] \
    ///   cargo test -p smah-core -- --ignored web_real_model_content
    /// ```
    #[test]
    #[ignore = "real-model e2e: set SMAH_ALIBABA_API_KEY (+ ALIBABA_BASE_URL) to run"]
    fn web_real_model_content() {
        let key = std::env::var("SMAH_ALIBABA_API_KEY").unwrap_or_default();
        if key.is_empty() {
            eprintln!("skipping: SMAH_ALIBABA_API_KEY not set");
            return;
        }
        let base_url = std::env::var("ALIBABA_BASE_URL").unwrap_or_default();
        let provider = if base_url.is_empty() {
            smah_providers::AlibabaProvider::new(Some(key))
        } else {
            smah_providers::AlibabaProvider::with_base_url(Some(key), base_url)
        }
        .expect("provider");

        struct Adapter(smah_providers::AlibabaProvider);
        impl ChatBackend for Adapter {
            fn stream_turn(
                &self,
                model: &str,
                system: &str,
                messages: &[ChatMessage],
                on_delta: &mut dyn FnMut(&str),
            ) -> Result<String, SmahError> {
                use smah_providers::Provider;
                let mut req_messages: Vec<smah_providers::ChatMessage> = messages
                    .iter()
                    .map(|m| match m.role {
                        Role::User => smah_providers::ChatMessage::user(m.content.clone()),
                        Role::Assistant => {
                            smah_providers::ChatMessage::assistant(m.content.clone())
                        }
                    })
                    .collect();
                req_messages.push(smah_providers::ChatMessage::system(system.to_string()));
                let req = smah_providers::CompletionRequest::new(model, req_messages);
                let completion =
                    self.0
                        .complete(&req)
                        .map_err(|err| SmahError::DirectiveProtocol {
                            tool: "web".to_string(),
                            detail: format!("provider error: {err}"),
                        })?;
                on_delta(completion.text.trim());
                Ok(completion.text)
            }
        }

        let cache = tempfile::tempdir().expect("cache");
        let tool_dir = tempfile::tempdir().expect("tooldir");
        let mut t = tool();
        t.path = write_tool_file(tool_dir.path(), "mock-page.md");
        let (_d, audit) = tmp_audit("e2e");
        let backend = Adapter(provider);
        let mut sink = VecSink::default();
        let out = run_with_dirs(
            &t,
            &backend,
            &mut sink,
            "alibaba.qwen3.8-flash",
            &["--no-watch".to_string()],
            None,
            false,
            &FsMode::NoneDeclared,
            &audit,
            "run-e2e",
            4,
            Some(cache.path().to_path_buf()),
            None,
        )
        .expect("real-model content generation");
        assert_eq!(out.turns, 1);
        let raw =
            std::fs::read_to_string(content_path(cache.path(), "mock-page")).expect("content");
        validate_content(&raw).expect("content validates");
        assert!(raw.contains("title"), "{raw}");
        for line in sink.notices() {
            eprintln!("{line}");
        }
    }
}
