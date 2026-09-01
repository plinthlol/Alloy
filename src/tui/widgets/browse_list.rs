// shared row styling for popup "browse" lists (search results, version
// pickers). ratatui's default `List` selection just swaps the foreground
// color, which barely registers on dim themes — the selected row vanishes
// into the rest of the list. the content list (content/list.rs) solves
// this with alternating stripes plus a solid accent left bar on the
// selected row; this gives popups the same treatment without
// tui_widget_list's virtualized machinery, which small (<=40 item) lists
// don't need.
//
// accent color comes from the caller, so a source-branded color (modrinth
// green vs curseforge red in browse_step) drives the highlight instead of
// every source looking identical but for a text label.

use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
    widgets::ListItem,
};
use ratatui_themekit::Theme;

// alternating background for row `index`, independent of selection - gives
// the list a subtle sense of depth even before anything is highlighted.
pub fn stripe_bg(theme: &dyn Theme, index: usize) -> ratatui::style::Color {
    if index % 2 == 0 {
        theme.background()
    } else {
        theme.stripe()
    }
}

// leading marker column: a solid accent bar on every line of the selected
// row (so it reads as one block running the full entry height), otherwise
// a blank space so unselected rows still line up.
fn marker(accent: ratatui::style::Color, is_selected: bool) -> Span<'static> {
    if is_selected {
        Span::styled("\u{258c}", Style::default().fg(accent))
    } else {
        Span::raw(" ")
    }
}

// title/primary line style: bold accent when selected, bold plain text
// otherwise. secondary lines (author, description) stay text_dim()
// regardless — only the title needs to pop. `accent` lets callers brand
// the highlight (modrinth green vs curseforge red) instead of always using
// the generic theme accent.
pub fn title_style(theme: &dyn Theme, accent: ratatui::style::Color, is_selected: bool) -> Style {
    if is_selected {
        Style::default().fg(accent).add_modifier(Modifier::BOLD)
    } else {
        Style::default().fg(theme.text()).add_modifier(Modifier::BOLD)
    }
}

// wraps already-built lines (one row) into a ListItem: prefixes every line
// with the marker column (so multi-line rows stay aligned) and applies the
// stripe background. callers build `lines` (title via `title_style`, rest
// dimmed) and hand them here for the visual treatment.
pub fn row<'a>(
    theme: &dyn Theme,
    accent: ratatui::style::Color,
    index: usize,
    is_selected: bool,
    mut lines: Vec<Line<'a>>,
) -> ListItem<'a> {
    for line in lines.iter_mut() {
        // every line gets the marker (not just the first) so the accent
        // bar runs the full height of the selected row, not just its title.
        let lead = marker(accent, is_selected);
        let mut spans = vec![lead];
        spans.extend(std::mem::take(&mut line.spans));
        *line = Line::from(spans);
    }

    ListItem::new(lines).style(Style::default().bg(stripe_bg(theme, index)))
}
