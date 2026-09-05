// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// global settings popup: paths, memory defaults, and UI timing knobs from
// config.toml, opened with 's' from the sidebar. distinct from the
// per-instance Settings tab (content/settings.rs), which edits one
// instance's InstanceConfig rather than the shared global Config.
//
// unlike the per-instance tab, edits here are NOT auto-saved per field —
// they live in an in-memory buffer until Ctrl+S writes config.toml.
// SETTINGS is a LazyLock read once at startup (config/mod.rs), so edits
// can't apply to the running process; closing (Esc) after a Ctrl+S save
// respawns alloy to pick them up. closing without saving discards.

use std::sync::LazyLock;
use std::sync::Mutex;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{Paragraph, Widget},
};

use crate::config::Config;
use crate::config::settings::ImageProtocol;
use crate::config::theme::THEME;
use crate::instance::models::normalize_memory_value;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    InstancesDir,
    MetaDir,
    JavaPath,
    MemoryMin,
    MemoryMax,
    ImageProtocol,
    ErrorAutoDismissMs,
    ErrorSlideStartMs,
    ErrorFlyOutMs,
    MaxErrorEvents,
    CurseForgeApiKey,
}

const ROWS: [Row; 11] = [
    Row::InstancesDir,
    Row::MetaDir,
    Row::JavaPath,
    Row::MemoryMin,
    Row::MemoryMax,
    Row::ImageProtocol,
    Row::ErrorAutoDismissMs,
    Row::ErrorSlideStartMs,
    Row::ErrorFlyOutMs,
    Row::MaxErrorEvents,
    Row::CurseForgeApiKey,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Paths,
    Defaults,
    Ui,
    CurseForge,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Paths => "PATHS",
            Section::Defaults => "DEFAULTS",
            Section::Ui => "UI",
            Section::CurseForge => "CURSEFORGE",
        }
    }
}

// width reserved for the label column (marker + space + label, left-padded)
const LABEL_COL_WIDTH: usize = 22;

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::InstancesDir => "Instances Dir",
            Row::MetaDir => "Meta Dir",
            Row::JavaPath => "Java",
            Row::MemoryMin => "Memory Min",
            Row::MemoryMax => "Memory Max",
            Row::ImageProtocol => "Image Protocol",
            Row::ErrorAutoDismissMs => "Toast Dismiss (ms)",
            Row::ErrorSlideStartMs => "Toast Slide (ms)",
            Row::ErrorFlyOutMs => "Toast Fly Out (ms)",
            Row::MaxErrorEvents => "Max Toast Events",
            Row::CurseForgeApiKey => "CurseForge Key",
        }
    }

    fn section(self) -> Section {
        match self {
            Row::InstancesDir | Row::MetaDir | Row::JavaPath => Section::Paths,
            Row::MemoryMin | Row::MemoryMax => Section::Defaults,
            Row::ImageProtocol
            | Row::ErrorAutoDismissMs
            | Row::ErrorSlideStartMs
            | Row::ErrorFlyOutMs
            | Row::MaxErrorEvents => Section::Ui,
            Row::CurseForgeApiKey => Section::CurseForge,
        }
    }

    // short inline hint appended after the value when this row is selected
    fn hint(self) -> Option<&'static str> {
        match self {
            Row::ImageProtocol => Some("⏎ cycles"),
            Row::JavaPath => Some("⏎ select other"),
            _ => None,
        }
    }
}

#[derive(Default)]
enum Edit {
    #[default]
    None,
    Active(String),
}

struct GlobalSettingsState {
    open: bool,
    config: Config,
    selected: usize,
    edit: Edit,
    // true once any field has been changed since the last save (or since
    // opening). drives the "unsaved changes" status line.
    dirty: bool,
    // true once Ctrl+S has written the current config to disk. if still
    // true when the popup closes, alloy restarts to pick it up.
    saved: bool,
}

