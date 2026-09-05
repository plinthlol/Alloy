// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// toast-style error/warning popup that auto-dismisses after a timeout.
// stacks in the top-right corner; border color changes based on severity.

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget, Wrap},
};
use tracing::Level;

use super::base::PopupFrame;
use crate::config::SETTINGS;
use crate::config::theme::THEME;
use crate::tui::error_buffer::ErrorEvent;

pub struct ErrorPopup {
    pub event: ErrorEvent,
}

impl ErrorPopup {
    pub fn new(event: ErrorEvent) -> Self {
        Self { event }
    }
}

impl Widget for ErrorPopup {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let theme = THEME.as_ref();
        let (border_color, label) = match self.event.level {
            Level::ERROR => (theme.error(), "ERROR"),
            Level::WARN => (theme.warning(), "WARN"),
            _ => (theme.text_dim(), "INFO"),
        };

        let title = Line::from(vec![Span::styled(
            format!(" {} ", label),
            Style::default()
                .fg(theme.text_bright())
                .bg(border_color)
                .add_modifier(Modifier::BOLD),
        )]);

        let message = self.event.message.clone();
        let text_color = theme.text();
        let bg_color = theme.surface();
        let popup = PopupFrame {
            title,
            border_color,
            bg: Some(bg_color),
            keybinds: None,
            search_line: None,
            content: Box::new(move |inner, buf| {
                Paragraph::new(message.as_str())
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(text_color))
                    .render(inner, buf);
            }),
        };

        popup.render(area, buf);
    }
}

// returns None when the toast has lived past its expiry, which is
// what triggers removal from the render loop
pub fn popup_area(frame_area: Rect, message: &str, base_y: u16, elapsed_ms: u128) -> Option<Rect> {
    use super::{top_right_rect, word_wrap_size};

    const MAX_W: usize = 58;
    const MIN_W: usize = 22;

    if elapsed_ms >= SETTINGS.ui.error_auto_dismiss_ms as u128 {
        return None;
    }

    let max_h = frame_area.height.saturating_sub(base_y).saturating_sub(1);
    if max_h < 3 {
        return None;
    }

    let (w, h) = word_wrap_size(message, MAX_W);
    let inner_w = w.max(MIN_W);

    // clamp height against the space actually left below base_y (for
    // toast stacking) before handing off to the shared corner-anchor math,
    // which only knows about the frame as a whole.
    let inner_h = (max_h as usize).saturating_sub(2).min(h);
    let mut rect = top_right_rect(frame_area, inner_w, inner_h);
    rect.y = base_y;
    Some(rect)
}

#[cfg(test)]
mod render_tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;
    use tracing::Level;

    use crate::tui::error_buffer::ErrorEvent;

    fn render(event: ErrorEvent, width: u16, height: u16) -> Terminal<TestBackend> {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                let popup = ErrorPopup::new(event);
                f.render_widget(popup, f.area());
            })
            .unwrap();
        terminal
    }

    fn event(level: Level, message: &str) -> ErrorEvent {
        ErrorEvent {
            id: 1,
            level,
            message: message.to_string(),
            pushed_at: Instant::now(),
        }
    }

    #[test]
    fn warn_level_renders() {
        let _ = render(event(Level::WARN, "Disk space low"), 40, 5);
    }

    #[test]
    fn error_level_renders() {
        let _ = render(event(Level::ERROR, "Connection refused"), 40, 5);
    }

    // info-level events hit the catch-all `_` arm in the label match; previously
    // there was no test covering it.
    #[test]
    fn info_level_renders() {
        let _ = render(event(Level::INFO, "Reloaded config"), 40, 5);
    }

    #[test]
    fn long_message_wraps() {
        let msg = "The Minecraft launcher could not reach the Mojang version manifest \
                   after three retries. Check your network connection or proxy settings.";
        let _ = render(event(Level::ERROR, msg), 40, 10);
    }

    #[test]
    fn narrow_frame_renders() {
        let _ = render(event(Level::WARN, "Short message"), 18, 5);
    }
}

#[cfg(test)]
mod area_tests {
    use super::popup_area;
    use crate::config::SETTINGS;
    use ratatui::layout::Rect;

    fn frame() -> Rect {
        Rect::new(0, 0, 80, 24)
    }

    #[test]
    fn returns_none_after_dismiss_timeout() {
        let past_dismiss = SETTINGS.ui.error_auto_dismiss_ms as u128 + 1;
        assert!(popup_area(frame(), "msg", 0, past_dismiss).is_none());
    }

    #[test]
    fn returns_some_inside_dismiss_window() {
        assert!(popup_area(frame(), "msg", 0, 0).is_some());
    }

    #[test]
    fn returns_none_when_vertical_room_too_small() {
        // base_y = 22 leaves only height 24 - 22 - 1 = 1 row of usable space,
        // less than the minimum 3 needed for border + content + border.
        assert!(popup_area(frame(), "msg", 22, 0).is_none());
    }

    #[test]
    fn clamps_popup_width_to_frame() {
        // a wider-than-the-frame message gets clamped so the popup fits
        // inside frame.width minus the right-edge padding (saturating_sub(4)).
        let huge = "x".repeat(200);
        let area = popup_area(frame(), &huge, 0, 0).unwrap();
        assert!(area.width <= frame().width.saturating_sub(4));
    }

    #[test]
    fn anchors_popup_to_right_edge() {
        let area = popup_area(frame(), "msg", 0, 0).unwrap();
        // popup_w is added to base_x to reach frame.width - 2 (right-edge gutter)
        assert_eq!(area.x + area.width + 2, frame().width);
    }
}
