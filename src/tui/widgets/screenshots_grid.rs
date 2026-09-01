// responsive grid of screenshot thumbnails rendered directly in the terminal.
// images load lazily on background threads as they scroll into view,
// and get converted to terminal graphics via ratatui-image protocols.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::{Arc, Mutex};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    Frame,
    layout::{Constraint, Layout, Rect},
    style::{Modifier, Style},
    text::Span,
    widgets::{Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState},
};
use ratatui_image::{Resize, StatefulImage, protocol::StatefulProtocol};
use throbber_widgets_tui::{Throbber, ThrobberState};

use crate::config::theme::THEME;
use crate::instance::screenshots::ScreenshotEntry;

// grid cell sizing constraints in terminal columns.
// the grid auto-fits columns within these bounds depending on terminal width
const TARGET_CELL_WIDTH: u16 = 34;
const MIN_CELL_WIDTH: u16 = 24;
const MAX_CELL_WIDTH: u16 = 52;
const NAME_ROW_HEIGHT: u16 = 1;
const GAP: u16 = 1;

type PendingScreenshots = Arc<Mutex<Option<(String, Vec<ScreenshotEntry>)>>>;

pub struct ScreenshotsState {
    pub entries: Vec<ScreenshotEntry>,
    protocols: HashMap<usize, StatefulProtocol>,
    // entries whose image::open() came back Err (corrupt file, unsupported
    // encoding, permissions, ...). previously a failed decode just left the
    // cell permanently blank with no indication anything went wrong, which
    // looks identical to "images aren't rendering" in general — now these
    // get a visible placeholder instead of silence.
    failed: HashSet<usize>,
    requested: HashSet<usize>,
    pub selected: usize,
    pub scroll_row: usize,
    pub loaded_for: Option<String>,
    pub loading: bool,
    cols: usize,
    visible_rows: usize,
    pub scrollbar_state: ScrollbarState,
    pub search: super::search::SearchState,
    pub font_size: (u16, u16),
    pending_entries: PendingScreenshots,
    // None means the decode failed for that index (see `failed` above)
    // rather than the load simply not having landed yet.
    pending_images: Arc<Mutex<Vec<(usize, Option<image::DynamicImage>)>>>,
    // the screenshots dir for the currently loaded instance, so ctrl+o can
    // open (and create, if missing) it even when there's nothing selected.
    dir: Option<std::path::PathBuf>,
}

impl Default for ScreenshotsState {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            protocols: HashMap::new(),
            failed: HashSet::new(),
            requested: HashSet::new(),
            selected: 0,
            scroll_row: 0,
            loaded_for: None,
            loading: false,
            cols: 3,
            visible_rows: 2,
            scrollbar_state: ScrollbarState::default(),
            search: super::search::SearchState::default(),
            font_size: (8, 16),
            pending_entries: Arc::new(Mutex::new(None)),
            pending_images: Arc::new(Mutex::new(Vec::new())),
            dir: None,
        }
    }
}

impl ScreenshotsState {
    // drop loaded state tied to `name` so a rename forces a fresh scan.
    pub fn invalidate(&mut self, name: &str) {
        if self.loaded_for.as_deref() == Some(name) {
            self.loaded_for = None;
        }
    }

    // true when the selection is already in the top row (or the grid is
    // empty). used to trigger instance rename from the content header when
    // the user keeps pressing k/Up past the top.
    pub fn is_at_top(&self) -> bool {
        self.entries.is_empty() || self.selected < self.cols.max(1)
    }

    pub fn start_load(&mut self, instances_dir: &Path, instance_name: &str) {
        self.loading = true;
        self.loaded_for = Some(instance_name.to_string());
        self.entries.clear();
        self.protocols.clear();
        self.failed.clear();
        self.requested.clear();
        self.selected = 0;
        self.scroll_row = 0;
        self.dir = Some(crate::instance::screenshots::screenshots_dir(
            instances_dir,
            instance_name,
        ));

        let dir = instances_dir.to_path_buf();
        let tag = instance_name.to_string();
        let pending = self.pending_entries.clone();

        tokio::spawn(async move {
            let scan_dir = dir.clone();
            let scan_name = tag.clone();
            let entries = tokio::task::spawn_blocking(move || {
                crate::instance::screenshots::scan_screenshots(&scan_dir, &scan_name)
            })
            .await
            .unwrap_or_default();

            if let Ok(mut slot) = pending.lock() {
                *slot = Some((tag, entries));
                crate::tui::request_redraw();
            }
        });
    }