impl Default for GlobalSettingsState {
    fn default() -> Self {
        Self {
            open: false,
            config: crate::config::SETTINGS.clone(),
            selected: 0,
            edit: Edit::None,
            dirty: false,
            saved: false,
        }
    }
}

static STATE: LazyLock<Mutex<GlobalSettingsState>> =
    LazyLock::new(|| Mutex::new(GlobalSettingsState::default()));

// snapshots the currently-loaded SETTINGS into the edit buffer and opens
// the popup focused on the first row.
pub fn open() {
    if let Ok(mut state) = STATE.lock() {
        state.config = crate::config::SETTINGS.clone();
        state.selected = 0;
        state.edit = Edit::None;
        state.dirty = false;
        state.saved = false;
        state.open = true;
    }
}

pub fn close() {
    if let Ok(mut state) = STATE.lock() {
        *state = GlobalSettingsState::default();
    }
}

pub fn is_open() -> bool {
    STATE.lock().map(|s| s.open).unwrap_or(false)
}

// JavaSelect popup picked a runtime while targeting the global config
// (app::JavaSelectTarget). marks the buffer dirty like any other edit —
// still needs Ctrl+S to persist.
pub fn set_java_path(path: Option<String>) {
    if let Ok(mut state) = STATE.lock() {
        state.config.paths.java_path = path;
        state.dirty = true;
    }
}

pub enum GlobalSettingsAction {
    None,
    Error(String),
    OpenJavaPicker(Option<String>),
    Close,
    CloseAndRestart,
}

pub fn handle_key(key_event: &KeyEvent) -> GlobalSettingsAction {
    let Ok(mut state) = STATE.lock() else {
        return GlobalSettingsAction::None;
    };

    if let Edit::Active(buf) = &state.edit {
        return match key_event.code {
            KeyCode::Enter => {
                let value = buf.trim().to_string();
                state.edit = Edit::None;
                let row = ROWS[state.selected];
                match build_updated_config(row, &state.config, &value) {
                    Ok(updated) => {
                        state.config = updated;
                        state.dirty = true;
                        GlobalSettingsAction::None
                    }
                    Err(message) => GlobalSettingsAction::Error(message),
                }
            }
            KeyCode::Esc => {
                state.edit = Edit::None;
                GlobalSettingsAction::None
            }
            KeyCode::Backspace => {
                let mut next = buf.clone();
                next.pop();
                state.edit = Edit::Active(next);
                GlobalSettingsAction::None
            }
            KeyCode::Char(c) => {
                let mut next = buf.clone();
                next.push(c);
                state.edit = Edit::Active(next);
                GlobalSettingsAction::None
            }
            // mid-edit: ignore everything else rather than leaking out to
            // global bindings -- Esc is the way out
            _ => GlobalSettingsAction::None,
        };
    }

    match key_event.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.selected = (state.selected + 1).min(ROWS.len() - 1);
            GlobalSettingsAction::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            GlobalSettingsAction::None
        }
        KeyCode::Char('s') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            match crate::config::save_config(&state.config) {
                Ok(()) => {
                    state.saved = true;
                    state.dirty = false;
                    GlobalSettingsAction::None
                }
                Err(e) => GlobalSettingsAction::Error(format!("Failed to save config: {e}")),
            }
        }
        KeyCode::Enter => {
            let row = ROWS[state.selected];
            if row == Row::ImageProtocol {
                let mut updated = state.config.clone();
                updated.ui.image_protocol = next_protocol(state.config.ui.image_protocol);
                state.config = updated;
                state.dirty = true;
                GlobalSettingsAction::None
            } else if row == Row::JavaPath {
                GlobalSettingsAction::OpenJavaPicker(state.config.paths.java_path.clone())
            } else {
                state.edit = Edit::Active(display_value(row, &state.config));
                GlobalSettingsAction::None
            }
        }
        KeyCode::Esc => {
            if state.saved {
                GlobalSettingsAction::CloseAndRestart
            } else {
                GlobalSettingsAction::Close
            }
        }
        _ => GlobalSettingsAction::None,
    }
}

