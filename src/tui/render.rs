use ratatui::Frame;
use ratatui::buffer::Buffer;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Paragraph};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::{App, Role};
use crate::logo;
use crate::tui::scrollbox::{ScrollBox, ScrollBoxState};

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

pub fn draw_inline(app: &App, f: &mut Frame) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    let composer_lines = (app.composer.lines().len() as u16).clamp(1, 3);
    let spinner_h: u16 = if app.in_flight.is_some() { 1 } else { 0 };
    let info_h: u16 = 1;
    let div_h: u16 = 1;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(spinner_h),
            Constraint::Length(div_h),
            Constraint::Length(composer_lines),
            Constraint::Length(info_h),
        ])
        .split(area);

    f.render_widget(
        live_view(app, chunks[0].width as usize, chunks[0].height as usize),
        chunks[0],
    );
    if spinner_h > 0 {
        f.render_widget(spinner_line(app), chunks[1]);
    }
    f.render_widget(divider(), chunks[2]);
    let input_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(chunks[3]);
    f.render_widget(input_prompt(), input_chunks[0]);
    f.render_widget(&app.composer, input_chunks[1]);

    // Info bar: model · cwd · stats (or status flash)
    f.render_widget(info_or_status(app), chunks[4]);
}

/// Fullscreen session page: scrollable message history + prompt at bottom.
pub fn draw_fullscreen_session(app: &App, f: &mut Frame, state: &mut ScrollBoxState) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    let composer_h = (app.composer.lines().len() as u16).clamp(1, 3);
    let spinner_h: u16 = if app.in_flight.is_some() { 1 } else { 0 };
    let info_h: u16 = 1;
    let div_h: u16 = 1;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(0),
            Constraint::Length(spinner_h),
            Constraint::Length(div_h),
            Constraint::Length(composer_h),
            Constraint::Length(info_h),
        ])
        .split(area);

    // Message history with ScrollBox
    let msg_area = Rect {
        x: chunks[0].x + 2,
        y: chunks[0].y,
        width: chunks[0].width.saturating_sub(4),
        height: chunks[0].height,
    };
    let msg_lines = build_message_lines(app, msg_area.width as usize);
    let scrollbox = ScrollBox::new(msg_lines);
    f.render_stateful_widget(scrollbox, msg_area, state);

    // Spinner
    if spinner_h > 0 {
        f.render_widget(spinner_line(app), chunks[1]);
    }

    f.render_widget(divider(), chunks[2]);

    // Input
    let input_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(chunks[3]);
    f.render_widget(input_prompt(), input_chunks[0]);
    f.render_widget(&app.composer, input_chunks[1]);

    // Info bar
    f.render_widget(info_bar(app), chunks[4]);
}

/// Build message history lines with role spacing for ScrollBox.
fn build_message_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut last_role: Option<Role> = None;

    for msg in &app.history {
        if msg.text.is_empty() {
            continue;
        }
        // Spacer blank line between user/assistant transitions
        if let Some(prev) = last_role {
            if prev != msg.role {
                lines.push(Line::raw(""));
            }
        }
        last_role = Some(msg.role);

        match msg.role {
            Role::User => {
                lines.push(Line::from(Span::styled(
                    format!("{} >", app.cfg.persona.user_name),
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                )));
                for line in wrap_text_block(&msg.text, width) {
                    lines.push(Line::from(Span::styled(line, Style::default().fg(Color::White))));
                }
            }
            Role::Assistant => {
                lines.push(Line::from(Span::styled(
                    format!("{} >", app.cfg.persona.assistant_name),
                    Style::default()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::BOLD),
                )));
                for line in wrap_text_block(&msg.text, width) {
                    lines.push(Line::from(Span::styled(line, Style::default().fg(Color::White))));
                }
            }
        }
    }
    lines
}

