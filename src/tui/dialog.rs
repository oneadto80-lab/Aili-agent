//! Dialog overlay system for fullscreen TUI.
//!
//! Dialogs render as centered panels on top of the current view,
//! managed by a `DialogStack` on the App. Currently supports:
//!   - Model selection list
//!   - Help keybinding reference

use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph, Widget};

use crate::config::ResolvedConfig;
use crate::tui::keymap::Action;

/// A simple list-based dialog for model selection.
pub struct ModelDialog {
    pub models: Vec<String>,
    pub selected: usize,
    pub list_state: ListState,
}

impl ModelDialog {
    pub fn new(cfg: &ResolvedConfig) -> Self {
        let models = vec![
            cfg.model.clone(),
            "deepseek-v4-flash".into(),
            "deepseek-v4-pro".into(),
        ];
        let mut state = ListState::default();
        state.select(Some(0));
        Self {
            models,
            selected: 0,
            list_state: state,
        }
    }

    pub fn current_model(&self) -> &str {
        &self.models[self.selected]
    }

    pub fn handle_key(&mut self, action: Action) -> Option<String> {
        match action {
            Action::DialogUp => {
                if self.selected > 0 {
                    self.selected -= 1;
                    self.list_state.select(Some(self.selected));
                }
                None
            }
            Action::DialogDown => {
                if self.selected + 1 < self.models.len() {
                    self.selected += 1;
                    self.list_state.select(Some(self.selected));
                }
                None
            }
            Action::DialogConfirm => Some(self.models[self.selected].clone()),
            Action::DialogClose => None,
            _ => None,
        }
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let dialog_area = centered_popup(area, 44, 7);
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .margin(1)
            .horizontal_margin(2)
            .split(dialog_area);

        // Title
        Paragraph::new(Line::from(Span::styled(
            " Switch Model ",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center)
        .render(chunks[0], buf);

        // List
        let items: Vec<ListItem> = self
            .models
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let style = if i == self.selected {
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::White)
                };
                ListItem::new(Line::from(Span::styled(format!("  {}  ", m), style)))
            })
            .collect();

        let list = List::new(items)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(Color::DarkGray)),
            )
            .highlight_style(
                Style::default()
                    .bg(Color::Rgb(40, 40, 60)),
            );

        StatefulWidget::render(list, chunks[1], buf, &mut self.list_state);
    }
}

/// Help dialog showing keybindings.
pub struct HelpDialog;

impl HelpDialog {
    pub fn render(&self, area: Rect, buf: &mut Buffer) {
        Clear.render(area, buf);

        let dialog_area = centered_popup(area, 52, 16);
        let title = Paragraph::new(Line::from(Span::styled(
            " Keybindings ",
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )))
        .alignment(Alignment::Center);

        let keys = vec![
            ("Enter", "Submit"),
            ("Shift+Enter", "Newline"),
            ("Ctrl+C", "Cancel / Quit (×2)"),
            ("Ctrl+D", "Quit"),
            ("Ctrl+K/J", "Scroll up/down"),
            ("PgUp/PgDn", "Scroll page"),
            ("Home/End", "Scroll top/bottom"),
            ("/model [name]", "Switch model"),
            ("/provider", "Show provider"),
            ("/params", "Show parameters"),
            ("/clear", "Clear history"),
            ("/help", "Show this help"),
            ("/exit /quit", "Exit"),
        ];

        let mut lines = vec![Line::raw("")];
        for (key, desc) in &keys {
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {:14}", key),
                    Style::default()
                        .fg(Color::LightCyan)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(desc.to_string(), Style::default().fg(Color::DarkGray)),
            ]));
        }
        lines.push(Line::raw(""));

        let body = Paragraph::new(lines).block(
            Block::default()
                .borders(Borders::ALL)
                .border_style(Style::default().fg(Color::DarkGray)),
        );

        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(0)])
            .margin(1)
            .horizontal_margin(1)
            .split(dialog_area);

        title.render(chunks[0], buf);
        body.render(chunks[1], buf);
    }
}

/// A stack of dialogs. Only the topmost receives input and renders.
#[derive(Default)]
pub struct DialogStack {
    pub model: Option<ModelDialog>,
    pub help: Option<HelpDialog>,
}

impl DialogStack {
    pub fn is_open(&self) -> bool {
        self.model.is_some() || self.help.is_some()
    }

    pub fn open_model(&mut self, cfg: &ResolvedConfig) {
        self.model = Some(ModelDialog::new(cfg));
        self.help = None;
    }

    pub fn open_help(&mut self) {
        self.help = Some(HelpDialog);
        self.model = None;
    }

    pub fn close(&mut self) {
        self.model = None;
        self.help = None;
    }

    /// Extract the state for rendering (leaving empty).
    pub fn take(&mut self) -> Self {
        std::mem::take(self)
    }

    pub fn handle_key(&mut self, action: Action) -> Option<String> {
        if let Some(d) = self.model.as_mut() {
            return d.handle_key(action);
        }
        self.close();
        None
    }

    pub fn render(&mut self, area: Rect, buf: &mut Buffer) {
        if let Some(d) = self.model.as_mut() {
            d.render(area, buf);
        } else if let Some(d) = &self.help {
            d.render(area, buf);
        }
    }
}

fn centered_popup(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    }
}

// Re-export StatefulWidget for internal use
use ratatui::widgets::StatefulWidget;