fn next_protocol(current: ImageProtocol) -> ImageProtocol {
    const ORDER: [ImageProtocol; 5] = [
        ImageProtocol::Halfblocks,
        ImageProtocol::Quadrants,
        ImageProtocol::Sixel,
        ImageProtocol::Kitty,
        ImageProtocol::Iterm2,
    ];
    let idx = ORDER.iter().position(|p| *p == current).unwrap_or(0);
    ORDER[(idx + 1) % ORDER.len()]
}

// builds a full copy of `config` with `row` set to the parsed `value`, or an
// error message if `value` doesn't parse for that row's type.
fn build_updated_config(row: Row, config: &Config, value: &str) -> Result<Config, String> {
    let mut updated = config.clone();
    match row {
        Row::InstancesDir => {
            if value.is_empty() {
                return Err("Instances directory cannot be empty".to_string());
            }
            updated.paths.instances_dir = value.to_string();
        }
        Row::MetaDir => {
            if value.is_empty() {
                return Err("Meta directory cannot be empty".to_string());
            }
            updated.paths.meta_dir = value.to_string();
        }
        Row::JavaPath => {
            updated.paths.java_path = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        Row::MemoryMin => {
            let Some(normalized) = normalize_memory_value(value) else {
                return Err(format!(
                    "Invalid memory value '{value}' (expected e.g. 512M, 2G, or 2.5G)"
                ));
            };
            updated.defaults.memory_min = normalized;
        }
        Row::MemoryMax => {
            let Some(normalized) = normalize_memory_value(value) else {
                return Err(format!(
                    "Invalid memory value '{value}' (expected e.g. 2G, 4096M, or 2.7G)"
                ));
            };
            updated.defaults.memory_max = normalized;
        }
        Row::ErrorAutoDismissMs => {
            updated.ui.error_auto_dismiss_ms = parse_ms(value)?;
        }
        Row::ErrorSlideStartMs => {
            updated.ui.error_slide_start_ms = parse_ms(value)?;
        }
        Row::ErrorFlyOutMs => {
            updated.ui.error_fly_out_ms = parse_ms(value)?;
        }
        Row::MaxErrorEvents => match value.parse::<usize>() {
            Ok(n) if n > 0 => updated.ui.max_error_events = n,
            Ok(_) => return Err("Max toast events must be at least 1".to_string()),
            Err(_) => return Err(format!("'{value}' isn't a whole number")),
        },
        Row::CurseForgeApiKey => {
            let trimmed = value.trim();
            updated.curseforge.api_key = if trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            };
        }
        Row::ImageProtocol => unreachable!("cycled via Enter, never text-edited"),
    }
    Ok(updated)
}

fn parse_ms(value: &str) -> Result<u64, String> {
    value
        .parse::<u64>()
        .map_err(|_| format!("'{value}' isn't a whole number of milliseconds"))
}