/// Render message history as a scrollable Paragraph.
/// Messages are separated by a blank line between roles.
#[allow(dead_code)]
fn message_history(app: &App, width: usize, height: usize) -> Paragraph<'static> {
    let lines = build_message_lines(app, width);

    // Show only the last N lines that fit the viewport
    let visible = if lines.len() > height && height > 0 {
        lines.into_iter().rev().take(height).rev().collect()
    } else {
        lines
    };

    Paragraph::new(visible).alignment(Alignment::Left)
}
pub fn draw_fullscreen_home(app: &App, f: &mut Frame) {
    let area = f.area();
    f.render_widget(
        Block::default().style(Style::default().bg(Color::Reset)),
        area,
    );

    let shape = logo::logo_shape();
    let logo_h = logo::logo_height(shape) as u16;
    let composer_h = (app.composer.lines().len() as u16).clamp(1, 3);
    let spinner_h: u16 = if app.in_flight.is_some() { 1 } else { 0 };
    let info_h: u16 = 1;
    let gap: u16 = 1;

    let total_fixed = logo_h + gap + composer_h + info_h + spinner_h + gap;
    let top_space = area.height.saturating_sub(total_fixed) / 3;

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(top_space),
            Constraint::Length(logo_h),
            Constraint::Length(gap),
            Constraint::Length(composer_h),
            Constraint::Length(spinner_h),
            Constraint::Length(info_h),
            Constraint::Min(0),
        ])
        .split(area);

    // Logo
    let logo_w = crate::logo::logo_width(shape) as u16;
    let logo_area = centered_rect(chunks[1], logo_w, logo_h);
    f.render_widget(
        logo::paragraph(shape, Color::Rgb(255, 140, 155), Color::Rgb(255, 180, 190)),
        logo_area,
    );

    // Spinner
    if spinner_h > 0 {
        f.render_widget(
            spinner_line(app),
            centered_rect(chunks[4], 60, spinner_h),
        );
    }

    // Composer (centered, max 75 wide)
    let composer_area = centered_rect(chunks[3], 75.min(area.width), composer_h);
    let input_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(2), Constraint::Min(0)])
        .split(composer_area);
    f.render_widget(input_prompt(), input_chunks[0]);
    f.render_widget(&app.composer, input_chunks[1]);

    // Info bar (centered)
    f.render_widget(
        info_bar(app),
        centered_rect(chunks[5], 75.min(area.width), info_h),
    );

    // Status message overlay (if any)
    if app.status_msg.is_some() {
        let status_area = Rect {
            y: area.y,
            height: 1,
            ..centered_rect(area, 80.min(area.width), 1)
        };
        f.render_widget(
            info_or_status(app),
            status_area,
        );
    }
}

fn centered_rect(area: Rect, width: u16, height: u16) -> Rect {
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect { x, y, width: width.min(area.width), height: height.min(area.height) }
}

fn info_bar(app: &App) -> Paragraph<'static> {
    let mut parts: Vec<Span<'static>> = Vec::new();
    let dim = Style::default().fg(Color::Gray);
    let highlight = Style::default().fg(Color::Gray);

    let model_name = crate::provider::model_info(&app.cfg.model).name;
    parts.push(Span::styled(model_name.to_string(), highlight));
    parts.push(Span::styled("  ·  ", dim));
    parts.push(Span::styled(pretty_cwd(), dim));

    // Always show context usage progress if we have data from any turn
    let ctx_tokens = if let Some(f) = app.in_flight.as_ref() {
        f.context_tokens.max(app.last_context_tokens)
    } else {
        app.last_context_tokens
    };
    let ctx_limit = if let Some(f) = app.in_flight.as_ref() {
        f.context_limit.max(app.last_context_limit)
    } else {
        app.last_context_limit
    };

    if ctx_limit > 0 && ctx_tokens > 0 {
        let pct = (ctx_tokens * 100 / ctx_limit).min(99);
        let filled = ((pct as f64 / 100.0) * 10.0).round() as usize;
        let bar: String = (0..10)
            .map(|i| if i < filled { '█' } else { '░' })
            .collect();
        parts.push(Span::styled(
            format!(
                "  ·  {}  {} / 1M tokens",
                bar,
                if ctx_tokens >= 1000 {
                    format!("{:.1}K", ctx_tokens as f64 / 1000.0)
                } else {
                    ctx_tokens.to_string()
                },
            ),
            dim,
        ));

        // Show elapsed time only while streaming
        if let Some(f) = app.in_flight.as_ref() {
            let elapsed = f.started.elapsed();
            parts.push(Span::styled(
                format!("  ·  {:.1}s", elapsed.as_secs_f64()),
                dim,
            ));
        }
    }

    Paragraph::new(Line::from(parts))
        .alignment(Alignment::Center)
}