    pub fn drain_pending_entries(&mut self) {
        let taken = match self.pending_entries.lock() {
            Ok(mut slot) => slot.take(),
            _ => None,
        };

        if let Some((instance_name, entries)) = taken
            && self.loaded_for.as_deref() == Some(&instance_name)
        {
            self.entries = entries;
            self.loading = false;
            self.selected = 0;
            self.scroll_row = 0;
            // the whole grid is new; stale failure flags belong to indices
            // that may now mean something else entirely.
            self.failed.clear();
        }
    }

    pub fn take_pending_images(&mut self) -> Vec<(usize, Option<image::DynamicImage>)> {
        match self.pending_images.lock() {
            Ok(mut slot) => std::mem::take(&mut *slot),
            _ => Vec::new(),
        }
    }

    pub fn set_protocol(&mut self, idx: usize, proto: StatefulProtocol) {
        self.failed.remove(&idx);
        self.protocols.insert(idx, proto);
    }

    // marks a thumbnail as having failed to decode (corrupt file,
    // unsupported encoding, permissions, ...) so the grid can show a
    // placeholder there instead of leaving the cell silently blank forever.
    pub fn mark_failed(&mut self, idx: usize) {
        self.protocols.remove(&idx);
        self.failed.insert(idx);
    }

    // only load images that are currently visible (or about to be).
    // no point decoding a 4K screenshot the user can't even see yet
    pub fn request_visible_loads(&mut self) {
        if self.entries.is_empty() {
            return;
        }

        let first = self.scroll_row * self.cols;
        let last = ((self.scroll_row + self.visible_rows + 1) * self.cols).min(self.entries.len());

        // screenshots can be 4K (≈33MB of RGBA at full res) but the grid
        // shows them a few cells wide — decoding at full res and letting
        // the protocol re-scale every frame wastes almost all of that.
        // downscale once to ~2x the target cell's pixel size: the protocol
        // resize from there is cheap, and the 2x headroom keeps a
        // large-cell terminal from looking soft. thumbnail() is a fast,
        // aspect-preserving box filter.
        let fw = self.font_size.0.max(1) as u32;
        let max_w = TARGET_CELL_WIDTH as u32 * fw * 2;

        for idx in first..last {
            if !self.protocols.contains_key(&idx) && self.requested.insert(idx) {
                let entry = &self.entries[idx];
                let path = entry.path.clone();
                // max_h proportional to the file's own dimensions so
                // portrait and landscape shots both bind at the same scale
                // inside the max_w box.
                let (ew, eh) = (entry.width.max(1), entry.height.max(1));
                let max_h = (max_w * eh / ew).max(1);
                let pending = self.pending_images.clone();

                tokio::spawn(async move {
                    let load_path = path.clone();
                    let img = tokio::task::spawn_blocking(move || {
                        match image::open(&load_path) {
                            Ok(img) => Some(img.thumbnail(max_w, max_h)),
                            Err(e) => {
                                tracing::warn!(
                                    "Failed to decode screenshot {}: {}",
                                    load_path.display(),
                                    e
                                );
                                None
                            }
                        }
                    })
                    .await
                    .unwrap_or(None);

                    // push either way — a decode failure is still a
                    // result, and the main thread needs it to mark the
                    // cell as failed instead of leaving it blank forever
                    // (see ScreenshotsState::mark_failed).
                    if let Ok(mut slot) = pending.lock() {
                        slot.push((idx, img));
                        crate::tui::request_redraw();
                    }
                });
            }
        }
    }

    fn ensure_visible(&mut self) {
        let row = self.selected.checked_div(self.cols).unwrap_or(0);

        if row < self.scroll_row {
            self.scroll_row = row;
        } else if row >= self.scroll_row + self.visible_rows {
            self.scroll_row = row.saturating_sub(self.visible_rows - 1);
        }

        let total = self.total_rows().saturating_sub(1);
        self.scrollbar_state = ScrollbarState::new(total).position(self.scroll_row);
    }

    fn total_rows(&self) -> usize {
        if self.cols == 0 {
            return 0;
        }
        self.entries.len().div_ceil(self.cols)
    }

    pub fn pending_delete(
        &self,
    ) -> Option<crate::tui::widgets::content::list::PendingContentDelete> {
        let entry = self.entries.get(self.selected)?;
        Some(crate::tui::widgets::content::list::PendingContentDelete {
            name: entry.name.clone(),
            path: entry.path.clone(),
        })
    }

