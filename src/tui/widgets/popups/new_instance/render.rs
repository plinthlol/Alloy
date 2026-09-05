// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// rendering for the new instance wizard. each step gets its own render fn
// and the popup resizes itself based on which step is active.

use super::state::{
    LoadState, ModpackHit, WIZARD_STATE, WizardState, WizardStep, clamp_loader_version_index,
    clamp_version_index, ensure_loader_versions_loaded, ensure_versions_loaded, visible_versions,
};
use crate::config::theme::THEME;
use crate::instance::models::ModLoader;
use crate::tui::app::FocusedArea;
use crate::tui::widgets::browse_step;
use crate::tui::widgets::popups::base::PopupFrame;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, StatefulWidget, Widget, Wrap},
};
use tui_prompts::State as PromptState;

use crate::tui::widgets::popups::description;
pub fn render(
    frame: &mut Frame,
    area: Rect,
    _focused: FocusedArea,
    picker: &ratatui_image::picker::Picker,
) {
    // Copy, so safe to move into the 'static content closure below — see
    // content_browse/render.rs for why the Picker itself doesn't travel
    // further than this.
    let fs = picker.font_size();
    let font_size = (fs.width, fs.height);
    // grab the lock, kick off any lazy-loading, then clone and release.
    // fetching happens here (in render) because the wizard is purely
    // reactive: version lists only load when you navigate to that step.
    let snapshot = match WIZARD_STATE.lock() {
        Ok(mut state) => {
            if state.step == WizardStep::Version {
                ensure_versions_loaded(&mut state);
                clamp_version_index(&mut state);
            }

            // vanilla has no loader version, so skip straight to confirm
            if state.step == WizardStep::LoaderVersion {
                if state.selected_loader() == ModLoader::Vanilla {
                    state.step = WizardStep::Confirm;
                } else {
                    clamp_loader_version_index(&mut state);
                    let game_version = state.selected_version().map(|v| v.id.clone());
                    let loader = state.selected_loader();
                    if let Some(game_version) = game_version {
                        ensure_loader_versions_loaded(&mut state, loader, game_version);
                    }
                }
            }

            state.clone()
        }
        Err(e) => {
            tracing::error!("Wizard state lock poisoned: {}", e);
            WizardState::default()
        }
    };

    let keybinds = step_keybinds(&snapshot);

    let search_line = snapshot.version_search.title_line();

    let theme = THEME.as_ref();
    let description_open = description::is_open();
    let description_title = description::title();
    let popup = PopupFrame {
        title: if description_open {
            crate::tui::widgets::styled_title(&description_title, false)
        } else {
            wizard_title(&snapshot)
        },
        border_color: theme.text_dim(),
        bg: Some(theme.surface()),
        keybinds: Some(if description_open {
            description::keybinds()
        } else {
            keybinds
        }),
        search_line: if description_open { None } else { search_line },
        content: Box::new(move |popup_area, buf| {
            // the description view replaces the wizard's content in place;
            // markdown::render runs after the frame below (needs &mut Frame)
            if description_open {
                return;
            }
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1)])
                .split(popup_area);

            match snapshot.step {
                WizardStep::Mode => render_mode_step(&snapshot, chunks[0], buf),
                WizardStep::Name => render_name_step(&snapshot, chunks[0], buf),
                WizardStep::Version => render_version_step(&snapshot, chunks[0], buf),
                WizardStep::Loader => render_loader_step(&snapshot, chunks[0], buf),
                WizardStep::LoaderVersion => render_loader_version_step(&snapshot, chunks[0], buf),
                WizardStep::Confirm => render_confirm_step(&snapshot, chunks[0], buf),
                WizardStep::ModpackBrowse => {
                    render_modpack_browse_step(&snapshot, chunks[0], buf, font_size)
                }
                WizardStep::ModpackVersion => {
                    render_modpack_version_step(&snapshot, chunks[0], buf)
                }
                WizardStep::ModpackConfirm => {
                    render_modpack_confirm_step(&snapshot, chunks[0], buf)
                }
            }
        }),
    };

    frame.render_widget(popup, area);

    if description_open {
        let inner = ratatui::widgets::Block::bordered().inner(area);
        description::render_content(frame, inner, picker);
    }
}