fn spinner_line(app: &App) -> Paragraph<'static> {
    let Some(f) = app.in_flight.as_ref() else {
        return Paragraph::new(Line::raw(""));
    };
    const FRAMES: [&str; 6] = ["✶", "✸", "✹", "✺", "✹", "✸"];
    let elapsed = f.started.elapsed();
    let frame_idx = (elapsed.as_millis() / 80) as usize % FRAMES.len();
    let secs = elapsed.as_secs();
    let mins = secs / 60;
    let time_str = if mins > 0 {
        format!("{}m {:02}s", mins, secs % 60)
    } else {
        format!("{}s", secs)
    };
    let kilo = f.chars as f64 / 4.0 / 1000.0;
    let brand = Color::LightCyan;
    Paragraph::new(Line::from(vec![
        Span::styled(
            format!("{} ", FRAMES[frame_idx]),
            Style::default().fg(brand),
        ),
        Span::styled("Calculating…", Style::default().fg(brand)),
        Span::styled(
            format!(" ({} · ↓ {:.1}k tokens)", time_str, kilo),
            Style::default().fg(Color::Gray),
        ),
    ]))
}

fn live_view(app: &App, width: usize, height: usize) -> Paragraph<'static> {
    let lines = if app.in_flight.is_some() {
        app.history
            .last()
            .filter(|m| m.role == Role::Assistant && !m.text.is_empty())
            .map(|m| assistant_message_lines(&app.cfg.persona.assistant_name, &m.text, width))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    Paragraph::new(trim_to_visible(lines, height)).alignment(Alignment::Left)
}

/// Full welcome page: block-art logo on the left, centered vertically next to
/// the text content, all wrapped in a single border.
#[allow(dead_code)]
pub fn welcome_page_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let logo_lines: Vec<&'static str> = logo::lines().collect();
    let logo_w = logo_lines
        .iter()
        .map(|s| UnicodeWidthStr::width(*s))
        .max()
        .unwrap_or(0);
    let gap = 3usize;
    let inner_width = width.saturating_sub(4);
    let min_text_width = 18usize;

    let text_col_width = if inner_width >= logo_w + gap + min_text_width {
        inner_width.saturating_sub(logo_w + gap).min(72)
    } else {
        return welcome_card_lines(app, width);
    };

    let text_content = welcome_card_content(app, text_col_width);
    let max_rows = text_content.len().max(logo_lines.len());
    let logo_start = centered_logo_start(&text_content, logo_lines.len(), max_rows);
    let text_lines: Vec<Line<'static>> = text_content
        .iter()
        .map(|line| center_line_to_width(line, text_col_width))
        .collect();
    let mut combined: Vec<Line<'static>> = Vec::with_capacity(max_rows);
    let gap_span = Span::raw(" ".repeat(gap));
    let logo_style = Style::default().fg(Color::DarkGray);

    for i in 0..max_rows {
        let mut spans: Vec<Span<'static>> = Vec::new();

        if i >= logo_start && i < logo_start + logo_lines.len() {
            let s = logo_lines[i - logo_start];
            let used = UnicodeWidthStr::width(s);
            let left_pad = logo_w.saturating_sub(used) / 2;
            let right_pad = logo_w.saturating_sub(used + left_pad);
            if left_pad > 0 {
                spans.push(Span::raw(" ".repeat(left_pad)));
            }
            spans.push(Span::styled(s.to_string(), logo_style));
            if right_pad > 0 {
                spans.push(Span::raw(" ".repeat(right_pad)));
            }
        } else {
            spans.push(Span::raw(" ".repeat(logo_w)));
        }

        spans.push(gap_span.clone());

        if i < text_lines.len() {
            let line = &text_lines[i];
            let used = line_width(line);
            spans.extend(line.spans.iter().cloned());
            if used < text_col_width {
                spans.push(Span::raw(" ".repeat(text_col_width - used)));
            }
        } else {
            spans.push(Span::raw(" ".repeat(text_col_width)));
        }

        combined.push(Line::from(spans));
    }

    with_border_fit(combined, width)
}

fn welcome_card_content(app: &App, width: usize) -> Vec<Line<'static>> {
    let content_width = width.max(1).min(72);
    let mut lines = vec![
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
    ];
    for line in wrap_text_block(
        &format!("Welcome back, {}!", app.cfg.persona.user_name),
        content_width,
    ) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::White),
        )));
    }
    lines.push(Line::raw(""));
    for line in wrap_text_block(
        &app.cfg.model,
        content_width,
    ) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default()
                .fg(Color::LightCyan)
                .add_modifier(Modifier::BOLD),
        )));
    }
    for line in wrap_text_block(&pretty_cwd(), content_width) {
        lines.push(Line::from(Span::styled(
            line,
            Style::default().fg(Color::DarkGray),
        )));
    }
    lines.push(Line::raw(""));
    lines
}

