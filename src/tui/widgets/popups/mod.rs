// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// shared utilities for popup widgets: layout helpers, word wrapping, keybind rendering.
// individual popup types live in their own submodules.

pub mod base;
pub mod confirm;
pub mod content_browse;
pub mod description;
pub mod error;
pub mod global_settings;
pub mod java_select;
pub mod new_instance;

use ratatui::layout::Rect;

// figures out the (width, height) a text block will need after word wrapping.
// used to size popups before rendering so they fit their content snugly.
pub fn word_wrap_size(text: &str, max_inner_width: usize) -> (usize, usize) {
    if text.is_empty() || max_inner_width == 0 {
        return (0, 1);
    }

    let mut lines: usize = 1;
    let mut current_line_len: usize = 0;
    let mut widest_line: usize = 0;

    for word in text.split_whitespace() {
        let word_len = word.len().min(max_inner_width);
        if current_line_len == 0 {
            current_line_len = word_len;
        } else if current_line_len + 1 + word_len <= max_inner_width {
            current_line_len += 1 + word_len;
        } else {
            widest_line = widest_line.max(current_line_len);
            lines += 1;
            current_line_len = word_len;
        }
    }
    widest_line = widest_line.max(current_line_len);

    (widest_line, lines)
}

pub fn top_right_rect(frame: Rect, inner_w: usize, inner_h: usize) -> Rect {
    let popup_w = (inner_w + 2) as u16;
    let popup_h = (inner_h + 2) as u16;
    let popup_w = popup_w.min(frame.width.saturating_sub(4));
    let popup_h = popup_h.min(frame.height.saturating_sub(2));
    let x = frame.width.saturating_sub(popup_w + 2);
    let y = 1u16;
    Rect {
        x,
        y,
        width: popup_w,
        height: popup_h,
    }
}

// \u{23ce} (return symbol) instead of the word "Enter" — shorter, and reads
// as the key it is. normalized centrally so every call site gets it without
// remembering to spell the glyph.
fn key_label(key: &str) -> &str {
    if key == "Enter" {
        "\u{23ce}"
    } else {
        key
    }
}

pub fn keybind_line(binds: &[(&str, &str)]) -> ratatui::text::Line<'static> {
    use crate::config::theme::THEME;
    use ratatui::{
        style::{Modifier, Style},
        text::{Line, Span},
    };
    let theme = THEME.as_ref();
    let key_style = Style::default()
        .fg(theme.accent())
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.text());

    // \u{23ce} (return symbol) instead of the word "Enter" — shorter, and
    // reads as the key it is. normalized centrally so every call site gets
    // it without remembering to spell the glyph.
    let mut spans: Vec<Span<'static>> = Vec::new();
    for (i, (key, label)) in binds.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ", label_style));
        }
        spans.push(Span::styled(format!("[{}]", key_label(key)), key_style));
        if !label.is_empty() {
            spans.push(Span::styled(label.to_string(), label_style));
        }
    }
    Line::from(spans)
}

// same as keybind_line but wraps to multiple rows when the popup is too narrow
// to fit everything on one line
pub fn keybind_lines_wrapped(
    binds: &[(&str, &str)],
    max_width: u16,
) -> Vec<ratatui::text::Line<'static>> {
    use crate::config::theme::THEME;
    use ratatui::{
        style::{Modifier, Style},
        text::{Line, Span},
    };
    let theme = THEME.as_ref();
    let key_style = Style::default()
        .fg(theme.accent())
        .add_modifier(Modifier::BOLD);
    let label_style = Style::default().fg(theme.text());

    // NOTE: this used to split into multiple `Line`s pushed via
    // `block.title_bottom(line)`. that doesn't work — a Block's bottom
    // border is a single row, so multiple title_bottom() calls lay out
    // side-by-side (right-aligned titles packed right-to-left) instead of
    // stacking. when they didn't fit, the leftmost title got clipped from
    // the *left*, dropping the first keybind's "[key]". See
    // https://github.com/ratatui/ratatui/issues/932.
    //
    // so: build one line and stop adding items once they no longer fit,
    // rather than wrapping to a row that would never render on its own.
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut width: usize = 0;

    for (key, label) in binds.iter() {
        let key = if *key == "Enter" { "\u{23ce}" } else { key };
        let sep_w = if spans.is_empty() { 0 } else { 2 };
        let item_w = key.chars().count() + 2 + label.len();
        let needed = sep_w + item_w;

        if width + needed > max_width as usize {
            break;
        }

        if !spans.is_empty() {
            spans.push(Span::styled("  ", label_style));
            width += 2;
        }

        spans.push(Span::styled(format!("[{}]", key), key_style));
        if !label.is_empty() {
            spans.push(Span::styled(label.to_string(), label_style));
        }
        width += item_w;
    }

    if spans.is_empty() {
        Vec::new()
    } else {
        vec![Line::from(spans).right_aligned()]
    }
}
