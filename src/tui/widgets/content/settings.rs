// per-instance settings tab: java selection, jvm args, memory bounds, and a
// linux-only system-glfw override. sits in the content tab bar (h/l cycles
// to it like the others).
//
// three short sections (Runtime / Memory / Advanced), one row per line. the
// selected row gets a short inline hint after its value instead of a
// dedicated hint line — the tab-bar footer covers the rest. values and edit
// buffers are width-clipped so a long java path or jvm-args string can
// never shove anything else around.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    Frame,
    layout::Rect,
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::Paragraph,
};

use crate::config::theme::THEME;
use crate::instance::models::{InstanceConfig, normalize_memory_value};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Row {
    Java,
    JvmArgs,
    MemoryMin,
    MemoryMax,
    SystemGlfw,
}

const ROWS: [Row; 5] = [
    Row::Java,
    Row::JvmArgs,
    Row::MemoryMin,
    Row::MemoryMax,
    Row::SystemGlfw,
];

// a thin section marker inserted between groups of rows purely for layout /
// rendering purposes; not part of the selectable ROWS list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Section {
    Runtime,
    Memory,
    Advanced,
}

impl Section {
    fn label(self) -> &'static str {
        match self {
            Section::Runtime => "RUNTIME",
            Section::Memory => "MEMORY",
            Section::Advanced => "ADVANCED",
        }
    }
}

// width reserved for the label column (marker + space + label, left-padded)
const LABEL_COL_WIDTH: usize = 16;

impl Row {
    fn label(self) -> &'static str {
        match self {
            Row::Java => "Java",
            Row::JvmArgs => "JVM Arguments",
            Row::MemoryMin => "Memory Min",
            Row::MemoryMax => "Memory Max",
            Row::SystemGlfw => "System GLFW",
        }
    }

    // section this row belongs to, used to decide where to draw a header
    fn section(self) -> Section {
        match self {
            Row::Java | Row::JvmArgs => Section::Runtime,
            Row::MemoryMin | Row::MemoryMax => Section::Memory,
            Row::SystemGlfw => Section::Advanced,
        }
    }

    // short inline hint appended after the value when this row is selected.
    // Memory/Advanced rows keep their row clean (gauge or bare value only) —
    // only Runtime rows get an inline hint.
    fn hint(self) -> Option<&'static str> {
        match self {
            Row::Java => Some("⏎ picker"),
            Row::JvmArgs => Some("spaces separate flags"),
            Row::MemoryMin | Row::MemoryMax | Row::SystemGlfw => None,
        }
    }
}

#[derive(Default)]
enum Edit {
    #[default]
    None,
    Active(String),
}

#[derive(Default)]
pub struct SettingsTabState {
    selected: usize,
    edit: Edit,
}

impl SettingsTabState {
    pub fn is_at_top(&self) -> bool {
        self.selected == 0 && matches!(self.edit, Edit::None)
    }

    // true while a text field (Java path, JVM args, Memory Min/Max) has an
    // active edit buffer — used by the tab footer to swap "launch/kill/..."
    // for "confirm/cancel" while typing.
    pub fn is_editing(&self) -> bool {
        matches!(self.edit, Edit::Active(_))
    }
}

pub enum SettingsTabAction {
    // consumed the key but there's nothing for the caller to do (selection
    // moved, edit buffer changed, etc.)
    None,
    // didn't touch this key at all — caller should fall through to whatever
    // it normally does (tab switching, focus changes, global shortcuts...)
    Unhandled,
    UpdateInstance(InstanceConfig),
    Error(String),
    // Java row's Enter key: hand off to the java picker popup instead of
    // handling it inline, since choosing a runtime needs its own overlay.
    OpenJavaPicker,
}

