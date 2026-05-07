use ratatui::Frame;
#[cfg(test)]
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::UnicodeWidthChar;

use super::{App, Role};

fn wrap_paragraph(text: &str, width: usize) -> Vec<String> {
    if text.is_empty() {
        return vec![String::new()];
    }
    let width = width.max(1);
    let mut out = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;

    for ch in text.chars() {
        let ch_width = char_width(ch);
        if line_width > 0 && line_width + ch_width > width {
            if starts_with_leading_punctuation(ch) {
                if let Some((moved, moved_width)) =
                    pop_last_char_from_line(&mut line, &mut line_width)
                {
                    out.push(std::mem::take(&mut line));
                    line.push(moved);
                    line_width = moved_width;
                } else {
                    out.push(std::mem::take(&mut line));
                    line_width = 0;
                }
            } else {
                out.push(std::mem::take(&mut line));
                line_width = 0;
            }
        }
        line.push(ch);
        line_width += ch_width;
    }

    if !line.is_empty() {
        out.push(line);
    }
    out
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

pub fn draw_fullscreen(app: &App, f: &mut Frame) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    let composer_lines = (app.composer.lines().len() as u16).clamp(1, 3);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(1),
            Constraint::Length(composer_lines),
            Constraint::Length(1),
        ])
        .split(area);

    f.render_widget(
        history_view(app, chunks[0].width as usize, chunks[0].height as usize),
        chunks[0],
    );
    f.render_widget(divider(), chunks[1]);

    let input_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(chunks[2]);
    f.render_widget(input_prompt(), input_chunks[0]);
    f.render_widget(&app.composer, input_chunks[1]);
    f.render_widget(status_line(app), chunks[3]);
}

fn history_view(app: &App, width: usize, height: usize) -> Paragraph<'static> {
    let mut lines = if app.history.is_empty() {
        welcome_lines(app)
    } else {
        conversation_lines(app, width)
    };
    lines = trim_to_visible(lines, height);
    Paragraph::new(lines).alignment(Alignment::Left)
}

fn welcome_lines(app: &App) -> Vec<Line<'static>> {
    let cwd = pretty_cwd();
    vec![
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                "Aili",
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" v{}", env!("CARGO_PKG_VERSION")),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled("Welcome back, ", Style::default().fg(Color::White)),
            Span::styled(
                app.cfg.persona.user_name.clone(),
                Style::default()
                    .fg(Color::White)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled("!", Style::default().fg(Color::White)),
        ]),
        Line::raw(""),
        Line::from(vec![
            Span::styled(
                app.cfg.model.clone(),
                Style::default()
                    .fg(Color::LightCyan)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" · ", Style::default().fg(Color::DarkGray)),
            Span::styled(
                app.cfg.provider.as_str().to_string(),
                Style::default().fg(Color::DarkGray),
            ),
        ]),
        Line::from(Span::styled(cwd, Style::default().fg(Color::DarkGray))),
    ]
}

fn conversation_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut out = Vec::new();
    for msg in &app.history {
        match msg.role {
            Role::User => {
                out.push(Line::from(Span::styled(
                    app.cfg.persona.user_name.clone(),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
            }
            Role::Assistant => {
                out.push(Line::from(Span::styled(
                    "Aili",
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
            }
        }
        for v in wrap_text_block(&msg.text, width) {
            out.push(Line::raw(v));
        }
        out.push(Line::raw(""));
    }
    out
}

fn trim_to_visible(lines: Vec<Line<'static>>, max_len: usize) -> Vec<Line<'static>> {
    if lines.len() <= max_len {
        return lines;
    }
    let skip = lines.len() - max_len;
    lines.into_iter().skip(skip).collect()
}

fn divider() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        "─".repeat(2048),
        Style::default().fg(Color::DarkGray),
    )))
}

fn input_prompt() -> Paragraph<'static> {
    Paragraph::new(Line::from(Span::styled(
        ">",
        Style::default()
            .fg(Color::LightCyan)
            .add_modifier(Modifier::BOLD),
    )))
}

fn status_line(app: &App) -> Paragraph<'static> {
    if let Some((msg, _)) = &app.status_msg {
        return Paragraph::new(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Yellow),
        )));
    }
    Paragraph::new(Line::raw(""))
}

