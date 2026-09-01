// "pick a java runtime" popup, opened from the instance Settings tab's Java
// row. lists every runtime discover_java_installations() found (PATH,
// JAVA_HOME, and well-known per-OS install dirs, deduplicated by resolved
// path), each with its detected version when available. Enter picks one and
// closes the popup; 'a' switches to typing a custom path instead; Esc backs
// out of whichever mode is active (custom-path entry, then the popup itself).

use std::sync::LazyLock;
use std::sync::Mutex;

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, Widget, Wrap},
};

use crate::config::theme::THEME;
use crate::net::JavaCandidate;

static STATE: LazyLock<Mutex<JavaSelectState>> =
    LazyLock::new(|| Mutex::new(JavaSelectState::default()));

#[derive(Default)]
struct JavaSelectState {
    candidates: Vec<JavaCandidate>,
    selected: usize,
    current: Option<String>,
    loading: bool,
    loaded: bool,
    // Some(buffer) while typing a custom path (triggered by 'a'), None while
    // browsing the discovered-runtimes list
    custom_edit: Option<String>,
}

// runs discovery (blocking — a handful of filesystem lookups plus a
// `-version` per candidate, fast enough to do inline on first open) and
// opens the popup focused on the instance's current java_path, if it's in
// the list.
pub fn open(current: Option<String>) {
    let Ok(mut state) = STATE.lock() else {
        return;
    };
    state.loading = true;
    state.current = current;
    state.custom_edit = None;
    drop(state);

    let candidates = crate::net::discover_java_installations();

    if let Ok(mut state) = STATE.lock() {
        state.selected = state
            .current
            .as_deref()
            .and_then(|cur| candidates.iter().position(|c| c.path == cur))
            .unwrap_or(0);
        state.candidates = candidates;
        state.loading = false;
        state.loaded = true;
    }
}

pub fn close() {
    if let Ok(mut state) = STATE.lock() {
        *state = JavaSelectState::default();
    }
}

pub fn is_open() -> bool {
    STATE.lock().map(|s| s.loaded).unwrap_or(false)
}

pub enum JavaSelectAction {
    None,
    Pick(String),
    Cancel,
}

pub fn handle_key(key_event: &KeyEvent) -> JavaSelectAction {
    let Ok(mut state) = STATE.lock() else {
        return JavaSelectAction::None;
    };

    if let Some(buf) = state.custom_edit.clone() {
        return match key_event.code {
            KeyCode::Enter => {
                let value = buf.trim().to_string();
                if value.is_empty() {
                    JavaSelectAction::None
                } else {
                    JavaSelectAction::Pick(value)
                }
            }
            // Esc backs out of custom-path entry back to the list, rather
            // than closing the whole popup
            KeyCode::Esc => {
                state.custom_edit = None;
                JavaSelectAction::None
            }
            KeyCode::Backspace => {
                let mut next = buf;
                next.pop();
                state.custom_edit = Some(next);
                JavaSelectAction::None
            }
            KeyCode::Char(c) => {
                let mut next = buf;
                next.push(c);
                state.custom_edit = Some(next);
                JavaSelectAction::None
            }
            _ => JavaSelectAction::None,
        };
    }

    match key_event.code {
        KeyCode::Char('j') | KeyCode::Down => {
            if !state.candidates.is_empty() {
                state.selected = (state.selected + 1).min(state.candidates.len() - 1);
            }
            JavaSelectAction::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            JavaSelectAction::None
        }
        KeyCode::Char('g') => {
            state.selected = 0;
            JavaSelectAction::None
        }
        KeyCode::Char('G') => {
            state.selected = state.candidates.len().saturating_sub(1);
            JavaSelectAction::None
        }
        KeyCode::Char('a') => {
            state.custom_edit = Some(state.current.clone().unwrap_or_default());
            JavaSelectAction::None
        }
        KeyCode::Enter => match state.candidates.get(state.selected) {
            Some(candidate) => JavaSelectAction::Pick(candidate.path.clone()),
            None => JavaSelectAction::None,
        },
        KeyCode::Esc => JavaSelectAction::Cancel,
        _ => JavaSelectAction::None,
    }
}