pub fn popup_rect(frame_area: Rect) -> Rect {
    use ratatui::layout::Constraint;

    let step = match WIZARD_STATE.lock() {
        Ok(s) => s.step.clone(),
        Err(_) => WizardStep::Name,
    };

    let w = Constraint::Percentage(50);

    match step {
        WizardStep::Mode => {
            let h = 5u16.min(frame_area.height.saturating_sub(4));
            frame_area.centered(Constraint::Percentage(40), Constraint::Length(h))
        }
        WizardStep::Name => {
            let h = 6u16.min(frame_area.height.saturating_sub(4));
            frame_area.centered(w, Constraint::Length(h))
        }
        WizardStep::Version | WizardStep::LoaderVersion => {
            let h = (frame_area.height * 2 / 3)
                .max(10)
                .min(frame_area.height.saturating_sub(4));
            frame_area.centered(w, Constraint::Length(h))
        }
        WizardStep::Loader => {
            let h = 9u16.min(frame_area.height.saturating_sub(4));
            frame_area.centered(w, Constraint::Length(h))
        }
        WizardStep::Confirm => {
            let h = 8u16.min(frame_area.height.saturating_sub(4));
            frame_area.centered(w, Constraint::Length(h))
        }
        // the modpack browser gets a near-fullscreen popup (other steps
        // stay modest centered boxes) since it shows a scrollable catalog
        // of search results rather than a short list of fixed choices — it
        // needs the room.
        WizardStep::ModpackBrowse => frame_area
            .centered(Constraint::Percentage(99), Constraint::Percentage(95)),
        WizardStep::ModpackVersion => frame_area
            .centered(Constraint::Percentage(90), Constraint::Percentage(85)),
        WizardStep::ModpackConfirm => {
            let h = 10u16.min(frame_area.height.saturating_sub(4));
            frame_area.centered(w, Constraint::Length(h))
        }
    }
}

fn wizard_title(state: &WizardState) -> Line<'static> {
    use crate::tui::widgets::styled_title;
    let title = match state.step {
        WizardStep::ModpackBrowse | WizardStep::ModpackVersion | WizardStep::ModpackConfirm => {
            "Modpacks"
        }
        _ => "New Instance",
    };
    styled_title(title, false)
}

fn step_keybinds(state: &WizardState) -> ratatui::text::Line<'static> {
    use crate::tui::widgets::popups::keybind_line;
    match state.step {
        WizardStep::Mode => keybind_line(&[("j/k", " choose"), ("Enter", " select")]),
        WizardStep::Name => keybind_line(&[("Enter", " continue")]),
        WizardStep::Loader => keybind_line(&[("b", " back"), ("Enter", " select")]),
        WizardStep::Version => keybind_line(&[
            ("/", " search"),
            ("s", " snap"),
            ("b", " back"),
            ("Enter", " select"),
        ]),
        WizardStep::LoaderVersion => keybind_line(&[("b", " back"), ("Enter", " select")]),
        WizardStep::Confirm => keybind_line(&[("b", " back"), ("Enter", " create")]),
        WizardStep::ModpackBrowse => {
            if state.modpack_query_focused {
                // search is live now - Enter just commits focus back to the
                // list instead of being what actually fires the search.
                keybind_line(&[("Tab", " source"), ("Enter", " done")])
            } else {
                keybind_line(&[
                    ("Tab", " source"),
                    ("/", " search"),
                    ("h", " home"),
                    ("Enter", " view"),
                    ("v", " versions"),
                    ("i", " install latest"),
                ])
            }
        }
        WizardStep::ModpackVersion => keybind_line(&[("b", " back"), ("Enter", " select")]),
        WizardStep::ModpackConfirm => keybind_line(&[("←", " back"), ("Enter", " install")]),
    }
}

fn render_mode_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    let items: Vec<ListItem> = ["New Instance", "Modpacks"]
        .into_iter()
        .map(|label| {
            ListItem::new(Line::from(Span::styled(
                label,
                Style::default().fg(theme.text()).add_modifier(Modifier::BOLD),
            )))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(theme.accent())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default().with_selected(Some(state.mode_idx));
    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn render_name_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    let value = state.name_state.value();
    // \u{2588} is the full block char used as a fake blinking cursor
    let line = if value.is_empty() {
        Line::from(vec![
            Span::styled("Instance name...", Style::default().fg(theme.text_dim())),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme.text_dim())
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])
    } else {
        Line::from(vec![
            Span::styled(value, Style::default().fg(theme.text())),
            Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme.text_dim())
                    .add_modifier(Modifier::SLOW_BLINK),
            ),
        ])
    };

    Paragraph::new(line).render(area, buf);
}