pub fn handle_key(
    key_event: &KeyEvent,
    state: &mut SettingsTabState,
    instance: Option<&InstanceConfig>,
) -> SettingsTabAction {
    let Some(instance) = instance else {
        return SettingsTabAction::Unhandled;
    };

    if let Edit::Active(buf) = &state.edit {
        return match key_event.code {
            KeyCode::Enter => {
                let value = buf.trim().to_string();
                state.edit = Edit::None;
                commit_edit(ROWS[state.selected], instance, &value)
            }
            KeyCode::Esc => {
                state.edit = Edit::None;
                SettingsTabAction::None
            }
            KeyCode::Backspace => {
                let mut next = buf.clone();
                next.pop();
                state.edit = Edit::Active(next);
                SettingsTabAction::None
            }
            KeyCode::Char(c) => {
                let mut next = buf.clone();
                next.push(c);
                state.edit = Edit::Active(next);
                SettingsTabAction::None
            }
            // while editing, everything else (arrows, tab, etc.) is just
            // ignored rather than leaking out to global bindings — you're
            // mid-edit, Esc is the way out
            _ => SettingsTabAction::None,
        };
    }

    match key_event.code {
        KeyCode::Char('j') | KeyCode::Down => {
            state.selected = (state.selected + 1).min(ROWS.len() - 1);
            SettingsTabAction::None
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.selected = state.selected.saturating_sub(1);
            SettingsTabAction::None
        }
        // manual path entry for the Java row — the picker also offers this
        // (press 'a' there); this is just a quicker shortcut from the row.
        KeyCode::Char('i') if ROWS[state.selected] == Row::Java => {
            state.edit = Edit::Active(instance.java_path.clone().unwrap_or_default());
            SettingsTabAction::None
        }
        KeyCode::Enter => match ROWS[state.selected] {
            Row::Java => SettingsTabAction::OpenJavaPicker,
            Row::JvmArgs => {
                state.edit = Edit::Active(instance.jvm_args.join(" "));
                SettingsTabAction::None
            }
            Row::MemoryMin => {
                state.edit = Edit::Active(instance.memory_min.clone().unwrap_or_default());
                SettingsTabAction::None
            }
            Row::MemoryMax => {
                state.edit = Edit::Active(instance.memory_max.clone().unwrap_or_default());
                SettingsTabAction::None
            }
            Row::SystemGlfw => toggle_glfw(instance),
        },
        // not one of ours — h/l (tab switching), I/C/A (focus switching),
        // q (quit), Esc, etc. all need to reach the global dispatcher
        _ => SettingsTabAction::Unhandled,
    }
}

fn commit_edit(row: Row, instance: &InstanceConfig, value: &str) -> SettingsTabAction {
    let mut updated = instance.clone();
    match row {
        Row::Java => {
            updated.java_path = if value.is_empty() {
                None
            } else {
                Some(value.to_string())
            };
        }
        Row::JvmArgs => {
            updated.jvm_args = value.split_whitespace().map(str::to_string).collect();
        }
        Row::MemoryMin => {
            if value.is_empty() {
                updated.memory_min = None;
            } else {
                let Some(normalized) = normalize_memory_value(value) else {
                    return SettingsTabAction::Error(format!(
                        "Invalid memory value '{value}' (expected e.g. 512M, 2G, or 2.5G)"
                    ));
                };
                updated.memory_min = Some(normalized);
            }
        }
        Row::MemoryMax => {
            if value.is_empty() {
                updated.memory_max = None;
            } else {
                let Some(normalized) = normalize_memory_value(value) else {
                    return SettingsTabAction::Error(format!(
                        "Invalid memory value '{value}' (expected e.g. 2G, 4096M, or 2.7G)"
                    ));
                };
                updated.memory_max = Some(normalized);
            }
        }
        Row::SystemGlfw => unreachable!("not a text-edit row"),
    }
    SettingsTabAction::UpdateInstance(updated)
}

fn toggle_glfw(instance: &InstanceConfig) -> SettingsTabAction {
    if !cfg!(target_os = "linux") {
        return SettingsTabAction::Error("System GLFW override is Linux-only".to_string());
    }
    let mut updated = instance.clone();
    updated.use_system_glfw = !updated.use_system_glfw;
    SettingsTabAction::UpdateInstance(updated)
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    is_focused: bool,
    state: &SettingsTabState,
    instance: Option<&InstanceConfig>,
) {
    let theme = THEME.as_ref();
    let Some(instance) = instance else {
        frame.render_widget(
            Paragraph::new("No instance selected.").style(Style::default().fg(theme.text_dim())),
            area,
        );
        return;
    };

    // everything after the label column has to fit in whatever's left of
    // the row, so a long java path or jvm-args string gets clipped instead
    // of silently overflowing/wrapping and shoving later lines around
    let value_budget = (area.width as usize).saturating_sub(LABEL_COL_WIDTH);

    let mut lines: Vec<Line> = Vec::new();
    let mut current_section: Option<Section> = None;

    for (i, row) in ROWS.iter().enumerate() {
        let section = row.section();
        if current_section != Some(section) {
            if current_section.is_some() {
                lines.push(Line::from(""));
            }
            lines.push(section_header_line(theme, section));
            current_section = Some(section);
        }

        let row_selected = is_focused && i == state.selected;
        let editing = row_selected && matches!(state.edit, Edit::Active(_));

        let marker = if row_selected { "\u{258f}" } else { " " };
        let marker_style = Style::default().fg(theme.accent());

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
            Span::styled(marker, marker_style),
            Span::styled(format!(" {:<14}", row.label()), label_style),
        ];

        if editing {
            let Edit::Active(buf) = &state.edit else {
                unreachable!()
            };
            // leave one column free for the blinking cursor block
            let visible = clip_tail(buf, value_budget.saturating_sub(1));
            spans.push(Span::styled(visible, value_style));
            spans.push(Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme.accent())
                    .add_modifier(Modifier::SLOW_BLINK),
            ));
        } else {
            let hint = if row_selected { row.hint() } else { None };
            push_value_with_hint(
                &mut spans,
                &display_value(*row, instance),
                hint,
                value_budget,
                value_style,
                Style::default().fg(theme.text_dim()),
            );
        }

        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