pub fn popup_rect(frame_area: Rect) -> Rect {
    let popup_w = 64u16.min(frame_area.width.saturating_sub(4));
    let popup_h = 16u16.min(frame_area.height.saturating_sub(4));
    frame_area.centered(Constraint::Length(popup_w), Constraint::Length(popup_h))
}

pub fn render(frame: &mut Frame, area: Rect) {
    use super::base::PopupFrame;
    use super::keybind_line;

    let theme = THEME.as_ref();
    let Ok(state) = STATE.lock() else {
        return;
    };

    let title = Line::from(vec![Span::styled(
        " Choose a Java Runtime ",
        Style::default()
            .fg(theme.text_bright())
            .add_modifier(Modifier::BOLD),
    )]);

    let kb = if state.custom_edit.is_some() {
        keybind_line(&[("⏎", " use path"), ("Esc", " back to list")])
    } else {
        keybind_line(&[
            ("j/k", " navigate"),
            ("⏎", " select"),
            ("a", " custom path"),
            ("Esc", " cancel"),
        ])
    };

    let border_color = theme.accent();
    let bg_color = theme.surface();

    let loading = state.loading;
    let candidates = state.candidates.clone();
    let current = state.current.clone();
    let selected = state.selected;
    let custom_edit = state.custom_edit.clone();

    let popup = PopupFrame {
        title,
        border_color,
        bg: Some(bg_color),
        keybinds: Some(kb),
        search_line: None,
        content: Box::new(move |inner, buf| {
            if let Some(edit_buf) = &custom_edit {
                let spans = vec![
                    Span::styled("Path: ", Style::default().fg(theme.text_dim())),
                    Span::styled(
                        edit_buf.clone(),
                        Style::default()
                            .fg(theme.accent())
                            .add_modifier(Modifier::BOLD),
                    ),
                    Span::styled(
                        "\u{2588}",
                        Style::default()
                            .fg(theme.accent())
                            .add_modifier(Modifier::SLOW_BLINK),
                    ),
                ];
                Paragraph::new(vec![
                    Line::from(spans),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Type the full path to a java binary, e.g. /usr/lib/jvm/temurin-21/bin/java",
                        Style::default().fg(theme.text_dim()),
                    )),
                ])
                .wrap(Wrap { trim: true })
                .render(inner, buf);
                return;
            }

            if loading {
                Paragraph::new("Scanning for java installations...")
                    .style(Style::default().fg(theme.text_dim()))
                    .render(inner, buf);
                return;
            }

            if candidates.is_empty() {
                Paragraph::new(
                    "No java runtimes found on PATH, JAVA_HOME, or common install locations.\n\
                     Press 'a' to type a path manually.",
                )
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.text_dim()))
                .render(inner, buf);
                return;
            }

            let items: Vec<ListItem> = candidates
                .iter()
                .map(|candidate| list_item(theme, candidate, current.as_deref()))
                .collect();

            let mut list_state = ListState::default().with_selected(Some(selected));
            let list = List::new(items).highlight_style(
                Style::default()
                    .fg(theme.accent())
                    .add_modifier(Modifier::BOLD),
            );
            ratatui::widgets::StatefulWidget::render(list, inner, buf, &mut list_state);
        }),
    };

    frame.render_widget(popup, area);
}

fn list_item<'a>(
    theme: &'a dyn ratatui_themekit::Theme,
    candidate: &JavaCandidate,
    current: Option<&str>,
) -> ListItem<'a> {
    let is_current = current == Some(candidate.path.as_str());
    let marker = if is_current { "\u{2713} " } else { "  " };
    let marker_style = if is_current {
        Style::default().fg(theme.success())
    } else {
        Style::default().fg(theme.text_dim())
    };

    let mut version_label = candidate
        .version
        .as_deref()
        .map(|v| format!("java {v}"))
        .unwrap_or_else(|| "version unknown".to_string());
    if candidate.provisioned {
        // this is a runtime alloy downloaded for an instance (lives under
        // <data_dir>/alloy/bin, not on PATH/JAVA_HOME) — flag it so it
        // doesn't look like an unexplained system install.
        version_label.push_str(" (alloy)");
    }

    ListItem::new(Line::from(vec![
        Span::styled(marker, marker_style),
        Span::styled(
            format!("{version_label:<24}"),
            Style::default().fg(theme.text()),
        ),
        Span::styled(candidate.path.clone(), Style::default().fg(theme.text_dim())),
    ]))
}