fn render_version_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    match &state.versions {
        LoadState::Idle | LoadState::Loading => {
            Paragraph::new("Loading versions...")
                .style(Style::default().fg(theme.text_dim()))
                .render(area, buf);
        }
        LoadState::Error(message) => {
            Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.error()))
                .render(area, buf);
        }
        LoadState::Loaded(_) => {
            let items: Vec<ListItem> = visible_versions(state)
                .into_iter()
                .map(|version| {
                    let suffix = if version.stable {
                        String::new()
                    } else {
                        " (snapshot)".to_string()
                    };
                    ListItem::new(Line::from(Span::styled(
                        format!("{}{}", version.id, suffix),
                        Style::default().fg(theme.text()),
                    )))
                })
                .collect();

            let list = List::new(items)
                .highlight_style(
                    Style::default()
                        .fg(theme.accent())
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            let mut list_state = ListState::default().with_selected(Some(state.version_idx));
            StatefulWidget::render(list, area, buf, &mut list_state);
        }
    }
}

fn render_loader_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    let loaders = [
        ModLoader::Vanilla,
        ModLoader::Fabric,
        ModLoader::Forge,
        ModLoader::NeoForge,
        ModLoader::Quilt,
    ];

    let items: Vec<ListItem> = loaders
        .into_iter()
        .map(|loader| {
            ListItem::new(Line::from(Span::styled(
                loader.to_string(),
                Style::default().fg(theme.text()),
            )))
        })
        .collect();

    let list = List::new(items)
        .highlight_style(
            Style::default()
                .fg(theme.accent())
                .add_modifier(Modifier::BOLD),
        )
        .highlight_symbol("▶ ");

    let mut list_state = ListState::default().with_selected(Some(state.loader_idx));
    StatefulWidget::render(list, area, buf, &mut list_state);
}

fn render_loader_version_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    if state.selected_loader() == ModLoader::Vanilla {
        Paragraph::new("Vanilla has no loader version.")
            .style(Style::default().fg(theme.text_dim()))
            .render(area, buf);
        return;
    }

    match &state.loader_versions {
        LoadState::Idle | LoadState::Loading => {
            Paragraph::new(format!("Loading {} versions...", state.selected_loader()))
                .style(Style::default().fg(theme.text_dim()))
                .render(area, buf);
        }
        LoadState::Error(message) => {
            Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.error()))
                .render(area, buf);
        }
        LoadState::Loaded(versions) => {
            let items: Vec<ListItem> = versions
                .iter()
                .map(|version| {
                    ListItem::new(Line::from(Span::styled(
                        version.clone(),
                        Style::default().fg(theme.text()),
                    )))
                })
                .collect();

            let list = List::new(items)
                .highlight_style(
                    Style::default()
                        .fg(theme.accent())
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            let mut list_state = ListState::default().with_selected(Some(state.loader_version_idx));
            StatefulWidget::render(list, area, buf, &mut list_state);
        }
    }
}

fn render_confirm_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    let theme = THEME.as_ref();
    let game_version = state
        .selected_version()
        .map(|version| version.id.as_str())
        .unwrap_or("<not selected>");
    let loader = state.selected_loader();
    let loader_version = if loader == ModLoader::Vanilla {
        "n/a".to_string()
    } else {
        state
            .selected_loader_version()
            .unwrap_or_else(|| "<not selected>".to_string())
    };

    let label_style = Style::default().fg(theme.text_dim());

    Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Name: ", label_style),
            Span::raw(state.name_state.value()),
        ]),
        Line::from(vec![
            Span::styled("MC: ", label_style),
            Span::raw(game_version),
        ]),
        Line::from(vec![
            Span::styled("Loader: ", label_style),
            Span::raw(loader.to_string()),
        ]),
        Line::from(vec![
            Span::styled("Loader version: ", label_style),
            Span::raw(loader_version),
        ]),
    ])
    .style(Style::default().fg(theme.text()))
    .wrap(Wrap { trim: true })
    .render(area, buf);
}