// clips a string to the last `max_chars` characters, prefixed with an
// ellipsis if anything was cut — used while editing so the cursor (which is
// always at the end) stays visible instead of scrolling off-screen.
fn clip_tail(s: &str, max_chars: usize) -> String {
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

// clips a string to the first `max_chars` characters, suffixed with an
// ellipsis if anything was cut — used for display values, which read
// left-to-right so the front (e.g. the start of a path) matters more.
fn clip_head(s: &str, max_chars: usize) -> String {
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
    let head: String = s.chars().take(max_chars - 1).collect();
    format!("{head}…")
}

// appends the (possibly clipped) value, and — only if there's still room
// left — a dim inline hint after it. never lets the two together exceed
// `budget` columns.
fn push_value_with_hint<'a>(
    spans: &mut Vec<Span<'a>>,
    value: &str,
    hint: Option<&'a str>,
    budget: usize,
    value_style: Style,
    hint_style: Style,
)  {
    let Some(hint) = hint else {
        spans.push(Span::styled(clip_head(value, budget), value_style));
        return;
    };

    let separator = "   ";
    let hint_cost = hint.chars().count() + separator.chars().count();
    let value_len = value.chars().count();

    if value_len + hint_cost <= budget {
        // plenty of room: show the value in full plus the hint
        spans.push(Span::styled(value.to_string(), value_style));
        spans.push(Span::raw(separator));
        spans.push(Span::styled(hint, hint_style));
    } else if hint_cost < budget {
        // not enough room for both in full — clip the value to make space
        let value_budget = budget - hint_cost;
        spans.push(Span::styled(clip_head(value, value_budget), value_style));
        spans.push(Span::raw(separator));
        spans.push(Span::styled(hint, hint_style));
    } else {
        // too narrow even for the hint alone — drop it, just show the value
        spans.push(Span::styled(clip_head(value, budget), value_style));
    }
}

fn section_header_line(theme: &dyn ratatui_themekit::Theme, section: Section) -> Line<'static> {
    // BOLD and DIM on the same span fight each other in most terminals and
    // collapse to plain-weight text — which, combined with the dim fg, is
    // why these were blending into the rows below them. a distinct accent
    // tint at full weight fixes that on its own — no rule, no letter
    // spacing, just the label.
    let label_style = Style::default()
        .fg(theme.accent_dim())
        .add_modifier(Modifier::BOLD);
    Line::from(Span::styled(section.label(), label_style))
}

fn display_value(row: Row, instance: &InstanceConfig) -> String {
    match row {
        Row::Java => instance
            .java_path
            .clone()
            .unwrap_or_else(|| "<auto>".to_string()),
        Row::JvmArgs => {
            if instance.jvm_args.is_empty() {
                "<none>".to_string()
            } else {
                instance.jvm_args.join(" ")
            }
        }
        Row::MemoryMin => instance
            .memory_min
            .clone()
            .unwrap_or_else(|| "512M".to_string()),
        Row::MemoryMax => instance
            .memory_max
            .clone()
            .unwrap_or_else(|| "2G".to_string()),
        Row::SystemGlfw => {
            if cfg!(target_os = "linux") {
                if instance.use_system_glfw { "on" } else { "off" }.to_string()
            } else {
                "n/a (linux only)".to_string()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_head_short_string_unchanged() {
        assert_eq!(clip_head("hello", 10), "hello");
    }

    #[test]
    fn clip_head_truncates_with_ellipsis() {
        assert_eq!(clip_head("/usr/lib/jvm/default/bin/java", 10), "/usr/lib/…");
    }

    #[test]
    fn clip_tail_truncates_with_leading_ellipsis() {
        assert_eq!(clip_tail("/usr/lib/jvm/default/bin/java", 10), "…/bin/java");
    }

    #[test]
    fn clip_zero_budget_is_empty() {
        assert_eq!(clip_head("anything", 0), "");
        assert_eq!(clip_tail("anything", 0), "");
    }
}
