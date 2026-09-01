// typed channel for background-task/popup → TUI notifications. replaces the
// ad-hoc global result queues event.rs used to drain by hand
// (PENDING_INSTANCES, *_RESULT, PENDING_LAST_PLAYED) with one seam:
// producers call emit(), the run loop calls drain_events(), and
// App::apply_event routes each event. a new handoff is one enum variant +
// one match arm, not a new global and new drain plumbing.

use std::sync::OnceLock;

use tokio::sync::mpsc;

use crate::instance::models::InstanceConfig;
use crate::tui::widgets::popups::content_browse::ContentInstallParams;
use crate::tui::widgets::popups::new_instance::{ModpackInstallParams, WizardParams};

#[derive(Debug)]
pub enum UiEvent {
    /// a background create/install task finished; add it to the sidebar.
    InstanceCreated(InstanceConfig),
    /// the new-instance wizard confirmed the standard flow.
    WizardConfirmed(WizardParams),
    /// the wizard confirmed a modpack install.
    ModpackConfirmed(ModpackInstallParams),
    /// the browse popup picked a file to install into an existing instance.
    ContentInstallConfirmed(ContentInstallParams),
    /// a play session ended (normal exit, manual kill, or orphan reaped).
    LastPlayed(String, chrono::DateTime<chrono::Utc>),
}

static TX: OnceLock<mpsc::UnboundedSender<UiEvent>> = OnceLock::new();

/// called once at startup (App::new). returns the receiver the run loop
/// drains every frame.
pub fn init() -> mpsc::UnboundedReceiver<UiEvent> {
    let (tx, rx) = mpsc::unbounded_channel();
    let _ = TX.set(tx);
    rx
}

/// post an event from anywhere (background task, popup key handler).
/// before init (e.g. in unit tests) events are dropped silently.
pub fn emit(event: UiEvent) {
    if let Some(tx) = TX.get() {
        let _ = tx.send(event);
    }
    crate::tui::request_redraw();
}