// the search/version step rendering lives in the shared browse_step module
// (was duplicated here near-verbatim with content_browse's browser). this
// just supplies the copy specific to browsing modpacks.
fn render_modpack_browse_step(
    state: &WizardState,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    font_size: (u16, u16),
) {
    let copy = browse_step::SearchStepCopy {
        // short verb-phrase hint; the keybind reminders (Tab source, Enter
        // search, / search) already live in the footer bar, so repeating
        // them here just made the line wrap.
        placeholder: "search modpacks…".to_string(),
        idle: format!("Install a modpack from {}.", state.modpack_source.label()),
        empty: "No modpacks found for that search.".to_string(),
    };
    browse_step::render_search_step(
        area,
        buf,
        state.modpack_source.label(),
        state.modpack_source.accent(),
        &state.modpack_query,
        state.modpack_query_focused,
        &state.modpack_results,
        state.modpack_idx,
        copy,
        font_size,
        None,
    );
}

fn render_modpack_version_step(state: &WizardState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    browse_step::render_version_step(
        area,
        buf,
        state.modpack_source.accent(),
        &state.modpack_versions,
        state.modpack_version_idx,
        "Loading pack versions...",
        "This pack has no published versions.",
    );
}

fn render_modpack_confirm_step(
    state: &WizardState,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
) {
    let theme = THEME.as_ref();
    let label_style = Style::default().fg(theme.text_dim());

    let pack_title = state.selected_modpack().map(|h| h.title()).unwrap_or("<not selected>");
    let source = state
        .selected_modpack()
        .map(|h| match h {
            ModpackHit::Modrinth(_) => "Modrinth",
            ModpackHit::CurseForge(_) => "CurseForge",
        })
        .unwrap_or("");
    let version = state
        .selected_modpack_version()
        .map(|v| v.label())
        .unwrap_or_else(|| "<not selected>".to_string());

    let name_value = state.modpack_name_state.value();
    let name_line = Line::from(vec![
        Span::styled("Instance name: ", label_style),
        Span::styled(name_value, Style::default().fg(theme.text())),
        Span::styled(
            "\u{2588}",
            Style::default()
                .fg(theme.text_dim())
                .add_modifier(Modifier::SLOW_BLINK),
        ),
    ]);

    Paragraph::new(vec![
        Line::from(vec![
            Span::styled("Pack: ", label_style),
            Span::raw(format!("{pack_title} ({source})")),
        ]),
        Line::from(vec![Span::styled("Version: ", label_style), Span::raw(version)]),
        Line::from(""),
        name_line,
        Line::from(Span::styled(
            "Game version and loader are read from the pack's manifest during install.",
            Style::default().fg(theme.text_dim()),
        )),
    ])
    .wrap(Wrap { trim: true })
    .render(area, buf);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tui::widgets::popups::new_instance::state::ModpackVersionHit;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // WIZARD_STATE is a process-global static; without serialisation, parallel
    // tests would race when each test sets the step and then renders, since
    // render re-acquires the WIZARD_STATE mutex internally. this guard mutex
    // ensures only one wizard snapshot test runs at a time.
    static TEST_SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn reset_wizard_state(step: WizardStep) {
        let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
        *guard = WizardState::default();
        guard.step = step;
    }

    #[test]
    fn new_instance_renders_name_step() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Name is the default step; render touches no network helpers.
        reset_wizard_state(WizardStep::Name);

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    #[test]
    fn new_instance_renders_loader_step() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Loader step is reached after Name; render just paints the hardcoded
        // loader list, no network.
        reset_wizard_state(WizardStep::Loader);

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    // Version step: pre-populate versions as LoadState::Loaded so
    // ensure_versions_loaded short-circuits and never spawns a network task.
    // the three synthetic versions are marked stable=true so they show with
    // show_snapshots=false (the default).
    #[test]
    fn new_instance_renders_version_step() {
        use crate::instance::loader::GameVersion;

        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
            *guard = WizardState::default();
            guard.step = WizardStep::Version;
            guard.versions = LoadState::Loaded(vec![
                GameVersion {
                    id: "1.20.1".into(),
                    stable: true,
                },
                GameVersion {
                    id: "1.19.4".into(),
                    stable: true,
                },
                GameVersion {
                    id: "1.18.2".into(),
                    stable: true,
                },
            ]);
        }

        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    // LoaderVersion step: needs both versions and loader_versions pre-loaded.
    // pick a non-Vanilla loader (loader_idx=2 = Forge) so the step doesn't
    // skip itself to Confirm.
    #[test]
    fn new_instance_renders_loader_version_step() {
        use crate::instance::loader::GameVersion;

        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
            *guard = WizardState::default();
            guard.step = WizardStep::LoaderVersion;
            guard.loader_idx = 2; // Forge
            guard.versions = LoadState::Loaded(vec![GameVersion {
                id: "1.20.1".into(),
                stable: true,
            }]);
            guard.loader_versions =
                LoadState::Loaded(vec!["47.2.0".into(), "47.1.0".into(), "47.0.50".into()]);
        }

        let backend = TestBackend::new(60, 14);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    // Confirm step: paints a summary, no network, no list. requires
    // versions + loader_versions Loaded so selected_*() return Some.
    #[test]
    fn new_instance_renders_confirm_step() {
        use crate::instance::loader::GameVersion;
        use tui_prompts::TextState;

        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
            *guard = WizardState::default();
            guard.step = WizardStep::Confirm;
            guard.loader_idx = 1; // Fabric
            guard.versions = LoadState::Loaded(vec![GameVersion {
                id: "1.20.1".into(),
                stable: true,
            }]);
            guard.loader_versions = LoadState::Loaded(vec!["0.15.0".into()]);
            // TextState exposes only constructors; rebuilding with the
            // desired initial value is the supported path.
            guard.name_state = TextState::new().with_value("MyPack");
        }

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    #[test]
    fn new_instance_renders_mode_step() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Mode is now the default step - just a 2-item list, no network.
        reset_wizard_state(WizardStep::Mode);

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    #[test]
    fn new_instance_renders_modpack_browse_step_idle() {
        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        // Idle state (no search fired yet) - no network helpers touched.
        reset_wizard_state(WizardStep::ModpackBrowse);

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    #[test]
    fn new_instance_renders_modpack_browse_step_with_results() {
        use crate::net::modrinth::SearchHit;

        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
            *guard = WizardState::default();
            guard.step = WizardStep::ModpackBrowse;
            guard.modpack_query_focused = false;
            guard.modpack_results = LoadState::Loaded(vec![ModpackHit::Modrinth(SearchHit {
                project_id: "abc123".into(),
                slug: "example-pack".into(),
                title: "Example Pack".into(),
                description: "A test modpack".into(),
                author: "someone".into(),
                downloads: 42,
                icon_url: None,
                categories: vec![],
                versions: vec![],
                project_type: "modpack".into(),
            })]);
        }

        let backend = TestBackend::new(80, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }

    #[test]
    fn new_instance_renders_modpack_confirm_step() {
        use crate::net::modrinth::{ProjectVersion, SearchHit};
        use tui_prompts::TextState;

        let _serial = TEST_SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        {
            let mut guard = WIZARD_STATE.lock().expect("WIZARD_STATE lock");
            *guard = WizardState::default();
            guard.step = WizardStep::ModpackConfirm;
            guard.modpack_results = LoadState::Loaded(vec![ModpackHit::Modrinth(SearchHit {
                project_id: "abc123".into(),
                slug: "example-pack".into(),
                title: "Example Pack".into(),
                description: "A test modpack".into(),
                author: "someone".into(),
                downloads: 42,
                icon_url: None,
                categories: vec![],
                versions: vec![],
                project_type: "modpack".into(),
            })]);
            guard.modpack_versions = LoadState::Loaded(vec![ModpackVersionHit::Modrinth(
                ProjectVersion {
                    id: "v1".into(),
                    project_id: "abc123".into(),
                    name: "Release 1".into(),
                    version_number: "1.0.0".into(),
                    game_versions: vec!["1.20.1".into()],
                    loaders: vec!["forge".into()],
                    version_type: "release".into(),
                    files: vec![],
                    dependencies: vec![],
                },
            )]);
            guard.modpack_name_state = TextState::new().with_value("Example Pack");
        }

        let backend = TestBackend::new(60, 12);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    FocusedArea::Popup,
                    // headless-safe picker (no terminal query); only used
                    // for font_size in the modpack browse render path.
                    &ratatui_image::picker::Picker::halfblocks(),
                )
            })
            .unwrap();
    }
}
