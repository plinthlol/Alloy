// rendering for the content browse popup. same two-step shape as the
// modpack browser — the actual search/version step rendering lives in the
// shared browse_step module now (was duplicated here before), so this file
// only owns the popup frame/keybinds and the copy specific to browsing
// mods/resourcepacks.

use super::state::{BROWSE_STATE, BrowseStep, ContentBrowseState, ContentKind};
use crate::config::theme::THEME;
use crate::tui::widgets::browse_step::{self, SearchStepCopy};
use crate::tui::widgets::popups::base::PopupFrame;
use crate::tui::widgets::popups::description;
use crate::tui::widgets::popups::keybind_line;
use crate::tui::widgets::styled_title;
use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    text::Line,
};

pub fn render(frame: &mut Frame, area: Rect, picker: &ratatui_image::picker::Picker) {
    let snapshot = match BROWSE_STATE.lock() {
        Ok(state) => {
            if !state.open {
                return;
            }
            clone_state(&state)
        }
        Err(e) => {
            tracing::error!("Content browse state lock poisoned: {}", e);
            return;
        }
    };

    let theme = THEME.as_ref();
    let description_open = description::is_open();
    let title = if description_open {
        description::title()
    } else {
        format!("Add {} \u{2014} {}", snapshot.kind.label(), snapshot.instance_name)
    };
    let keybinds = if description_open {
        description::keybinds()
    } else {
        step_keybinds(&snapshot)
    };
    // font_size is Copy, so it can move into the 'static content closure;
    // the Picker itself (and its terminal-query state) can't — and doesn't
    // need to. thumbnails go through the process-wide WEB_ICONS cache,
    // which only needs the picker at decode time (drained each tick from
    // the main event loop).
    let fs = picker.font_size();
    let font_size = (fs.width, fs.height);

    let popup = PopupFrame {
        title: styled_title(&title, false),
        border_color: theme.text_dim(),
        bg: Some(theme.surface()),
        keybinds: Some(keybinds),
        search_line: None,
        content: Box::new(move |popup_area, buf| {
            // the description view replaces the popup's content in place;
            // markdown::render runs after the frame below (needs &mut Frame)
            if description_open {
                return;
            }
            match snapshot.step {
            BrowseStep::Search => render_search_step(&snapshot, popup_area, buf, font_size),
            BrowseStep::Version => render_version_step(&snapshot, popup_area, buf),
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
    // large enough that the 4-line search-result rows (title/meta/desc/spacer
    // plus a 3-row icon) still show a useful page of results; 80% keeps a
    // healthy border of the app visible without cramping the list.
    let w = Constraint::Percentage(80);
    let h = Constraint::Percentage(80);
    let horizontal = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), w, Constraint::Fill(1)])
        .split(frame_area);
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), h, Constraint::Fill(1)])
        .split(horizontal[1]);
    vertical[1]
}

// PopupFrame's content closure needs 'static, so render() clones the
// (cheap) snapshot out from behind the lock rather than holding it across
// the render call — same pattern new_instance's wizard uses.
fn clone_state(state: &ContentBrowseState) -> ContentBrowseState {
    ContentBrowseState {
        open: state.open,
        kind: state.kind,
        instance_name: state.instance_name.clone(),
        dest_dir: state.dest_dir.clone(),
        game_version: state.game_version.clone(),
        loader: state.loader,
        step: state.step,
        source: state.source,
        query: state.query.clone(),
        query_focused: state.query_focused,
        search_generation: state.search_generation,
        last_searched_query: state.last_searched_query.clone(),
        results: state.results.clone(),
        idx: state.idx,
        versions: state.versions.clone(),
        version_idx: state.version_idx,
        pending_install: state.pending_install,
        installed: state.installed.clone(),
    }
}

fn step_keybinds(state: &ContentBrowseState) -> Line<'static> {
    match state.step {
        BrowseStep::Search => {
            if state.query_focused {
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
        BrowseStep::Version => keybind_line(&[("b", " back"), ("Enter", " install")]),
    }
}

fn render_search_step(
    state: &ContentBrowseState,
    area: Rect,
    buf: &mut ratatui::buffer::Buffer,
    font_size: (u16, u16),
) {
    let kind_noun = match state.kind {
        ContentKind::Mod => "mod",
        ContentKind::ResourcePack => "resource pack",
    };
    // resource packs aren't filtered by game version/loader (see
    // ensure_search), so the idle copy shouldn't claim they are — only mods
    // get the "compatible with X Y" framing.
    let idle = match state.kind {
        ContentKind::Mod => format!(
            "Results are filtered to {kind_noun}s compatible with {} {}.",
            state.game_version,
            state.loader,
        ),
        ContentKind::ResourcePack => {
            // packs skip the version/loader facets entirely, so say that
            // rather than echo the mods line above.
            "Resource packs aren't filtered by game version or loader.".to_string()
        }
    };
    let copy = SearchStepCopy {
        // short verb-phrase hint; the keybind reminders (Tab source, Enter
        // search, / search) already live in the footer bar, so repeating
        // them here just made the line wrap.
        placeholder: format!("search {kind_noun}s…"),
        idle,
        empty: "No results for that search.".to_string(),
    };
    browse_step::render_search_step(
        area,
        buf,
        state.source.label(),
        state.source.accent(),
        &state.query,
        state.query_focused,
        &state.results,
        state.idx,
        copy,
        font_size,
        Some(&state.installed),
    );
}

fn render_version_step(state: &ContentBrowseState, area: Rect, buf: &mut ratatui::buffer::Buffer) {
    browse_step::render_version_step(
        area,
        buf,
        state.source.accent(),
        &state.versions,
        state.version_idx,
        "Loading versions...",
        "No versions of this compatible with your instance's game version/loader.",
    );
}
