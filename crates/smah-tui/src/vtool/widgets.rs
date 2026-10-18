//! Minimal widget set for the virtual-tool TUI (#18): answer pane,
//! directive-results pane, status line, and the `ui`-op surfaces (pane +
//! menu overlay). Deliberately NOT M7 widget parity.
//!
//! All text is pre-sanitized by the sink layer; widgets never receive raw
//! model bytes (T-017: widget-mediated rendering, chrome never model-
//! controlled).

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use ratatui::Frame;

use super::sink::ContentSink;

/// A decoded, validated `ui` op payload (see `parse_ui_op`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UiOp {
    /// Replace the model status text.
    Status {
        /// The status text.
        text: String,
    },
    /// Add/replace the side pane.
    Pane {
        /// Optional pane title.
        title: Option<String>,
        /// Pane body text.
        text: String,
    },
    /// Open the menu overlay.
    Menu {
        /// Optional menu title.
        title: Option<String>,
        /// Menu items.
        items: Vec<String>,
    },
}

const CHROME: Style = Style::new().fg(Color::Cyan);
const HINT: Style = Style::new().fg(Color::DarkGray);

/// Render one frame.
pub fn draw(frame: &mut Frame, app: &super::VtoolTui) {
    let sink = app.sink();
    let has_pane = sink.latest_pane().is_some();

    let body = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(1)])
        .split(frame.area());

    let columns = if has_pane {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
            .split(body[0])
    } else {
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(100)])
            .split(body[0])
    };

    let left = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(6)])
        .split(columns[0]);

    draw_answer(frame, left[0], sink);
    draw_notices(frame, left[1], sink);
    if has_pane {
        draw_pane(frame, columns[1], sink);
    }
    draw_status(frame, body[1], app);
    if let Some(menu) = sink.menu() {
        draw_menu(frame, menu, frame.area());
    }
}

fn draw_answer(frame: &mut Frame, area: Rect, sink: &ContentSink) {
    let viewport = area.height.saturating_sub(2) as usize; // minus borders
    let top = sink.answer_scroll_top(viewport);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" answer ")
        .style(CHROME);
    let paragraph = Paragraph::new(sink.answer())
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((top as u16, 0));
    frame.render_widget(paragraph, area);
}

fn draw_notices(frame: &mut Frame, area: Rect, sink: &ContentSink) {
    let lines: Vec<Line> = sink
        .notices()
        .iter()
        .map(|n| Line::from(n.clone()))
        .collect();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" directives ")
        .style(CHROME);
    let paragraph = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .scroll((u16::MAX, 0)); // pin to the tail of the notice list
    frame.render_widget(paragraph, area);
}

fn draw_pane(frame: &mut Frame, area: Rect, sink: &ContentSink) {
    let Some(pane) = sink.latest_pane() else {
        return;
    };
    let title = match &pane.title {
        Some(title) => format!(" {title} "),
        None => " pane ".to_string(),
    };
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .style(CHROME);
    let paragraph = Paragraph::new(pane.text.as_str())
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(paragraph, area);
}

fn draw_status(frame: &mut Frame, area: Rect, app: &super::VtoolTui) {
    let sink = app.sink();
    let model_status = sink
        .status_text()
        .map(|t| format!(" ◆ {t}"))
        .unwrap_or_default();
    let spans = vec![
        Span::styled(
            format!(" {} ", app.tool()),
            Style::new().fg(Color::Black).bg(Color::Cyan),
        ),
        Span::raw(format!(" {} · turn {} ", app.model(), app.turn())),
        Span::styled(model_status, Style::new().fg(Color::Yellow)),
        Span::styled(format!("  [{}]", app.quit_hint()), HINT),
    ];
    frame.render_widget(Line::from(spans), area);
}

fn draw_menu(frame: &mut Frame, menu: &super::sink::MenuState, area: Rect) {
    let width = menu
        .items
        .iter()
        .map(|i| i.chars().count())
        .max()
        .unwrap_or(10)
        .clamp(20, 60) as u16;
    let height = (menu.items.len() as u16 + 2).min(area.height.saturating_sub(2));
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    let box_area = Rect::new(x, y, width.min(area.width), height);

    let title = match &menu.title {
        Some(title) => format!(" {title} "),
        None => " menu ".to_string(),
    };
    frame.render_widget(Clear, box_area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title)
        .border_style(CHROME)
        .style(Style::new().bg(Color::Reset));
    let inner = block.inner(box_area);
    frame.render_widget(block, box_area);

    for (idx, item) in menu.items.iter().enumerate() {
        if idx as u16 >= inner.height {
            break;
        }
        let selected = idx == menu.selected;
        let line = Line::from(vec![
            Span::raw(if selected { "> " } else { "  " }),
            Span::styled(
                item.clone(),
                if selected {
                    Style::new()
                        .fg(Color::Black)
                        .bg(Color::Cyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::new()
                },
            ),
        ]);
        frame.render_widget(
            Paragraph::new(line),
            Rect::new(inner.x, inner.y + idx as u16, inner.width, 1),
        );
    }
}
