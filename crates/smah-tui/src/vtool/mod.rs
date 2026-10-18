//! The virtual-tool TUI presentation (#18): answer stream, directive
//! results, status line, and model-driven `ui` ops — minimal widget set,
//! deliberately NOT the M7 widget parity work.
//!
//! Plain strings only: the smah-core driver feeds [`VtoolTui`] content,
//! notices, and validated `ui` op payloads, and pumps crossterm events via
//! [`TtySession`]. This crate never blocks on the model and never sees a
//! tool definition.
//!
//! Injection posture (THREAT-MODEL T-017): every byte that reaches a pane
//! passes through [`sanitize`] first (C0/C1 control characters and DEL are
//! dropped except `\n`/`\t`), and rendering is widget-mediated — model text
//! lands inside a `Paragraph`, never in the pane chrome, never as raw
//! escape sequences. The line-oriented fallback in the driver emits the
//! *unsanitized* stream (byte-for-byte cli parity); sanitization is a TTY
//! presentation concern.

pub mod sink;
pub mod widgets;

use std::io::{self, Stdout};
use std::time::{Duration, Instant};

use crossterm::event::{self, Event as TermEvent, KeyCode, KeyEvent, KeyModifiers};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use sink::ContentSink;
use widgets::UiOp;

/// Per-field text cap for `ui` op payloads (the driver additionally caps the
/// whole directive payload at the family `WRITE_CONTENT_MAX`).
pub const UI_TEXT_MAX: usize = 64 * 1024;
/// Menu item caps for `{"op":"menu"}` payloads.
pub const UI_MENU_ITEMS_MAX: usize = 64;
pub const UI_MENU_ITEM_MAX: usize = 512;

/// Strip control characters that could alter terminal state or pane chrome:
/// C0 controls other than `\n`/`\t`, DEL, and the whole C1 range. Escape
/// sequences are therefore inert by construction (T-017).
pub fn sanitize(input: &str) -> String {
    input
        .chars()
        .filter(|c| {
            let code = *c as u32;
            match *c {
                '\n' | '\t' => true,
                _ if code < 0x20 => false,
                '\u{7F}' => false,
                _ if (0x80..=0x9F).contains(&code) => false,
                _ => true,
            }
        })
        .collect()
}

/// What a [`TtySession::pump`] decided after draining pending events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep running.
    Continue,
    /// The user asked to quit (`q`, or `Esc` outside a menu).
    Quit,
}

/// The mutable presentation state of one virtual-tool TUI run.
pub struct VtoolTui {
    sink: ContentSink,
    tool: String,
    model: String,
    turn: usize,
    quit_hint: &'static str,
    user_scrolled: bool,
}

impl VtoolTui {
    /// A fresh TUI for one run (tool + model appear on the status line —
    /// both are host-controlled, never model-controlled).
    pub fn new(tool: &str, model: &str) -> Self {
        VtoolTui {
            sink: ContentSink::new(),
            tool: tool.to_string(),
            model: model.to_string(),
            turn: 0,
            quit_hint: "q quit · pgup/pgdn scroll",
            user_scrolled: false,
        }
    }

    /// Streamed answer piece (sanitized on entry).
    pub fn content(&mut self, piece: &str) {
        self.sink.push_answer(&sanitize(piece));
        self.auto_scroll();
    }

    /// A driver notice line (directive results, refusals — sanitized).
    pub fn notice(&mut self, line: &str) {
        self.sink.push_notice(&sanitize(line));
    }

    /// The turn counter shown on the status line.
    pub fn set_turn(&mut self, turn: usize) {
        self.turn = turn;
    }

    /// Apply one validated `ui` op payload (`{"op": "status"|"pane"|"menu",
    /// ...}`). `Err` carries a one-line reason — the driver turns it into a
    /// soft `[ERROR ui ...]` result for the model, never a protocol abort.
    pub fn ui_op(&mut self, payload: &str) -> Result<(), String> {
        let op = parse_ui_op(payload)?;
        match op {
            UiOp::Status { text } => {
                self.sink.set_status_text(text);
            }
            UiOp::Pane { title, text } => {
                self.sink.push_pane(title, text);
                self.auto_scroll();
            }
            UiOp::Menu { title, items } => {
                self.sink.open_menu(title, items);
            }
        }
        Ok(())
    }

