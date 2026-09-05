// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// the "overview" panel showing download/install progress.
// shows a gauge when the total is known, or a spinner when it's not.
// when idle, shows a small tail of the launcher's own recent log lines
// (the same ring buffer the full-screen 'O' log overlay reads from)
// instead of just sitting there saying "Ready".

use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, Paragraph},
};
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::config::theme::{BORDER_STYLE, THEME};
use crate::tui::app::FocusedArea;
use crate::tui::logging::get_app_logs;
use crate::tui::progress::PROGRESS;
use ratatui_themekit::Theme;

use super::styled_title;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    focused: FocusedArea,
    throbber_state: &mut ThrobberState,
) {
    let theme = THEME.as_ref();
    let border_color = if focused == FocusedArea::Overview {
        theme.accent()
    } else {
        theme.border()
    };

    let block = Block::default()
        .title(styled_title("Overview", true))
        .borders(Borders::ALL)
        .border_type(BORDER_STYLE.to_border_type())
        .border_style(Style::default().fg(border_color));

    let state = match PROGRESS.lock() {
        Ok(s) => s.clone(),
        Err(_) => {
            render_idle(frame, area, block, theme);
            return;
        }
    };

    if state.current_action.is_none() {
        render_idle(frame, area, block, theme);
        return;
    }

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let action_text = state.current_action.as_deref().unwrap_or("");
    let sub_text = state.sub_action.as_deref().unwrap_or("");

    match state.progress {
        Some((current, total)) if total > 0 => {
            let chunks = Layout::vertical([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);

            let ratio = (current as f64 / total as f64).min(1.0);
            let gauge = Gauge::default()
                .gauge_style(
                    Style::default()
                        .fg(theme.success())
                        .bg(theme.surface())
                        .add_modifier(Modifier::BOLD),
                )
                .percent((ratio * 100.0) as u16);
            frame.render_widget(gauge, chunks[0]);
            frame.render_widget(
                Paragraph::new(action_text).style(Style::default().fg(theme.text())),
                chunks[1],
            );
            if !sub_text.is_empty() {
                frame.render_widget(
                    Paragraph::new(sub_text).style(Style::default().fg(theme.text_dim())),
                    chunks[2],
                );
            }
        }
        _ => {
            let chunks =
                Layout::vertical([Constraint::Length(1), Constraint::Length(1)]).split(inner);
            let throbber = Throbber::default()
                .label(action_text)
                .style(Style::default().fg(theme.text()))
                .throbber_style(
                    Style::default()
                        .fg(theme.text_dim())
                        .add_modifier(Modifier::BOLD),
                );
            frame.render_stateful_widget(throbber, chunks[0], throbber_state);
            if !sub_text.is_empty() {
                frame.render_widget(
                    Paragraph::new(sub_text).style(Style::default().fg(theme.text_dim())),
                    chunks[1],
                );
            }
        }
    }
}

// idle: tail the launcher's own recent log lines into the panel instead
// of a bare "Ready" — which is what shows if nothing's been logged yet
// (e.g. right at startup).
fn render_idle(frame: &mut Frame, area: Rect, block: Block, theme: &dyn Theme) {
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let logs = get_app_logs();
    if logs.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("Ready", Style::default().fg(theme.text_dim()))),
            inner,
        );
        return;
    }

    let visible = inner.height as usize;
    let start = logs.len().saturating_sub(visible);
    let lines: Vec<Line> = logs[start..]
        .iter()
        .map(|line| Line::from(Span::styled(line.clone(), idle_log_line_style(line, theme))))
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

fn idle_log_line_style(line: &str, theme: &dyn Theme) -> Style {
    let upper = line.to_uppercase();
    if upper.contains("ERROR") {
        Style::default().fg(theme.error())
    } else if upper.contains("WARN") {
        Style::default().fg(theme.warning())
    } else {
        Style::default().fg(theme.text_dim())
    }
}