pub fn popup_rect(frame_area: Rect) -> Rect {
    let popup_w = 68u16.min(frame_area.width.saturating_sub(4));
    let popup_h = 22u16.min(frame_area.height.saturating_sub(4));
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
        " Global Settings ",
        Style::default()
            .fg(theme.text_bright())
            .add_modifier(Modifier::BOLD),
    )]);

    let editing = matches!(state.edit, Edit::Active(_));
    let kb = if editing {
        keybind_line(&[("⏎", " save"), ("Esc", " cancel")])
    } else {
        keybind_line(&[
            ("j/k", " navigate"),
            ("⏎", " edit/cycle/pick"),
            ("ctrl+s", " save"),
            ("Esc", " close"),
        ])
    };

    let border_color = theme.accent();
    let bg_color = theme.surface();
    let selected = state.selected;
    let edit_buf = match &state.edit {
        Edit::Active(buf) => Some(buf.clone()),
        Edit::None => None,
    };
    let config = state.config.clone();
    drop(state);

    let popup = PopupFrame {
        title,
        border_color,
        bg: Some(bg_color),
        keybinds: Some(kb),
        search_line: None,
        content: Box::new(move |inner, buf| {
            let value_budget = (inner.width as usize).saturating_sub(LABEL_COL_WIDTH);
            let mut lines: Vec<Line> = Vec::new();
            let mut current_section: Option<Section> = None;

            for (i, row) in ROWS.iter().enumerate() {
                let section = row.section();
                if current_section != Some(section) {
                    if current_section.is_some() {
                        lines.push(Line::from(""));
                    }
                    lines.push(Line::from(Span::styled(
                        section.label(),
                        Style::default()
                            .fg(theme.accent_dim())
                            .add_modifier(Modifier::BOLD),
                    )));
                    current_section = Some(section);
                }

                let row_selected = i == selected;
                let row_editing = row_selected && edit_buf.is_some();

                let marker = if row_selected { "\u{258f}" } else { " " };
                let label_style = if row_selected {
                    Style::default()
                        .fg(theme.text_bright())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text_dim())
                };
                let value_style = if row_selected {
                    Style::default()
                        .fg(theme.accent())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(theme.text())
                };

                let mut spans = vec![
                    Span::styled(marker, Style::default().fg(theme.accent())),
                    Span::styled(format!(" {:<20}", row.label()), label_style),
                ];

                if row_editing {
                    let buf_text = edit_buf.clone().unwrap_or_default();
                    let visible = truncate(&buf_text, value_budget.saturating_sub(1));
                    spans.push(Span::styled(visible, value_style));
                    spans.push(Span::styled(
                        "\u{2588}",
                        Style::default()
                            .fg(theme.accent())
                            .add_modifier(Modifier::SLOW_BLINK),
                    ));
                } else {
                    let value = display_value(*row, &config);
                    let value = if *row == Row::CurseForgeApiKey {
                        if config.curseforge.is_using_default_key() {
                            "(default key)".to_string()
                        } else {
                            mask_secret(&value)
                        }
                    } else {
                        value
                    };
                    let hint = if row_selected { row.hint() } else { None };
                    match hint {
                        Some(hint)
                            if value.chars().count() + hint.chars().count() + 3
                                <= value_budget =>
                        {
                            spans.push(Span::styled(value, value_style));
                            spans.push(Span::raw("   "));
                            spans.push(Span::styled(
                                hint,
                                Style::default().fg(theme.text_dim()),
                            ));
                        }
                        _ => {
                            spans.push(Span::styled(truncate(&value, value_budget), value_style));
                        }
                    }
                }

                lines.push(Line::from(spans));
            }

            Paragraph::new(lines).render(inner, buf);
        }),
    };

    frame.render_widget(popup, area);
}

fn display_value(row: Row, config: &Config) -> String {
    match row {
        Row::InstancesDir => config.paths.instances_dir.clone(),
        Row::MetaDir => config.paths.meta_dir.clone(),
        Row::JavaPath => config
            .paths
            .java_path
            .clone()
            .unwrap_or_else(default_java_path),
        Row::MemoryMin => config.defaults.memory_min.clone(),
        Row::MemoryMax => config.defaults.memory_max.clone(),
        Row::ImageProtocol => format!("{:?}", config.ui.image_protocol).to_lowercase(),
        Row::ErrorAutoDismissMs => config.ui.error_auto_dismiss_ms.to_string(),
        Row::ErrorSlideStartMs => config.ui.error_slide_start_ms.to_string(),
        Row::ErrorFlyOutMs => config.ui.error_fly_out_ms.to_string(),
        Row::MaxErrorEvents => config.ui.max_error_events.to_string(),
        Row::CurseForgeApiKey => config.curseforge.api_key.clone().unwrap_or_default(),
    }
}