fn pretty_cwd() -> String {
    let cwd = std::env::current_dir()
        .ok()
        .and_then(|p| p.into_os_string().into_string().ok())
        .unwrap_or_else(|| "?".into());
    if let Some(home) = dirs::home_dir().and_then(|h| h.into_os_string().into_string().ok()) {
        if cwd == home {
            return "~".into();
        }
        if let Some(rest) = cwd.strip_prefix(&format!("{home}/")) {
            return format!("~/{rest}");
        }
    }
    cwd
}

/// "{user_name}\n<wrapped text>\n" formatted for tests and legacy callers.
#[cfg(test)]
pub fn user_message_lines(user_name: &str, text: &str, width: usize) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        user_name.to_string(),
        Style::default()
            .fg(Color::Green)
            .add_modifier(Modifier::BOLD),
    ))];
    for v in wrap_text_block(text, width) {
        out.push(Line::raw(v));
    }
    out.push(Line::raw(""));
    out
}

/// Paint a `Vec<Line>` into a buffer without emitting printable spaces for
/// wide-character continuation cells.
#[cfg(test)]
pub fn paint_lines(buf: &mut Buffer, lines: Vec<Line<'static>>) {
    let area = buf.area;
    for (row, line) in lines.iter().enumerate() {
        if row >= area.height as usize {
            break;
        }
        paint_line(buf, area.x, area.y + row as u16, line);
    }
}

fn char_width(ch: char) -> usize {
    match ch {
        '\t' => 4,
        _ => UnicodeWidthChar::width(ch).unwrap_or(0),
    }
}

#[cfg(test)]
fn paint_line(buf: &mut Buffer, mut x: u16, y: u16, line: &Line<'_>) {
    let right = buf.area.right();
    for span in line {
        let style = span.style;
        for ch in span.content.chars() {
            if ch.is_control() {
                continue;
            }
            if ch == '\t' {
                for _ in 0..4 {
                    if x >= right {
                        return;
                    }
                    buf[(x, y)].set_symbol(" ").set_style(style);
                    x += 1;
                }
                continue;
            }
            let width = char_width(ch);
            if width == 0 {
                continue;
            }
            if x.saturating_add(width as u16) > right {
                return;
            }

            let mut encoded = [0; 4];
            buf[(x, y)]
                .set_symbol(ch.encode_utf8(&mut encoded))
                .set_style(style);
            for offset in 1..width {
                buf[(x + offset as u16, y)].set_symbol("").set_style(style);
            }
            x += width as u16;
        }
    }
}

fn starts_with_leading_punctuation(ch: char) -> bool {
    matches!(
        ch,
        '，' | '。'
            | '、'
            | '？'
            | '！'
            | '：'
            | '；'
            | ')'
            | ']'
            | '}'
            | '）'
            | '】'
            | '》'
            | '」'
            | '』'
    )
}

fn pop_last_char_from_line(line: &mut String, line_width: &mut usize) -> Option<(char, usize)> {
    let Some((idx, ch)) = line.char_indices().last() else {
        return None;
    };
    if idx == 0 {
        return None;
    }
    line.truncate(idx);
    let width = char_width(ch);
    *line_width = line_width.saturating_sub(width);
    Some((ch, width))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::layout::Rect;

    #[test]
    fn wraps_cjk_without_inserting_spaces() {
        let lines = wrap_text_block("你好，你是什么模型？", 8);
        assert_eq!(lines.concat(), "你好，你是什么模型？");
        assert!(lines.iter().all(|line| !line.contains(' ')));
    }

    #[test]
    fn wraps_long_text_by_width() {
        let lines = wrap_text_block("abcdef", 3);
        assert_eq!(lines, vec!["abc".to_string(), "def".to_string()]);
    }

    #[test]
    fn user_header_uses_configured_name() {
        let lines = user_message_lines("Rose", "hi", 80);
        assert_eq!(lines[0].spans[0].content.as_ref(), "Rose");
    }

    #[test]
    fn paint_lines_does_not_emit_space_after_wide_chars() {
        let mut buf = Buffer::empty(Rect::new(0, 0, 12, 1));
        paint_lines(&mut buf, vec![Line::raw("你好")]);
        assert_eq!(buf[(0, 0)].symbol(), "你");
        assert_eq!(buf[(1, 0)].symbol(), "");
        assert_eq!(buf[(2, 0)].symbol(), "好");
        assert_eq!(buf[(3, 0)].symbol(), "");
    }

    #[test]
    fn wrapping_keeps_closing_punctuation_with_previous_char() {
        let lines = wrap_text_block("你的吗？", 7);
        assert_eq!(lines, vec!["你的".to_string(), "吗？".to_string()]);
    }
}
