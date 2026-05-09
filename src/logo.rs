//! Aili logo: unicode block-art using special character markers.
//!
//! Markers (inspired by opencode's template system):
//!   `█`   full-block glyph (fg=brand, bg=transparent)
//!   `_`   shadow cell (space with bg=shadow — renders as a tinted block)
//!   `^`   half-block top-lit (▀ with fg=brand, bg=shadow)
//!   `~`   half-block shadow-only top (▀ with fg=shadow)
//!   `,`   half-block shadow-only bottom (▄ with fg=shadow)
//!
//! Two rendering paths:
//!   - `lines_for_scrollback()` → Vec<Line<'static> for terminal.insert_before()
//!   - `paragraph()` → ratatui Paragraph widget for fullscreen TUI

use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

pub struct LogoShape {
    pub left: &'static [&'static str],
    pub right: &'static [&'static str],
}

const AILI_LOGO: LogoShape = LogoShape {
    left: &[
        " ██████    ██  ██     ██",
        "██    ██       ██       ",
        "████████   ██  ██     ██",
        "██    ██   ██  ██     ██",
        "██    ██   ██  █████  ██",
    ],
    right: &["", "", "", "", ""],
};

const GAP: usize = 2;

pub fn logo_shape() -> &'static LogoShape {
    &AILI_LOGO
}

pub fn logo_width(shape: &LogoShape) -> usize {
    let left_w = shape.left.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    let right_w = shape.right.iter().map(|s| s.chars().count()).max().unwrap_or(0);
    left_w + GAP + right_w
}

pub fn logo_height(shape: &LogoShape) -> usize {
    shape.left.len().max(shape.right.len())
}

/// Produce styled lines suitable for `terminal.insert_before()`.
/// In the scrollback, the logo is rendered with the brand tint and shadow
/// blending — exactly like opencode's entry splash.
pub fn lines_for_scrollback(
    shape: &LogoShape,
    fg: Color,
    shadow: Color,
) -> Vec<Line<'static>> {
    let brand = Style::default().fg(fg);
    let rows = logo_height(shape);

    let mut out = Vec::with_capacity(rows);
    for i in 0..rows {
        let left_line = shape.left.get(i).copied().unwrap_or("");
        let right_line = shape.right.get(i).copied().unwrap_or("");
        let mut spans: Vec<Span<'static>> = Vec::new();

        // Left side
        for ch in left_line.chars() {
            match ch {
                '█' => spans.push(Span::styled("█", brand)),
                '_' => spans.push(Span::styled(" ", Style::default().bg(shadow))),
                '^' => spans.push(Span::styled("▀", Style::default().fg(fg).bg(shadow))),
                '~' => spans.push(Span::styled("▀", Style::default().fg(shadow))),
                ',' => spans.push(Span::styled("▄", Style::default().fg(shadow))),
                ' ' => spans.push(Span::raw(" ")),
                _ => spans.push(Span::styled(ch.to_string(), brand)),
            }
        }

        // Padding between left and right
        spans.push(Span::raw(" ".repeat(GAP)));

        // Right side
        for ch in right_line.chars() {
            match ch {
                '█' => spans.push(Span::styled("█", brand)),
                '_' => spans.push(Span::styled(" ", Style::default().bg(shadow))),
                '^' => spans.push(Span::styled("▀", Style::default().fg(fg).bg(shadow))),
                '~' => spans.push(Span::styled("▀", Style::default().fg(shadow))),
                ',' => spans.push(Span::styled("▄", Style::default().fg(shadow))),
                ' ' => spans.push(Span::raw(" ")),
                _ => spans.push(Span::styled(ch.to_string(), brand)),
            }
        }

        out.push(Line::from(spans));
    }
    out
}

/// Build a plain-text version of the logo (for debugging, no shadow markers).
pub fn plain_text(shape: &LogoShape) -> String {
    let rows = logo_height(shape);
    let mut s = String::new();
    for i in 0..rows {
        let left_line = shape.left.get(i).copied().unwrap_or("");
        let right_line = shape.right.get(i).copied().unwrap_or("");
        let mut row = left_line.to_string();
        row.push_str(&" ".repeat(GAP));
        row.push_str(right_line);
        row = row.replace('_', "█");
        row = row.replace('^', "▀");
        row = row.replace('~', "▀");
        row = row.replace(',', "▄");
        s.push_str(&row);
        if i + 1 < rows {
            s.push('\n');
        }
    }
    s
}

/// A ratatui Paragraph widget for the fullscreen TUI.
pub fn paragraph(shape: &LogoShape, fg: Color, shadow: Color) -> ratatui::widgets::Paragraph<'static> {
    let lines = lines_for_scrollback(shape, fg, shadow);
    ratatui::widgets::Paragraph::new(lines)
        .alignment(ratatui::layout::Alignment::Center)
        .style(Style::default())
}

/// Previous tiny logo placeholder (kept for compatibility during migration).
pub const WELCOME_LOGO: &str = "Aili";

pub fn lines() -> impl Iterator<Item = &'static str> {
    WELCOME_LOGO.lines()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn logo_has_rows() {
        let shape = logo_shape();
        assert!(shape.left.len() >= 3);
        assert!(shape.right.len() >= 3);
        assert_eq!(shape.left.len(), shape.right.len());
    }

    #[test]
    fn scrollback_lines_match_height() {
        let shape = logo_shape();
        let lines = lines_for_scrollback(shape, Color::Cyan, Color::DarkGray);
        assert_eq!(lines.len(), logo_height(shape));
    }

    #[test]
    fn plain_text_has_no_markers() {
        let shape = logo_shape();
        let text = plain_text(shape);
        assert!(!text.contains('_'));
        assert!(!text.contains('^'));
        assert!(!text.contains('~'));
        assert!(!text.contains(','));
        assert!(text.contains('█'));
    }

    #[test]
    fn logo_width_is_positive() {
        assert!(logo_width(logo_shape()) > 10);
    }
}