// masks a secret for display when the row isn't being edited — shows only
// the last 4 characters so a shoulder-surfer (or screen recording) can't
// read it, while still letting the user confirm which key is saved. the
// edit buffer is always seeded from the raw `display_value`, never this
// masked form, so editing never corrupts the key with bullet characters.
fn mask_secret(value: &str) -> String {
    if value.is_empty() {
        return "(not set)".to_string();
    }
    let chars: Vec<char> = value.chars().collect();
    let visible = 4.min(chars.len());
    let hidden = (chars.len() - visible).min(24);
    let dots: String = "•".repeat(hidden);
    let tail: String = chars[chars.len() - visible..].iter().collect();
    format!("{dots}{tail}")
}

// the runtime that'd actually be used when paths.java_path is unset --
// the newest detected install, same as the first-run auto-select in
// config::ensure_config_exists.
fn default_java_path() -> String {
    crate::net::best_installed_java().unwrap_or_else(|| "not found".to_string())
}

// clips to the last `max_chars`, with a leading ellipsis — mirrors
// clip_tail in content/settings.rs so a long path can't push the popup
// border around while keeping the cursor (at the end while editing)
// visible.
fn truncate(s: &str, max_chars: usize) -> String {
    let len = s.chars().count();
    if len <= max_chars {
        return s.to_string();
    }
    if max_chars == 0 {
        return String::new();
    }
    if max_chars == 1 {
        return "…".to_string();
    }
    let tail: String = s.chars().skip(len - (max_chars - 1)).collect();
    format!("…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_protocol_wraps_around() {
        assert_eq!(next_protocol(ImageProtocol::Iterm2), ImageProtocol::Halfblocks);
    }

    #[test]
    fn next_protocol_advances() {
        assert_eq!(next_protocol(ImageProtocol::Kitty), ImageProtocol::Iterm2);
    }

    #[test]
    fn build_updated_config_rejects_empty_instances_dir() {
        let config = crate::config::SETTINGS.clone();
        assert!(build_updated_config(Row::InstancesDir, &config, "").is_err());
    }

    #[test]
    fn build_updated_config_rejects_bad_memory_value() {
        let config = crate::config::SETTINGS.clone();
        assert!(build_updated_config(Row::MemoryMin, &config, "not-a-size").is_err());
    }

    #[test]
    fn build_updated_config_accepts_valid_memory_value() {
        let config = crate::config::SETTINGS.clone();
        let updated = build_updated_config(Row::MemoryMax, &config, "4G").unwrap();
        assert_eq!(updated.defaults.memory_max, "4G");
    }

    #[test]
    fn build_updated_config_rejects_zero_max_error_events() {
        let config = crate::config::SETTINGS.clone();
        assert!(build_updated_config(Row::MaxErrorEvents, &config, "0").is_err());
    }

    #[test]
    fn build_updated_config_sets_curseforge_key() {
        let config = crate::config::SETTINGS.clone();
        let updated = build_updated_config(Row::CurseForgeApiKey, &config, "  abc123  ").unwrap();
        assert_eq!(updated.curseforge.api_key.as_deref(), Some("abc123"));
    }

    #[test]
    fn build_updated_config_clears_curseforge_key_on_empty() {
        let mut config = crate::config::SETTINGS.clone();
        config.curseforge.api_key = Some("abc123".to_string());
        let updated = build_updated_config(Row::CurseForgeApiKey, &config, "   ").unwrap();
        assert_eq!(updated.curseforge.api_key, None);
    }

    #[test]
    fn mask_secret_shows_placeholder_when_unset() {
        assert_eq!(mask_secret(""), "(not set)");
    }

    #[test]
    fn mask_secret_keeps_last_four_chars() {
        assert_eq!(mask_secret("abcd1234"), "••••1234");
    }

    #[test]
    fn mask_secret_short_value_shows_no_dots() {
        assert_eq!(mask_secret("ab"), "ab");
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_keeps_tail() {
        assert_eq!(truncate("/usr/lib/jvm/default/bin/java", 10), "…/bin/java");
    }
}
