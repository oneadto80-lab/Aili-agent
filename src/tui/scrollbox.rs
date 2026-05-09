//! Virtual scrollbox widget for fullscreen TUI message history.
//!
//! Holds a flat list of ratatui `Line`s and renders a visible window
//! driven by `ScrollBoxState`. Supports keyboard scrolling, sticky-bottom
//! auto-follow, and an optional scrollbar.

use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{StatefulWidget, Widget};
use unicode_width::UnicodeWidthChar;

/// The mutable state of a ScrollBox, tracking which portion is visible.
#[derive(Debug, Clone)]
pub struct ScrollBoxState {
    /// Index of the top-most visible line (0 = top of content).
    pub viewport_top: usize,
    /// When true, new lines appended automatically scroll to the bottom.
    pub sticky_bottom: bool,
}

impl ScrollBoxState {
    pub fn new() -> Self {
        Self {
            viewport_top: 0,
            sticky_bottom: true,
        }
    }

    pub fn scroll_up(&mut self, lines: usize) {
        self.viewport_top = self.viewport_top.saturating_sub(lines);
        self.sticky_bottom = false;
    }

    pub fn scroll_down(&mut self, lines: usize, total_lines: usize, visible_height: usize) {
        let max_top = total_lines.saturating_sub(visible_height);
        self.viewport_top = (self.viewport_top + lines).min(max_top);
        if self.viewport_top >= max_top {
            self.sticky_bottom = true;
        }
    }

    pub fn scroll_to_top(&mut self) {
        self.viewport_top = 0;
        self.sticky_bottom = false;
    }

    pub fn scroll_to_bottom(&mut self, total_lines: usize, visible_height: usize) {
        self.viewport_top = total_lines.saturating_sub(visible_height);
        self.sticky_bottom = true;
    }
}

/// A scrollable list of styled lines.
pub struct ScrollBox<'a> {
    lines: Vec<Line<'a>>,
    scrollbar: bool,
}

impl<'a> ScrollBox<'a> {
    pub fn new(lines: Vec<Line<'a>>) -> Self {
        Self {
            lines,
            scrollbar: false,
        }
    }

    pub fn scrollbar(mut self, visible: bool) -> Self {
        self.scrollbar = visible;
        self
    }

    pub fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }
}

impl StatefulWidget for ScrollBox<'_> {
    type State = ScrollBoxState;

    fn render(self, area: Rect, buf: &mut Buffer, state: &mut Self::State) {
        let total = self.lines.len();
        let visible_h = area.height as usize;

        // Sticky-bottom: always start from the end
        if state.sticky_bottom && total > visible_h {
            state.viewport_top = total - visible_h;
        }

        let max_top = total.saturating_sub(visible_h);
        if state.viewport_top > max_top {
            state.viewport_top = max_top;
        }

        // Render visible lines
        let end = (state.viewport_top + visible_h).min(total);
        for (i, line_idx) in (state.viewport_top..end).enumerate() {
            let line = &self.lines[line_idx];
            render_line(line, area.x, area.y + i as u16, area.width, buf);
        }

        // Scrollbar
        if self.scrollbar && total > visible_h {
            render_scrollbar(area, total, visible_h, state.viewport_top, buf);
        }
    }
}

impl Widget for ScrollBox<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        StatefulWidget::render(self, area, buf, &mut ScrollBoxState::new());
    }
}

fn render_line(line: &Line<'_>, x: u16, y: u16, max_width: u16, buf: &mut Buffer) {
    let right = x + max_width;
    let mut cx = x;

    for span in line {
        let style = span.style;
        for ch in span.content.chars() {
            if ch.is_control() {
                continue;
            }
            let width = UnicodeWidthChar::width(ch).unwrap_or(0);
            if width == 0 {
                continue;
            }
            if cx.saturating_add(width as u16) > right {
                return;
            }
            let mut encoded = [0; 4];
            buf[(cx, y)]
                .set_symbol(ch.encode_utf8(&mut encoded))
                .set_style(style);
            for offset in 1..width {
                buf[(cx + offset as u16, y)].set_symbol("").set_style(style);
            }
            cx += width as u16;
        }
    }
}

fn render_scrollbar(area: Rect, total: usize, visible: usize, offset: usize, buf: &mut Buffer) {
    let x = area.right().saturating_sub(1);
    if x < area.x || area.height < 3 {
        return;
    }

    let track_h = (area.height as usize).saturating_sub(2);
    let thumb_h = ((visible as f64 / total as f64) * track_h as f64).ceil() as usize;
    let thumb_h = thumb_h.max(1).min(track_h);

    let max_offset = total - visible;
    let thumb_pos = if max_offset == 0 {
        0
    } else {
        ((offset as f64 / max_offset as f64) * (track_h - thumb_h) as f64) as usize
    };

    let dim = Style::default().fg(Color::DarkGray);

    // Up arrow
    buf[(x, area.y)].set_symbol("▲").set_style(dim);
    // Down arrow
    buf[(x, area.y + area.height - 1)].set_symbol("▼").set_style(dim);
    // Track and thumb
    for i in 0..track_h {
        let y = area.y + 1 + i as u16;
        if i >= thumb_pos && i < thumb_pos + thumb_h {
            buf[(x, y)].set_symbol("█").set_style(dim);
        } else {
            buf[(x, y)].set_symbol("│").set_style(Style::default().fg(Color::Rgb(60, 60, 60)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_up_moves_viewport() {
        let mut state = ScrollBoxState::new();
        state.viewport_top = 10;
        state.scroll_up(3);
        assert_eq!(state.viewport_top, 7);
        assert!(!state.sticky_bottom);
    }

    #[test]
    fn scroll_down_at_bottom_restores_sticky() {
        let mut state = ScrollBoxState::new();
        state.sticky_bottom = false;
        // Scroll all the way down until max_top
        state.scroll_down(100, 15, 10);
        assert!(state.sticky_bottom);
    }

    #[test]
    fn scroll_to_bottom_sets_sticky() {
        let mut state = ScrollBoxState::new();
        state.sticky_bottom = false;
        state.scroll_to_bottom(20, 10);
        assert_eq!(state.viewport_top, 10);
        assert!(state.sticky_bottom);
    }

    #[test]
    fn scroll_up_clamps_at_zero() {
        let mut state = ScrollBoxState::new();
        state.viewport_top = 2;
        state.scroll_up(5);
        assert_eq!(state.viewport_top, 0);
    }
}
