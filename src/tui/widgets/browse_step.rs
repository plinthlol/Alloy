// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// shared render logic for the two-step (search → version) "browse" UI.
// three flows use this exact shape — modpacks (new_instance wizard) and
// mods/resourcepacks (content_browse popup) — and used to each carry their
// own copy of this rendering, so a fix in one silently didn't apply to the
// other two and the popups drifted apart. centralized here so there's one
// implementation to get right; callers only supply what's genuinely
// different per content kind (placeholder/idle/empty strings).

use crate::config::theme::THEME;
use crate::tui::widgets::browse_list;
use crate::tui::widgets::popups::new_instance::{LoadState, ModpackHit, ModpackVersionHit};
use crate::tui::widgets::web_icon::{self, WEB_ICONS};
use ratatui::{
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::{List, ListItem, ListState, Paragraph, StatefulWidget, Widget, Wrap},
};
use ratatui_image::{Resize, StatefulImage};
use tui_prompts::{State as PromptState, TextState};

// every search-result row is 3 content lines (title, author+downloads,
// description) plus one blank spacer so rows don't run into each other.
// icons are sized/drawn against TEXT_LINES (content only) so they stay
// proportioned to the text instead of stretching into the spacer.
const TEXT_LINES: u16 = 3;
const ROW_LINES: u16 = TEXT_LINES + 1;

/// copy that differs per content kind (mod / resource pack / modpack).
/// everything else about the search step is identical across all three.
pub struct SearchStepCopy {
    pub placeholder: String,
    pub idle: String,
    pub empty: String,
}

#[allow(clippy::too_many_arguments)]
pub fn render_search_step(
    area: Rect,
    buf: &mut Buffer,
    source_label: &str,
    source_accent: ratatui::style::Color,
    query: &TextState<'static>,
    query_focused: bool,
    results: &LoadState<Vec<ModpackHit>>,
    idx: usize,
    copy: SearchStepCopy,
    font_size: (u16, u16),
    // project source_key -> installed filename, for callers that track
    // installed content (content_browse). None for callers that don't
    // (the modpack browser in new_instance, which installs into a
    // not-yet-created instance so "already installed" doesn't apply).
    installed: Option<&std::collections::HashMap<String, String>>,
) {
    let theme = THEME.as_ref();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1)])
        .split(area);

    let query_text = query.value();
    let query_style = if query_focused {
        Style::default().fg(theme.text())
    } else {
        Style::default().fg(theme.text_dim())
    };
    let mut query_line = vec![Span::styled(
        format!("[{source_label}] "),
        Style::default().fg(source_accent).add_modifier(Modifier::BOLD),
    )];
    // the search box shows only while in use — focused (typing) or
    // carrying a query (filtering). browsing the default listing shows
    // just the source chip, so '/' visibly summons the box instead of a
    // placeholder staring at you.
    let show_search_input = query_focused || !query_text.is_empty();
    if show_search_input {
        if !query_text.is_empty() {
            query_line.push(Span::styled(query_text, query_style));
        }
        if query_focused {
            query_line.push(Span::styled(
                "\u{2588}",
                Style::default()
                    .fg(theme.text_dim())
                    .add_modifier(Modifier::SLOW_BLINK),
            ));
        }
    }
    Paragraph::new(Line::from(query_line)).render(chunks[0], buf);

    match results {
        LoadState::Idle => {
            Paragraph::new(copy.idle.clone())
                .style(Style::default().fg(theme.text_dim()))
                .wrap(Wrap { trim: true })
                .render(chunks[1], buf);
        }
        LoadState::Loading => {
            Paragraph::new("Searching...")
                .style(Style::default().fg(theme.text_dim()))
                .render(chunks[1], buf);
        }
        LoadState::Error(message) => {
            Paragraph::new(message.as_str())
                .wrap(Wrap { trim: true })
                .style(Style::default().fg(theme.error()))
                .render(chunks[1], buf);
        }
        LoadState::Loaded(hits) => {
            if hits.is_empty() {
                Paragraph::new(copy.empty.clone())
                    .style(Style::default().fg(theme.text_dim()))
                    .render(chunks[1], buf);
                return;
            }

            // square terminal-cell width for a TEXT_LINES-tall icon (the
            // content lines only, not the spacer below them), and a blank
            // span of that width prepended to a row's content lines so the
            // text lays out to the right of where the thumbnail will be
            // drawn - reserved unconditionally (even for hits with no
            // icon_url, or before their icon has loaded) so titles stay
            // aligned across the whole list instead of jumping around as
            // icons pop in.
            let icon_cols = web_icon::square_icon_columns(TEXT_LINES, font_size);
            let icon_pad = " ".repeat(icon_cols as usize + 1);

            let items: Vec<ListItem> = hits
                .iter()
                .enumerate()
                .map(|(i, hit)| {
                    let is_selected = i == idx;
                    let is_installed = installed.is_some_and(|m| m.contains_key(&hit.source_key()));
                    let mut title_line = vec![
                        Span::raw(icon_pad.clone()),
                        Span::styled(
                            hit.title().to_string(),
                            browse_list::title_style(theme, source_accent, is_selected),
                        ),
                    ];
                    if is_installed {
                        title_line.push(Span::styled(
                            "  Installed",
                            Style::default()
                                .fg(theme.success())
                                .add_modifier(Modifier::BOLD),
                        ));
                    }
                    let lines = vec![
                        Line::from(title_line),
                        Line::from(vec![
                            Span::raw(icon_pad.clone()),
                            Span::styled(
                                format!(
                                    "by {}  \u{b7}  {} downloads",
                                    hit.author(),
                                    crate::util::format_count(hit.downloads())
                                ),
                                Style::default().fg(theme.text_dim()),
                            ),
                        ]),
                        Line::from(vec![
                            Span::raw(icon_pad.clone()),
                            Span::styled(
                                truncate_desc(hit.description(), 120),
                                Style::default().fg(theme.text_dim()),
                            ),
                        ]),
                        // blank spacer: gives each row breathing room so a
                        // full page of dense title+meta+description rows
                        // doesn't read as one unbroken wall of text.
                        Line::from(""),
                    ];
                    browse_list::row(theme, source_accent, i, is_selected, lines)
                })
                .collect();

            let list = List::new(items);
            let mut list_state = ListState::default().with_selected(Some(idx));
            StatefulWidget::render(list, chunks[1], buf, &mut list_state);

            // overlay thumbnails over the padding reserved above, for rows
            // actually scrolled into view (the ListState offset from the
            // render just above says which hit is topmost). only fetch for
            // on-screen rows — same "don't decode what you can't see"
            // principle as request_visible_loads.
            let offset = list_state.offset();
            let icon_area_width = icon_cols.min(chunks[1].width.saturating_sub(2));
            if icon_area_width > 0
                && let Ok(mut cache) = WEB_ICONS.lock()
            {
                for (i, hit) in hits.iter().enumerate().skip(offset) {
                    let Some(url) = hit.icon_url() else { continue };
                    let row_top = chunks[1].y + ((i - offset) as u16) * ROW_LINES;
                    if row_top + TEXT_LINES > chunks[1].y + chunks[1].height {
                        break;
                    }
                    cache.request(url);
                    if let Some(proto) = cache.get(url) {
                        let icon_area = Rect {
                            // +1 so the icon starts past the row's accent
                            // marker column, matching the padding above.
                            x: chunks[1].x + 1,
                            y: row_top,
                            width: icon_area_width,
                            height: TEXT_LINES,
                        };
                        let widget = StatefulImage::default().resize(Resize::Fit(None));
                        StatefulWidget::render(widget, icon_area, buf, proto);
                    }
                }
            }
        }
    }
}

