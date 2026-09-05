// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// reusable incremental search state used across multiple widgets.
// handles case-insensitive filtering and inline match highlighting.

use crossterm::event::{KeyCode, KeyEvent};
use ratatui::{
    style::{Modifier, Style},
    text::{Line, Span},
};

use crate::config::theme::THEME;

/// what a keypress did to the search state. callers use this to apply their
/// own side effect (e.g. resetting list selection to the top when the query
/// changed) instead of re-implementing the search key loop themselves.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchAction {
    /// `/` pressed while not searching: editing started.
    Activated,
    /// Enter: editing stopped, the filter stays active.
    Confirmed,
    /// Esc: editing stopped and the query was cleared.
    Deactivated,
    /// a character was pushed or popped: the query changed.
    Edited,
    /// consumed by search mode but did nothing (any other key while typing).
    Handled,
    /// search mode wasn't active and the key wasn't `/`.
    Unhandled,
}

#[derive(Debug, Default, Clone)]
pub struct SearchState {
    pub query: String,
    pub active: bool,
}

impl SearchState {
    /// the one search key loop. every widget that has a search box routes its
    /// keys through here instead of copy-pasting the Enter/Esc/Backspace/Char
    /// match (that loop used to exist in five places).
    pub fn handle_key(&mut self, key: &KeyEvent) -> SearchAction {
        if !self.active {
            if key.code == KeyCode::Char('/') {
                self.activate();
                return SearchAction::Activated;
            }
            return SearchAction::Unhandled;
        }
        match key.code {
            KeyCode::Enter => {
                self.confirm();
                SearchAction::Confirmed
            }
            KeyCode::Esc => {
                self.deactivate();
                SearchAction::Deactivated
            }
            KeyCode::Backspace => {
                self.pop();
                SearchAction::Edited
            }
            KeyCode::Char(c) => {
                self.push(c);
                SearchAction::Edited
            }
            _ => SearchAction::Handled,
        }
    }

    pub fn activate(&mut self) {
        self.active = true;
    }

    pub fn deactivate(&mut self) {
        self.active = false;
        self.query.clear();
    }

    // exit search mode but keep the filter active so the user can
    // navigate the filtered results
    pub fn confirm(&mut self) {
        self.active = false;
    }

    pub fn push(&mut self, c: char) {
        self.query.push(c);
    }

    pub fn pop(&mut self) {
        self.query.pop();
    }

    pub fn is_empty(&self) -> bool {
        self.query.is_empty()
    }

    pub fn matches(&self, text: &str) -> bool {
        if self.query.is_empty() {
            return true;
        }
        text.to_lowercase().contains(&self.query.to_lowercase())
    }

    // splits a line into spans, bolding+underlining the parts that match
    // the query so they pop out visually
    pub fn highlight_line<'a>(&self, text: &'a str, base_style: Style) -> Line<'a> {
        if self.query.is_empty() {
            return Line::from(Span::styled(text, base_style));
        }

        let query_lower = self.query.to_lowercase();
        let text_lower = text.to_lowercase();
        let mut spans = Vec::new();
        let mut last = 0;

        for (start, _) in text_lower.match_indices(&query_lower) {
            if start > last {
                spans.push(Span::styled(&text[last..start], base_style));
            }
            spans.push(Span::styled(
                &text[start..start + self.query.len()],
                base_style.add_modifier(Modifier::BOLD | Modifier::UNDERLINED),
            ));
            last = start + self.query.len();
        }

        if last < text.len() {
            spans.push(Span::styled(&text[last..], base_style));
        }

        if spans.is_empty() {
            Line::from(Span::styled(text, base_style))
        } else {
            Line::from(spans)
        }
    }

    // renders the "/ query█" indicator in the block title bar
    pub fn title_line(&self) -> Option<Line<'static>> {
        if !self.active && self.query.is_empty() {
            return None;
        }

        let theme = THEME.as_ref();
        let dim = Style::default().fg(theme.text_dim());
        let accent = Style::default()
            .fg(theme.text_dim())
            .add_modifier(Modifier::BOLD);

        let mut spans = vec![
            Span::styled(" / ", dim),
            Span::styled(self.query.clone(), accent),
        ];

        if self.active {
            spans.push(Span::styled("\u{2588}", accent));
        }

        spans.push(Span::raw(" "));

        Some(Line::from(spans).right_aligned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_key_slash_activates() {
        let mut s = SearchState::default();
        let action = s.handle_key(&KeyEvent::new(KeyCode::Char('/'), ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Activated);
        assert!(s.active);
    }

    #[test]
    fn handle_key_edits_query_while_active() {
        let mut s = SearchState::default();
        s.activate();
        let action = s.handle_key(&KeyEvent::new(KeyCode::Char('a'), ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Edited);
        assert_eq!(s.query, "a");
    }

    #[test]
    fn handle_key_enter_confirms_and_esc_deactivates() {
        let mut s = SearchState::default();
        s.activate();
        s.push('a');
        let action = s.handle_key(&KeyEvent::new(KeyCode::Enter, ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Confirmed);
        assert!(!s.active);
        assert_eq!(s.query, "a"); // filter stays active after confirm

        s.activate();
        let action = s.handle_key(&KeyEvent::new(KeyCode::Esc, ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Deactivated);
        assert!(s.query.is_empty());
    }

    #[test]
    fn handle_key_consumes_other_keys_while_active() {
        let mut s = SearchState::default();
        s.activate();
        let action = s.handle_key(&KeyEvent::new(KeyCode::Down, ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Handled);
    }

    #[test]
    fn handle_key_unhandled_when_inactive() {
        let mut s = SearchState::default();
        let action = s.handle_key(&KeyEvent::new(KeyCode::Down, ratatui::crossterm::event::KeyModifiers::NONE));
        assert_eq!(action, SearchAction::Unhandled);
    }

    #[test]
    fn confirm_keeps_query_but_deactivates() {
        let mut s = SearchState::default();
        s.activate();
        s.push('a');
        s.push('b');
        s.confirm();
        assert!(!s.active);
        assert_eq!(s.query, "ab");
        // filter should still match
        assert!(s.matches("abc"));
        assert!(!s.matches("xyz"));
    }

    #[test]
    fn deactivate_clears_query() {
        let mut s = SearchState::default();
        s.activate();
        s.push('x');
        s.deactivate();
        assert!(!s.active);
        assert!(s.query.is_empty());
        // with empty query, everything matches
        assert!(s.matches("anything"));
    }

    #[test]
    fn confirm_then_reactivate_preserves_query() {
        let mut s = SearchState::default();
        s.activate();
        s.push('t');
        s.push('e');
        s.confirm();
        // user presses search key again to edit
        s.activate();
        assert!(s.active);
        assert_eq!(s.query, "te");
    }
}