    fn auto_scroll(&mut self) {
        if !self.user_scrolled {
            self.sink.scroll_to_bottom();
        }
    }

    /// The sink view widgets render from.
    pub fn sink(&self) -> &ContentSink {
        &self.sink
    }

    /// Tool name (status line).
    pub fn tool(&self) -> &str {
        &self.tool
    }

    /// Model id (status line).
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Current turn (status line).
    pub fn turn(&self) -> usize {
        self.turn
    }

    /// The static key hint (status line).
    pub fn quit_hint(&self) -> &'static str {
        self.quit_hint
    }
}

/// Validate + decode one raw `ui` op payload. Exposed for the driver's
/// tests (the same validation runs in [`VtoolTui::ui_op`]).
pub fn parse_ui_op(payload: &str) -> Result<UiOp, String> {
    if payload.len() > crate::vtool::UI_TEXT_MAX * 2 {
        return Err(format!(
            "ui payload exceeds {} bytes",
            crate::vtool::UI_TEXT_MAX * 2
        ));
    }
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|err| format!("ui payload is not valid JSON: {err}"))?;
    let obj = value
        .as_object()
        .ok_or_else(|| "ui payload must be a JSON object".to_string())?;
    match obj.get("op").and_then(|v| v.as_str()) {
        Some("status") => {
            let text = require_text(obj, "text", "status")?;
            Ok(UiOp::Status { text })
        }
        Some("pane") => {
            let text = require_text(obj, "text", "pane")?;
            if text.len() > UI_TEXT_MAX {
                return Err(format!("pane text exceeds {UI_TEXT_MAX} bytes"));
            }
            let title = match obj.get("title") {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => Some(
                    v.as_str()
                        .ok_or_else(|| "'title' must be a string".to_string())?
                        .to_string(),
                ),
            };
            Ok(UiOp::Pane { title, text })
        }
        Some("menu") => {
            let title = match obj.get("title") {
                None | Some(serde_json::Value::Null) => None,
                Some(v) => Some(
                    v.as_str()
                        .ok_or_else(|| "'title' must be a string".to_string())?
                        .to_string(),
                ),
            };
            let items_value = obj
                .get("items")
                .ok_or_else(|| "menu payload requires 'items'".to_string())?;
            let list = items_value
                .as_array()
                .ok_or_else(|| "'items' must be an array of strings".to_string())?;
            if list.len() > UI_MENU_ITEMS_MAX {
                return Err(format!("menu exceeds {UI_MENU_ITEMS_MAX} items"));
            }
            let mut items = Vec::with_capacity(list.len());
            for item in list {
                let text = item
                    .as_str()
                    .ok_or_else(|| "'items' entries must be strings".to_string())?;
                if text.len() > UI_MENU_ITEM_MAX {
                    return Err(format!("menu item exceeds {UI_MENU_ITEM_MAX} bytes"));
                }
                items.push(text.to_string());
            }
            Ok(UiOp::Menu { title, items })
        }
        Some(other) => Err(format!(
            "unknown ui op '{other}' (known: status, pane, menu)"
        )),
        None => Err("ui payload requires an 'op' field (status, pane, menu)".to_string()),
    }
}

fn require_text(
    obj: &serde_json::Map<String, serde_json::Value>,
    key: &str,
    op: &str,
) -> Result<String, String> {
    let value = obj
        .get(key)
        .ok_or_else(|| format!("'{op}' payload requires '{key}'"))?;
    let text = value
        .as_str()
        .ok_or_else(|| format!("'{key}' must be a string"))?;
    if text.len() > UI_TEXT_MAX {
        return Err(format!("'{key}' exceeds {UI_TEXT_MAX} bytes"));
    }
    Ok(text.to_string())
}