pub fn render_version_step(
    area: Rect,
    buf: &mut Buffer,
    source_accent: ratatui::style::Color,
    versions: &LoadState<Vec<ModpackVersionHit>>,
    version_idx: usize,
    loading_text: &str,
    empty_text: &str,
) {
    let theme = THEME.as_ref();
    match versions {
        LoadState::Idle | LoadState::Loading => {
            Paragraph::new(loading_text)
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
            if versions.is_empty() {
                Paragraph::new(empty_text)
                    .wrap(Wrap { trim: true })
                    .style(Style::default().fg(theme.text_dim()))
                    .render(area, buf);
                return;
            }

            let items: Vec<ListItem> = versions
                .iter()
                .enumerate()
                .map(|(i, v)| {
                    let is_selected = i == version_idx;
                    // big text is the Minecraft version (ModpackVersionHit::label);
                    // the release's own name and channel live on the dim line.
                    let mut meta = vec![Span::styled(
                        v.version_name(),
                        Style::default().fg(theme.text_dim()),
                    )];
                    if let Some(channel) = v.channel() {
                        meta.push(Span::styled(
                            " \u{b7} ",
                            Style::default().fg(theme.text_dim()),
                        ));
                        meta.push(Span::styled(
                            channel,
                            Style::default().fg(theme.warning()).add_modifier(Modifier::BOLD),
                        ));
                    }
                    meta.push(Span::styled(
                        " \u{b7} ",
                        Style::default().fg(theme.text_dim()),
                    ));
                    meta.push(Span::styled(
                        v.loaders(),
                        Style::default().fg(theme.text_dim()),
                    ));
                    let lines = vec![
                        Line::from(Span::styled(
                            v.label(),
                            browse_list::title_style(theme, source_accent, is_selected),
                        )),
                        Line::from(meta),
                    ];
                    browse_list::row(theme, source_accent, i, is_selected, lines)
                })
                .collect();

            let list = List::new(items);
            let mut list_state = ListState::default().with_selected(Some(version_idx));
            StatefulWidget::render(list, area, buf, &mut list_state);
        }
    }
}

fn truncate_desc(s: &str, max_chars: usize) -> String {
    let len = s.chars().count();
    if len <= max_chars {
        return s.to_string();
    }
    let clipped: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    format!("{clipped}\u{2026}")
}
