use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Widget};
use textwrap::Options;

use super::App;

fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let opts = Options::new(width.max(1)).break_words(true);
    textwrap::wrap(text, &opts)
        .into_iter()
        .map(|s| s.into_owned())
        .collect()
}

fn wrap_text_block(text: &str, width: usize) -> Vec<String> {
    let mut out = Vec::new();
    for paragraph in text.split('\n') {
        for v in wrap_paragraph(paragraph, width) {
            out.push(v);
        }
    }
    out
}

// ────────────────────────── inline viewport (bottom UI) ──────────────────────────

pub fn draw_inline(app: &App, f: &mut Frame) {
    let area = f.area();
    let composer_lines = (app.composer.lines().len() as u16).clamp(1, 2);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),               // status
            Constraint::Length(1),               // divider
            Constraint::Length(composer_lines),  // composer (1-2 rows)
            Constraint::Length(1),               // hint
            Constraint::Min(0),                  // filler
        ])
        .split(area);

    f.render_widget(render_status(app), chunks[0]);
    f.render_widget(render_divider(), chunks[1]);
    f.render_widget(&app.composer, chunks[2]);
    f.render_widget(render_hint(app), chunks[3]);
}

fn render_status(app: &App) -> Paragraph<'static> {
    let mut spans = vec![
        Span::styled(
            "Aili",
            Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled(
            app.cfg.provider.as_str().to_string(),
            Style::default().fg(Color::DarkGray),
        ),
        Span::styled(" · ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            app.cfg.model.clone(),
            Style::default().fg(Color::DarkGray),
        ),
    ];
    if app.in_flight.is_some() {
        spans.push(Span::raw("    "));
        spans.push(Span::styled(
            "● streaming",
            Style::default().fg(Color::Yellow),
        ));
    }
    Paragraph::new(Line::from(spans)).alignment(Alignment::Left)
}

fn render_divider() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "─".repeat(2048),
        Style::default().fg(Color::DarkGray),
    )))
}

fn render_hint(app: &App) -> Paragraph<'static> {
    if let Some((msg, _)) = &app.status_msg {
        return Paragraph::new(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Yellow),
        )));
    }
    let s = if app.in_flight.is_some() {
        "Ctrl-C cancel  ·  Ctrl-D quit"
    } else {
        "Enter send  ·  Shift+Enter newline  ·  /help  ·  Ctrl-D quit"
    };
    Paragraph::new(Line::from(Span::styled(
        s.to_string(),
        Style::default().fg(Color::DarkGray),
    )))
}

// ────────────────────── scrollback writers (insert_before) ──────────────────────

/// Welcome card shown once at startup. Returns the buffer painter and the
/// number of rows it needs (so the caller can size `insert_before`).
pub fn welcome_card_lines(model: &str, version: &str) -> Vec<Line<'static>> {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .map(|s| {
            if let Some(home) = dirs::home_dir().and_then(|h| h.into_os_string().into_string().ok())
            {
                if s == home {
                    "~".to_string()
                } else if let Some(rest) = s.strip_prefix(&format!("{home}/")) {
                    format!("~/{rest}")
                } else {
                    s
                }
            } else {
                s
            }
        })
        .unwrap_or_else(|| "?".into());

    vec![
        Line::from(vec![
            Span::styled(">_ ", Style::default().fg(Color::DarkGray)),
            Span::styled("Aili", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
            Span::styled(format!(" (v{version})"), Style::default().fg(Color::DarkGray)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("model:     ", Style::default().fg(Color::DarkGray)),
            Span::styled(model.to_string(), Style::default().fg(Color::White)),
            Span::raw("    "),
            Span::styled("/model", Style::default().fg(Color::Cyan)),
            Span::styled(" to change", Style::default().fg(Color::DarkGray)),
        ]),
        Line::from(vec![
            Span::styled("directory: ", Style::default().fg(Color::DarkGray)),
            Span::raw(cwd),
        ]),
        Line::raw(""),
        Line::from(Span::styled(
            "·  type a message to start.  scroll with the terminal — trackpad, wheel, Cmd+↑/↓.",
            Style::default().fg(Color::DarkGray),
        )),
        Line::raw(""),
    ]
}

/// "you\n<wrapped text>\n" formatted for scrollback insertion.
pub fn user_message_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        "you",
        Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
    ))];
    for v in wrap_text_block(text, width) {
        out.push(Line::raw(v));
    }
    out.push(Line::raw(""));
    out
}

/// "Aili" header (no body — body lines are inserted as tokens stream).
pub fn assistant_header_lines() -> Vec<Line<'static>> {
    vec![Line::from(Span::styled(
        "Aili",
        Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD),
    ))]
}

/// Render a chunk of assistant body text (already-completed lines) for
/// scrollback insertion. Wraps to `width`.
pub fn assistant_body_lines(text: &str, width: usize) -> Vec<Line<'static>> {
    wrap_text_block(text, width)
        .into_iter()
        .map(Line::raw)
        .collect()
}

/// A trailing blank line + an inline `· {note}` tag to mark cancel/error.
pub fn note_lines(note: &str) -> Vec<Line<'static>> {
    vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled("·  ", Style::default().fg(Color::DarkGray)),
            Span::styled(note.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::raw(""),
    ]
}

/// One-line separator between turns.
pub fn turn_separator() -> Vec<Line<'static>> {
    vec![Line::raw("")]
}

/// Paint a `Vec<Line>` into a scrollback buffer (called from `insert_before`).
pub fn paint_lines(buf: &mut Buffer, lines: Vec<Line<'static>>) {
    let area = buf.area;
    Paragraph::new(lines).render(area, buf);
}

#[allow(dead_code)]
pub fn welcome_card_with_box(model: &str, version: &str) -> (Vec<Line<'static>>, Block<'static>) {
    (
        welcome_card_lines(model, version),
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(Style::default().fg(Color::DarkGray)),
    )
}
