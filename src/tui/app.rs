// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// app state: holds everything the TUI needs between frames.
// this is basically the "god struct" of the UI. not ideal, but ratatui
// kinda pushes you into this pattern since you need mutable access
// to all the widget states during rendering.

use std::collections::HashMap;

use tachyonfx::Effect;

use super::widgets::{self, instances};
use crate::instance::InstanceManager;

pub struct App {
    pub(super) exit: bool,
    pub(super) focused: FocusedArea,
    pub(super) pre_overlay_focused: FocusedArea,
    pub(super) instances_state: instances::State,
    // the whole tabbed content area (mods/resourcepacks/worlds/screenshots/
    // logs/settings + active tab) as one unit - see ContentArea::tick
    pub(super) content: widgets::content::ContentArea,
    pub(super) account_state: widgets::account::AccountState,
    pub(super) picker: ratatui_image::picker::Picker,
    pub(super) instance_manager: InstanceManager,
    // notifications from background tasks/popups, drained every frame by
    // event.rs's drain_events() -> apply_event() dispatch
    pub(super) ui_rx: tokio::sync::mpsc::UnboundedReceiver<super::events::UiEvent>,
    pub(super) log_overlay_scroll: usize,
    pub(super) log_overlay_max_scroll: usize,
    pub(super) log_overlay_search: widgets::search::SearchState,
    pub(super) log_overlay_scrollbar: ratatui::widgets::ScrollbarState,
    pub(super) throbber_state: throbber_widgets_tui::ThrobberState,
    pub(super) throbber_tick: u8,
    pub(super) error_effects: HashMap<u64, ErrorEffectState>,
    pub(super) pending_editor: Option<std::path::PathBuf>,
    // in-progress instance rename triggered from the content header
    // (k/Up past the top of the active tab's list). mirrors
    // instances_state.renaming but for the Content-focused path.
    pub(super) content_renaming: Option<String>,
    // set when the global settings screen saves changes and the user then
    // closes it -- config.toml is only read once at startup, so applying
    // an edited value means relaunching the process. checked in tui/mod.rs
    // after the event loop exits and the terminal is restored.
    pub(super) restart_requested: bool,
    // which "thing" the JavaSelect popup is currently picking a runtime
    // for, so Pick(path) lands in the right place
    pub(super) java_select_target: JavaSelectTarget,
    pub(super) overlay_was_open: bool,
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum JavaSelectTarget {
    #[default]
    Instance,
    Global,
}

// lifecycle of an error toast animation: slide in -> sit there -> fade out
pub(super) enum ErrorEffectState {
    SlidingIn(Effect, std::time::Instant),
    Idle,
    FadingOut(Effect, std::time::Instant),
}

#[derive(Default, Debug, Clone, Copy, PartialEq)]
pub enum FocusedArea {
    #[default]
    Instances,
    Content,
    Account,
    Overview,
    OverviewExpanded,
    Popup,
    ContentBrowse,
    ErrorPopup,
    ConfirmDelete,
    JavaSelect,
    GlobalSettings,
}

impl App {
    pub fn new(picker: ratatui_image::picker::Picker) -> Self {
        let instances_dir = crate::config::SETTINGS.paths.resolve_instances_dir();
        let meta_dir = crate::config::SETTINGS.paths.resolve_meta_dir();

        let _ = std::fs::create_dir_all(&instances_dir);
        let _ = std::fs::create_dir_all(&meta_dir);

        crate::running::reconcile_orphans();

        let manager = InstanceManager::new(instances_dir, meta_dir);
        let instances = manager.load_all();
        let instances_state = instances::State::with_instances(instances);

        App {
            ui_rx: super::events::init(),
            exit: false,
            focused: FocusedArea::default(),
            pre_overlay_focused: FocusedArea::default(),
            instances_state,
            content: {
                let mut c = widgets::content::ContentArea::default();
                let fs = picker.font_size();
                c.screenshots.font_size = (fs.width, fs.height);
                c
            },
            account_state: widgets::account::AccountState::default(),
            picker,
            instance_manager: manager,
            log_overlay_scroll: 0,
            log_overlay_max_scroll: 0,
            log_overlay_search: widgets::search::SearchState::default(),
            log_overlay_scrollbar: ratatui::widgets::ScrollbarState::default(),
            throbber_state: throbber_widgets_tui::ThrobberState::default(),
            throbber_tick: 0,
            error_effects: HashMap::new(),
            pending_editor: None,
            content_renaming: None,
            restart_requested: false,
            java_select_target: JavaSelectTarget::default(),
            overlay_was_open: false,
        }
    }

    // true when the active content tab's list selection is already at the
    // top (or empty), meaning another k/Up press should open the instance
    // rename field in the content header instead of moving the selection.
    // (the per-tab cache invalidation for renames lives in
    // ContentArea::invalidate)
    pub(super) fn content_at_top(&self) -> bool {
        self.content.at_top()
    }
}