    pub fn remove_path(&mut self, path: &Path) {
        self.entries.retain(|entry| entry.path != path);
        self.protocols.clear();
        self.failed.clear();
        self.requested.clear();

        if self.entries.is_empty() {
            self.selected = 0;
            self.scroll_row = 0;
        } else {
            self.selected = self.selected.min(self.entries.len().saturating_sub(1));
            self.ensure_visible();
        }
    }
}

pub fn handle_key(key_event: &KeyEvent, state: &mut ScreenshotsState) -> bool {
    use super::search::SearchAction;

    if state.search.active {
        match state.search.handle_key(key_event) {
            SearchAction::Edited | SearchAction::Confirmed | SearchAction::Deactivated => {
                state.selected = 0;
            }
            // '/' while already searching and unrelated keys (Handled) don't
            // move the selection. Activated is impossible in this branch.
            _ => {}
        }
        return true;
    }

    let filtered: Vec<usize> = state
        .entries
        .iter()
        .enumerate()
        .filter(|(_, e)| state.search.matches(&e.name))
        .map(|(i, _)| i)
        .collect();
    let count = filtered.len();
    if count == 0 {
        if key_event.code == KeyCode::Char('/') {
            state.search.activate();
            return true;
        }
        return false;
    }
    let cols = state.cols.max(1);

    match key_event.code {
        KeyCode::Char('/') => {
            state.search.activate();
            state.selected = 0;
            true
        }
        // ctrl+o opens the containing folder. (was shift+enter, but most
        // terminals can't report that distinctly without kitty keyboard
        // protocol support, so ctrl+o — it works everywhere)
        KeyCode::Char('o') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
            let dir = state
                .entries
                .get(state.selected)
                .and_then(|entry| entry.path.parent())
                .map(|p| p.to_path_buf())
                .or_else(|| state.dir.clone());
            if let Some(dir) = dir {
                if let Err(e) = std::fs::create_dir_all(&dir) {
                    tracing::error!("Failed to create directory {}: {}", dir.display(), e);
                } else if let Err(e) = open::that_detached(&dir) {
                    tracing::error!("Failed to open directory: {}", e);
                }
            }
            true
        }
        KeyCode::Enter => {
            if let Some(entry) = state.entries.get(state.selected)
                && let Err(e) = open::that_detached(&entry.path)
            {
                tracing::error!("Failed to open file: {}", e);
            }
            true
        }
        KeyCode::Char('L') | KeyCode::Right
            if key_event.modifiers.contains(KeyModifiers::SHIFT) =>
        {
            if state.selected + 1 < count {
                state.selected += 1;
                state.ensure_visible();
            }
            true
        }
        KeyCode::Char('H') | KeyCode::Left if key_event.modifiers.contains(KeyModifiers::SHIFT) => {
            state.selected = state.selected.saturating_sub(1);
            state.ensure_visible();
            true
        }
        KeyCode::Char('J') | KeyCode::Down if key_event.modifiers.contains(KeyModifiers::SHIFT) => {
            let next = state.selected + cols;
            if next < count {
                state.selected = next;
            }
            state.ensure_visible();
            true
        }
        KeyCode::Char('K') | KeyCode::Up if key_event.modifiers.contains(KeyModifiers::SHIFT) => {
            state.selected = state.selected.saturating_sub(cols);
            state.ensure_visible();
            true
        }
        _ => false,
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut ScreenshotsState,
    is_focused: bool,
    throbber_state: &mut ThrobberState,
) {
    let theme = THEME.as_ref();
    if state.loading {
        frame.render_widget(
            Paragraph::new("Loading screenshots...").style(Style::default().fg(theme.text_dim())),
            area,
        );
        return;
    }

    if state.entries.is_empty() {
        frame.render_widget(
            Paragraph::new("No screenshots.").style(Style::default().fg(theme.text_dim())),
            area,
        );
        return;
    }

    let min_cols = (area.width / MAX_CELL_WIDTH).max(1) as usize;
    let max_cols = (area.width / MIN_CELL_WIDTH).max(1) as usize;
    let target_cols = (area.width / TARGET_CELL_WIDTH).max(1) as usize;
    let cols = target_cols.clamp(min_cols, max_cols);
    let cell_width = area.width / cols as u16;

    // figure out how tall each thumbnail should be in terminal rows.
    // need the font's pixel aspect ratio to keep screenshots from
    // looking stretched since terminal cells aren't square
    let (img_w, img_h) = state
        .entries
        .first()
        .map(|e| (e.width, e.height))
        .unwrap_or((1920, 1080));
    let (fw, fh) = (
        state.font_size.0.max(1) as u32,
        state.font_size.1.max(1) as u32,
    );
    let img_rows = (cell_width as u32 * fw * img_h / (fh * img_w)).max(2) as u16;
    let cell_height = img_rows + NAME_ROW_HEIGHT + GAP;
    let visible_rows = (area.height / cell_height).max(1) as usize;

    state.cols = cols;
    state.visible_rows = visible_rows;
    state.ensure_visible();

    for vr in 0..visible_rows {
        for vc in 0..cols {
            let idx = (state.scroll_row + vr) * cols + vc;
            if idx >= state.entries.len() {
                break;
            }

            let raw_x = area.x + vc as u16 * cell_width;
            let raw_y = area.y + vr as u16 * cell_height;
            let raw_w = cell_width.min(area.x + area.width - raw_x);
            let raw_h = cell_height.min(area.y + area.height - raw_y);

            let cell = Rect {
                x: raw_x,
                y: raw_y,
                width: raw_w.saturating_sub(GAP),
                height: raw_h.saturating_sub(GAP),
            };

            if cell.height < 2 || cell.width < 4 {
                continue;
            }

            let is_selected = is_focused && idx == state.selected;

            let [img_area, name_area] =
                Layout::vertical([Constraint::Min(0), Constraint::Length(NAME_ROW_HEIGHT)])
                    .areas(cell);

            if state.failed.contains(&idx) {
                frame.render_widget(
                    Paragraph::new(Span::styled(
                        "\u{26a0} failed to load",
                        Style::default().fg(theme.warning()),
                    ))
                    .alignment(ratatui::layout::Alignment::Center),
                    img_area,
                );
            } else if let Some(proto) = state.protocols.get_mut(&idx) {
                // Always re-render. Sixel/Kitty/iTerm2 graphics are drawn
                // out-of-band (outside ratatui's cell buffer), so anything
                // that later overwrites this screen region — a popup
                // opening/closing over the grid, the log overlay, a resize —
                // erases the graphic without ratatui's diffing ever knowing,
                // since as far as the buffer is concerned nothing changed.
                // Previously this skipped re-encoding when `img_area`
                // matched last frame's rect (a real perf win, since
                // persistent protocols don't need to be re-sent just to
                // stay put), but that's exactly the case that goes stale
                // silently: the thumbnail flashes in once, something else
                // repaints over it, and it never comes back because we
                // never noticed the rect "hadn't changed". Trade the
                // micro-optimization for a thumbnail that reliably stays
                // on screen.
                let widget: StatefulImage<StatefulProtocol> =
                    StatefulImage::default().resize(Resize::Fit(None));
                frame.render_stateful_widget(widget, img_area, proto);
            } else if state.requested.contains(&idx) {
                // decode requested but not back yet - screenshots can be
                // multi-megabyte 4K PNGs, so this can take a visible moment;
                // show a spinner instead of leaving the cell looking dead.
                let throbber = Throbber::default()
                    .label("decoding")
                    .style(Style::default().fg(theme.text_dim()))
                    .throbber_style(
                        Style::default()
                            .fg(theme.accent())
                            .add_modifier(Modifier::BOLD),
                    )
                    .throbber_set(throbber_widgets_tui::BRAILLE_EIGHT_DOUBLE)
                    .use_type(throbber_widgets_tui::WhichUse::Spin);
                let [_, spinner_area, _] = Layout::vertical([
                    Constraint::Min(0),
                    Constraint::Length(1),
                    Constraint::Min(0),
                ])
                .areas(img_area);
                frame.render_stateful_widget(throbber, spinner_area, throbber_state);
            }

            let name = &state.entries[idx].name;
            let name_style = if is_selected {
                Style::default()
                    .fg(theme.accent())
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(theme.text_dim())
            };

            let truncated = if name.len() > cell_width as usize {
                &name[..cell_width as usize]
            } else {
                name
            };
            frame.render_widget(
                Paragraph::new(Span::styled(truncated, name_style)),
                name_area,
            );
        }
    }

    let scrollbar_area = Rect {
        x: area.x + area.width.saturating_sub(0),
        y: area.y + 1,
        width: 1,
        height: area.height.saturating_sub(2),
    };
    frame.render_stateful_widget(
        Scrollbar::default()
            .orientation(ScrollbarOrientation::VerticalRight)
            .begin_symbol(Some("\u{25b2}"))
            .style(
                Style::default()
                    .fg(theme.text_dim())
                    .add_modifier(Modifier::BOLD),
            )
            .thumb_symbol("\u{2551}")
            .track_symbol(Some(""))
            .end_symbol(Some("\u{25bc}")),
        scrollbar_area,
        &mut state.scrollbar_state,
    );
}
