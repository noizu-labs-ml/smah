//! The presentation-state sink: where streamed answers, driver notices,
//! `ui` pane ops, and scroll state accumulate. Pure data — widgets read it,
//! the driver writes it.

/// One `ui {"op":"pane"}` payload (title optional).
pub struct PaneContent {
    pub title: Option<String>,
    pub text: String,
}

/// A model-opened menu (`ui {"op":"menu"}`). v1 menus are display surfaces:
/// arrow keys + Enter move a highlight and echo the selection to the status
/// line; there is no model-visible feedback channel (documented in the
/// driver doc).
pub struct MenuState {
    pub title: Option<String>,
    pub items: Vec<String>,
    pub selected: usize,
}

/// Accumulated TUI state for one run.
pub struct ContentSink {
    answer: String,
    notices: Vec<String>,
    panes: Vec<PaneContent>,
    status_text: Option<String>,
    menu: Option<MenuState>,
    /// Scroll position: lines from the bottom of the answer (0 = live tail).
    scroll_from_bottom: usize,
    /// Cap so a pathological answer cannot grow the TUI unboundedly (the
    /// model is already line-capped; this bounds redraw cost too).
    max_answer_bytes: usize,
}

impl ContentSink {
    pub fn new() -> Self {
        ContentSink {
            answer: String::new(),
            notices: Vec::new(),
            panes: Vec::new(),
            status_text: None,
            menu: None,
            scroll_from_bottom: 0,
            max_answer_bytes: 2 * 1024 * 1024,
        }
    }

    /// Append streamed answer text.
    pub fn push_answer(&mut self, piece: &str) {
        if self.answer.len() >= self.max_answer_bytes {
            return; // answer pane is full: further pieces are dropped (rare, bounded)
        }
        self.answer.push_str(piece);
        if self.answer.len() > self.max_answer_bytes {
            self.answer.truncate(self.max_answer_bytes);
        }
    }

    /// Append a notice line (directive results, refusals).
    pub fn push_notice(&mut self, line: &str) {
        self.notices.push(line.to_string());
        if self.notices.len() > 256 {
            self.notices.remove(0);
        }
    }

    /// `{"op":"status"}` — replace the user status text.
    pub fn set_status_text(&mut self, text: String) {
        self.status_text = Some(text);
    }

    /// `{"op":"pane"}` — add a pane (latest pane is rendered).
    pub fn push_pane(&mut self, title: Option<String>, text: String) {
        self.panes.push(PaneContent { title, text });
        if self.panes.len() > 8 {
            self.panes.remove(0);
        }
    }

    /// `{"op":"menu"}` — open (replace) the menu overlay.
    pub fn open_menu(&mut self, title: Option<String>, items: Vec<String>) {
        self.menu = Some(MenuState {
            title,
            items,
            selected: 0,
        });
    }

    /// Is a menu overlay active?
    pub fn menu_active(&self) -> bool {
        self.menu.is_some()
    }

    /// Close the menu without selecting.
    pub fn close_menu(&mut self) {
        self.menu = None;
    }

    pub fn menu_prev(&mut self) {
        if let Some(menu) = self.menu.as_mut() {
            menu.selected = menu.selected.saturating_sub(1);
        }
    }

    pub fn menu_next(&mut self) {
        if let Some(menu) = self.menu.as_mut() {
            if menu.selected + 1 < menu.items.len() {
                menu.selected += 1;
            }
        }
    }

    /// Echo the highlighted item to the status line and close the menu
    /// (v1: selection is display-only — see `MenuState`).
    pub fn menu_select(&mut self) {
        if let Some(menu) = self.menu.take() {
            if let Some(item) = menu.items.get(menu.selected) {
                self.status_text = Some(format!("selected: {item}"));
            }
        }
    }

    pub fn scroll_up(&mut self) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_add(4);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_from_bottom = self.scroll_from_bottom.saturating_sub(4);
    }

    /// Snap to the live tail.
    pub fn scroll_to_bottom(&mut self) {
        self.scroll_from_bottom = 0;
    }

    /// Is the view at the live tail?
    pub fn at_bottom(&self) -> bool {
        self.scroll_from_bottom == 0
    }

    /// The full answer text.
    pub fn answer(&self) -> &str {
        &self.answer
    }

    /// Answer line count (scroll math is unwrapped-line approximate; the
    /// wrap renderer may visually differ by a few lines — documented v1).
    pub fn answer_lines(&self) -> usize {
        self.answer.lines().count()
            + usize::from(!self.answer.is_empty() && !self.answer.ends_with('\n'))
    }

    /// Top offset (lines from the answer's first line) for a viewport of
    /// `viewport` lines at the current scroll position.
    pub fn answer_scroll_top(&self, viewport: usize) -> usize {
        let total = self.answer_lines();
        let max_top = total.saturating_sub(viewport);
        let from_bottom = self.scroll_from_bottom.min(max_top);
        max_top.saturating_sub(from_bottom)
    }

    /// Notice lines (the directive-results pane).
    pub fn notices(&self) -> &[String] {
        &self.notices
    }

    /// The latest `ui` pane, if any.
    pub fn latest_pane(&self) -> Option<&PaneContent> {
        self.panes.last()
    }

    /// Model-set status text (`ui {"op":"status"}`), if any.
    pub fn status_text(&self) -> Option<&str> {
        self.status_text.as_deref()
    }

    /// The active menu overlay, if any.
    pub fn menu(&self) -> Option<&MenuState> {
        self.menu.as_ref()
    }
}

impl Default for ContentSink {
    fn default() -> Self {
        ContentSink::new()
    }
}