fn center_line_to_width(line: &Line<'_>, width: usize) -> Line<'static> {
    let used = line_width(line);
    if used >= width {
        return owned_line(line);
    }

    let left_pad = (width - used) / 2;
    let right_pad = width - used - left_pad;
    let mut spans = Vec::with_capacity(line.spans.len() + 2);
    if left_pad > 0 {
        spans.push(Span::raw(" ".repeat(left_pad)));
    }
    spans.extend(owned_line(line).spans);
    if right_pad > 0 {
        spans.push(Span::raw(" ".repeat(right_pad)));
    }
    Line::from(spans)
}

fn owned_line(line: &Line<'_>) -> Line<'static> {
    Line {
        style: line.style,
        alignment: line.alignment,
        spans: line
            .spans
            .iter()
            .map(|span| Span::styled(span.content.to_string(), span.style))
            .collect(),
    }
}

fn centered_logo_start(
    text_lines: &[Line<'_>],
    logo_line_count: usize,
    total_rows: usize,
) -> usize {
    if logo_line_count == 0 || total_rows == 0 {
        return 0;
    }

    let visible_rows: Vec<usize> = text_lines
        .iter()
        .enumerate()
        .filter_map(|(idx, line)| (line_width(line) > 0).then_some(idx))
        .collect();

    let center_row = match (visible_rows.first(), visible_rows.last()) {
        (Some(first), Some(last)) => (first + last) / 2,
        _ => total_rows / 2,
    };

    center_row
        .saturating_sub(logo_line_count / 2)
        .min(total_rows.saturating_sub(logo_line_count))
}

pub fn welcome_card_lines(app: &App, width: usize) -> Vec<Line<'static>> {
    let width = width.max(1);
    let content_width = width.saturating_sub(4).max(1);
    with_border_fit(welcome_card_content(app, content_width), width)
}

/// "{user_name}\n<wrapped text>\n" formatted for terminal scrollback.
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

pub fn assistant_message_lines(
    assistant_name: &str,
    text: &str,
    width: usize,
) -> Vec<Line<'static>> {
    let mut out = vec![Line::from(Span::styled(
        assistant_name.to_string(),
        Style::default()
            .fg(Color::Cyan)
            .add_modifier(Modifier::BOLD),
    ))];
    for v in wrap_text_block(text, width) {
        out.push(Line::raw(v));
    }
    out.push(Line::raw(""));
    out
}

pub fn note_lines(note: &str) -> Vec<Line<'static>> {
    vec![
        Line::from(vec![
            Span::styled("· ", Style::default().fg(Color::DarkGray)),
            Span::styled(note.to_string(), Style::default().fg(Color::DarkGray)),
        ]),
        Line::raw(""),
    ]
}

#[cfg(test)]
fn with_border(lines: Vec<Line<'static>>) -> Vec<Line<'static>> {
    let content_width = lines.iter().map(line_width).max().unwrap_or(0);
    let border_inner_width = content_width + 2;
    let mut out = Vec::with_capacity(lines.len() + 2);
    out.push(Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(border_inner_width)),
        Style::default().fg(Color::DarkGray),
    )));

    for line in lines {
        let used_width = line_width(&line);
        let mut spans = Vec::with_capacity(line.spans.len() + 3);
        spans.push(Span::styled("│ ", Style::default().fg(Color::DarkGray)));
        spans.extend(line.spans);
        if used_width < content_width {
            spans.push(Span::styled(
                " ".repeat(content_width - used_width),
                Style::default().fg(Color::DarkGray),
            ));
        }
        spans.push(Span::styled(" │", Style::default().fg(Color::DarkGray)));
        out.push(Line::from(spans));
    }

    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(border_inner_width)),
        Style::default().fg(Color::DarkGray),
    )));
    out
}