/// Terminal lifecycle guard: raw mode + alternate screen, restored on drop
/// (every exit path — run outcome, protocol error, model error, panic-safe
/// best effort).
pub struct TerminalGuard {
    active: bool,
}

impl TerminalGuard {
    fn enter() -> io::Result<Self> {
        enable_raw_mode()?;
        execute!(io::stdout(), EnterAlternateScreen)?;
        Ok(TerminalGuard { active: true })
    }
}

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        if self.active {
            let _ = disable_raw_mode();
            let _ = execute!(io::stdout(), LeaveAlternateScreen);
        }
    }
}

/// A live terminal session: owns the ratatui `Terminal`, the guard, and the
/// app. The driver calls [`pump`](TtySession::pump) from its streaming
/// callbacks and between turns; redraws are throttled to one per ~16 ms.
pub struct TtySession {
    terminal: Terminal<CrosstermBackend<Stdout>>,
    _guard: TerminalGuard,
    app: VtoolTui,
    last_redraw: Instant,
    pending_quit: bool,
}

impl TtySession {
    /// Enter raw mode + alt screen and build the session. Fails (cleanly,
    /// caller falls back to the line presentation) when the terminal cannot
    /// be configured.
    pub fn new(tool: &str, model: &str) -> io::Result<Self> {
        let guard = TerminalGuard::enter()?;
        let backend = CrosstermBackend::new(io::stdout());
        let mut terminal = Terminal::new(backend)?;
        terminal.clear()?;
        Ok(TtySession {
            terminal,
            _guard: guard,
            app: VtoolTui::new(tool, model),
            last_redraw: Instant::now() - Duration::from_millis(1000),
            pending_quit: false,
        })
    }

    /// The app (feed content/notices/ui ops through it).
    pub fn app(&mut self) -> &mut VtoolTui {
        &mut self.app
    }

    /// Drain pending crossterm events (non-blocking), handle keys, and
    /// redraw if the throttle window elapsed.
    pub fn pump(&mut self, budget: Duration) -> Flow {
        while event::poll(Duration::ZERO).unwrap_or(false) {
            if let Ok(term_event) = event::read() {
                match term_event {
                    TermEvent::Key(key) => self.handle_key(key),
                    TermEvent::Resize(_, _) => self.force_redraw(),
                    _ => {}
                }
            }
        }
        if self.last_redraw.elapsed() >= budget {
            self.force_redraw();
        }
        if self.pending_quit {
            Flow::Quit
        } else {
            Flow::Continue
        }
    }

    /// Redraw now (the driver calls this after content bursts).
    pub fn force_redraw(&mut self) {
        let _ = self.terminal.draw(|frame| {
            crate::vtool::widgets::draw(frame, &self.app);
        });
        self.last_redraw = Instant::now();
    }

    fn handle_key(&mut self, key: KeyEvent) {
        // Ctrl-C is an explicit quit everywhere (the driver owns no signal
        // handling on the TTY path).
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            self.pending_quit = true;
            return;
        }
        if self.app.sink.menu_active() {
            match key.code {
                KeyCode::Esc => self.app.sink.close_menu(),
                KeyCode::Up | KeyCode::Char('k') => self.app.sink.menu_prev(),
                KeyCode::Down | KeyCode::Char('j') => self.app.sink.menu_next(),
                KeyCode::Enter => self.app.sink.menu_select(),
                KeyCode::Char('q') => self.pending_quit = true,
                _ => {}
            }
            return;
        }
        match key.code {
            KeyCode::Char('q') => self.pending_quit = true,
            KeyCode::Esc => self.pending_quit = true,
            KeyCode::PageUp => {
                self.app.sink.scroll_up();
                self.app.user_scrolled_moved();
            }
            KeyCode::PageDown => {
                self.app.sink.scroll_down();
                self.app.user_scrolled_moved();
            }
            _ => {}
        }
    }
}

impl VtoolTui {
    fn user_scrolled_moved(&mut self) {
        // Bottom detection lives in the sink; the app just stops forcing
        // auto-scroll until the user returns to the bottom.
        self.user_scrolled = !self.sink.at_bottom();
    }
}