fn with_border_fit(lines: Vec<Line<'static>>, width: usize) -> Vec<Line<'static>> {
    if width < 4 {
        return lines
            .iter()
            .map(|line| fit_line_to_width(line, width))
            .collect();
    }

    let max_content_width = width - 4;
    let content_width = lines
        .iter()
        .map(line_width)
        .max()
        .unwrap_or(0)
        .min(max_content_width);
    let border_inner_width = content_width + 2;
    let border_style = Style::default().fg(Color::DarkGray);
    let mut out = Vec::with_capacity(lines.len() + 2);
    out.push(Line::from(Span::styled(
        format!("╭{}╮", "─".repeat(border_inner_width)),
        border_style,
    )));

    for line in lines {
        let fitted = fit_line_to_width(&line, content_width);
        let used_width = line_width(&fitted);
        let mut spans = Vec::with_capacity(fitted.spans.len() + 3);
        spans.push(Span::styled("│ ", border_style));
        spans.extend(fitted.spans);
        if used_width < content_width {
            spans.push(Span::styled(
                " ".repeat(content_width - used_width),
                border_style,
            ));
        }
        spans.push(Span::styled(" │", border_style));
        out.push(Line::from(spans));
    }

    out.push(Line::from(Span::styled(
        format!("╰{}╯", "─".repeat(border_inner_width)),
        border_style,
    )));
    out
}

fn line_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|span| UnicodeWidthStr::width(span.content.as_ref()))
        .sum()
}

fn fit_line_to_width(line: &Line<'_>, width: usize) -> Line<'static> {
    if width == 0 {
        return Line::raw("");
    }
    let mut used = 0usize;
    let mut spans = Vec::new();
    for span in &line.spans {
        let mut content = String::new();
        for ch in span.content.chars() {
            if ch.is_control() {
                continue;
            }
            let ch_width = char_width(ch);
            if ch_width == 0 {
                continue;
            }
            if used + ch_width > width {
                if !content.is_empty() {
                    spans.push(Span::styled(content, span.style));
                }
                return Line::from(spans);
            }
            content.push(ch);
            used += ch_width;
        }
        if !content.is_empty() {
            spans.push(Span::styled(content, span.style));
        }
    }
    Line::from(spans)
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
        Style::default().fg(Color::Gray),
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

fn info_or_status(app: &App) -> Paragraph<'static> {
    if let Some((msg, _)) = &app.status_msg {
        return Paragraph::new(Line::from(Span::styled(
            msg.clone(),
            Style::default().fg(Color::Yellow),
        )))
        .alignment(Alignment::Left);
    }
    info_bar(app)
}

#[allow(dead_code)]
fn fit_text_to_width(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in text.chars() {
        if ch.is_control() {
            continue;
        }
        let ch_width = char_width(ch);
        if ch_width == 0 {
            continue;
        }
        if used + ch_width > width {
            break;
        }
        out.push(ch);
        used += ch_width;
    }
    out
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

/// Paint a `Vec<Line>` into an inline history buffer without emitting printable
/// spaces for wide-character continuation cells.
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
    use crate::config::{AltScreenMode, Persona, ResolvedConfig, TuiConfig};
    use ratatui::layout::Rect;

    fn test_app() -> App {
        App::new(
            ResolvedConfig {
                base_url: "https://example.test/v1".into(),
                api_key: "test-key".into(),
                model: "deepseek-v4-flash".into(),
                temperature: None,
                top_p: None,
                max_tokens: None,
                stop: Vec::new(),
                persona: Persona {
                    user_name: "Rose".into(),
                    assistant_name: "Aili".into(),
                    description: String::new(),
                },
                tui: TuiConfig {
                    alternate_screen: AltScreenMode::Never,
                },
            },
            reqwest::Client::new(),
        )
    }

    #[allow(dead_code)]
    fn line_text(line: &Line<'_>) -> String {
        line.spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect::<String>()
    }

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
    fn welcome_card_has_border() {
        let lines = with_border(vec![Line::raw("Aili")]);
        assert_eq!(lines[0].spans[0].content.as_ref(), "╭──────╮");
        assert_eq!(lines[2].spans[0].content.as_ref(), "╰──────╯");
    }

    #[test]
    fn info_bar_constructed_without_panic() {
        let app = test_app();
        let _p = info_bar(&app);
    }

    #[test]
    fn with_border_wraps_combined_text_and_logo_block() {
        // Simulate the post-merge content: a few "text" lines + same-row logo spans.
        let mut combined: Vec<Line<'static>> = Vec::new();
        for _ in 0..6 {
            combined.push(Line::from(vec![
                Span::raw("hello world         "),
                Span::raw("   "),
                Span::raw("▀▀▄▀█▀▄▀"),
            ]));
        }
        let bordered = with_border(combined);
        assert!(bordered[0].spans[0].content.starts_with("╭"));
        assert!(
            bordered[bordered.len() - 1].spans[0]
                .content
                .starts_with("╰")
        );
        for line in &bordered[1..bordered.len() - 1] {
            assert_eq!(line.spans[0].content.as_ref(), "│ ");
            assert_eq!(line.spans.last().unwrap().content.as_ref(), " │");
        }
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
