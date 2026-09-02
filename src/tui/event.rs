use color_eyre::eyre::Context;
use crossterm::event::{self, Event};
use ratatui::crossterm::event::KeyEventKind;
use std::time::Duration;

use super::Tui;
use super::app::App;
use super::widgets::{self, popups::content_browse, popups::new_instance};
use crate::instance::InstanceManager;
use crate::tui::error_buffer;
use crate::tui::progress;

impl App {
    /// main loop: poll async results and input at ~60Hz, drawing only when state changes
    pub async fn run(&mut self, terminal: &mut Tui) -> color_eyre::Result<()> {
        let mut last_draw = std::time::Instant::now()
            .checked_sub(Duration::from_secs(1))
            .unwrap_or_else(std::time::Instant::now);
        while !self.exit {
            let redraw_requested = super::take_redraw_request();

            self.dismiss_expired_errors();

            // every content tab's streaming/watcher/image queues drain in
            // one call (ContentArea::tick). reaping runs first so the
            // LastPlayed events it emits drain and persist in this frame.
            crate::running::reap_dead_orphans();
            self.content.tick(&self.picker);

            // one typed channel carries every background-task/popup
            // notification into the loop.
            self.drain_events();
            self.account_state.drain_auth_result();
            widgets::account::drain_device_code(&mut self.account_state);
            // web-fetched thumbnails decode on background tasks
            // (WebIconCache::request); this turns freshly-decoded ones into
            // terminal protocols, which must happen on the main thread.
            if let Ok(mut icons) = widgets::web_icon::WEB_ICONS.lock() {
                icons.drain(&self.picker);
            }
            let progress_active = progress::is_active();
            let spinner_active = progress_active || crate::running::has_active();
            if spinner_active {
                // only advance the spinner every 8 ticks to keep it readable
                self.throbber_tick = self.throbber_tick.wrapping_add(1);
                if self.throbber_tick.is_multiple_of(8) {
                    self.throbber_state.calc_next();
                }
            }

            let input_changed = self.handle_events().wrap_err("handle events failed")?;
            let continuously_animated = spinner_active || error_buffer::has_errors();
            let safety_refresh = last_draw.elapsed() >= Duration::from_secs(1);
            if input_changed || continuously_animated || safety_refresh || redraw_requested {
                terminal.draw(|frame| self.render_frame(frame))?;
                last_draw = std::time::Instant::now();
            }

            if let Some(path) = self.pending_editor.take()
                && Self::run_editor(terminal, &path)
            {
                self.reload_edited_config(&path);
            }
        }
        Ok(())
    }

    // polls for input with a 16ms timeout (~60fps). press + repeat are both
    // handled — dropping repeat made held-down vim keys/arrow keys feel
    // glitchy (one step per physical mash instead of a smooth scroll).
    // releases are still ignored thanks to the enhanced keyboard protocol.
    fn handle_events(&mut self) -> color_eyre::Result<bool> {
        match crossterm::event::poll(Duration::from_millis(16)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(key_event))
                    if matches!(
                        key_event.kind,
                        KeyEventKind::Press | KeyEventKind::Repeat
                    ) =>
                {
                    self.handle_key_event(key_event)
                        .wrap_err_with(|| format!("handling key event failed:\n{key_event:#?}"))?;
                    Ok(true)
                }
                Ok(_) => Ok(true),
                Err(e) => {
                    tracing::error!("Event read error: {}", e);
                    Ok(false)
                }
            },
            Ok(false) => Ok(false),
            Err(e) => {
                tracing::error!("Event poll error: {}", e);
                Ok(false)
            }
        }
    }

    fn spawn_create(&self, params: new_instance::WizardParams) {
        let instances_dir = self.instance_manager.instances_dir.clone();
        let meta_dir = crate::config::SETTINGS.paths.resolve_meta_dir();

        tokio::spawn(async move {
            progress::set_action(format!("Creating instance '{}'...", params.name));
            progress::set_sub_action(format!("{} {}", params.game_version, params.loader));

            let manager = InstanceManager::new(instances_dir, meta_dir);
            match manager
                .create(
                    &params.name,
                    &params.game_version,
                    params.loader,
                    params.loader_version.as_deref(),
                )
                .await
            {
                Ok(config) => {
                    super::events::emit(super::events::UiEvent::InstanceCreated(config));
                }
                Err(e) => {
                    progress::clear();
                    error_buffer::push_error(error_buffer::ErrorEvent {
                        id: 0,
                        level: tracing::Level::ERROR,
                        message: format!("Failed to create instance '{}': {e}", params.name),
                        pushed_at: std::time::Instant::now(),
                    });
                }
            }
        });
    }

    // like spawn_create, but the modpack path: download the pack, read its
    // manifest for game_version/loader (unknown until then), create the base
    // instance, then layer mods + overrides on top. which catalog it came
    // from changes the flow: Modrinth packs are self-contained .mrpack
    // files, CurseForge packs resolve their manifest's project/file id
    // pairs through the API as a separate step.
    fn spawn_install_modpack(&self, params: new_instance::ModpackInstallParams) {
        let instances_dir = self.instance_manager.instances_dir.clone();
        let meta_dir = crate::config::SETTINGS.paths.resolve_meta_dir();

        tokio::spawn(async move {
            progress::set_action(format!("Installing modpack '{}'...", params.name));

            let manager = InstanceManager::new(instances_dir, meta_dir);
            let install_name = params.name.clone();
            let result = match params.source {
                new_instance::ModpackInstallSource::Modrinth(version) => {
                    crate::instance::modpack::install_from_modrinth(
                        &manager,
                        &params.name,
                        &version,
                        |status| progress::set_sub_action(status.to_string()),
                    )
                    .await
                }
                new_instance::ModpackInstallSource::CurseForge { file } => {
                    let api_key = crate::config::SETTINGS
                        .curseforge
                        .effective_api_key()
                        .unwrap_or("")
                        .to_string();
                    crate::instance::modpack::install_from_curseforge(
                        &manager,
                        &params.name,
                        &api_key,
                        &file,
                        |status| progress::set_sub_action(status.to_string()),
                    )
                    .await
                }
            };

            match result {
                Ok(config) => {
                    super::events::emit(super::events::UiEvent::InstanceCreated(config));
                }
                Err(e) => {
                    progress::clear();
                    error_buffer::push_error(error_buffer::ErrorEvent {
                        id: 0,
                        level: tracing::Level::ERROR,
                        message: format!("Failed to install modpack '{install_name}': {e}"),
                        pushed_at: std::time::Instant::now(),
                    });
                }
            }
        });
    }

    // downloads a single mod/resourcepack file from the content browse
    // popup straight into the instance's dir. no creation or manifest
    // processing (unlike spawn_install_modpack) — the dir watcher
    // (ContentListState::watch_dir) picks up the new file on its own.
    fn spawn_install_content(&self, params: content_browse::ContentInstallParams) {
        tokio::spawn(async move {
            let client = crate::net::HttpClient::shared();
            let dest_dir = params.dest_dir;

            if let Err(e) = std::fs::create_dir_all(&dest_dir) {
                progress::clear();
                error_buffer::push_error(error_buffer::ErrorEvent {
                    id: 0,
                    level: tracing::Level::ERROR,
                    message: format!("Failed to create {}: {}", dest_dir.display(), e),
                    pushed_at: std::time::Instant::now(),
                });
                return;
            }

            progress::set_action("Downloading...");

            let result: Result<String, String> = match params.source {
                content_browse::ContentInstallSource::Modrinth(version) => {
                    let file = version
                        .files
                        .iter()
                        .find(|f| f.primary)
                        .or_else(|| version.files.first())
                        .cloned();
                    match file {
                        None => Err("Version has no downloadable files".to_string()),
                        Some(file) => {
                            let dest = dest_dir.join(&file.filename);
                            progress::set_sub_action(file.filename.clone());
                            crate::net::modrinth::download_primary_file(&client, &version, &dest, |cur, total| {
                                progress::set_progress(cur, total);
                            })
                            .await
                            .map(|_| file.filename)
                            .map_err(|e| e.to_string())
                        }
                    }
                }
                content_browse::ContentInstallSource::CurseForge { file } => {
                    let dest = dest_dir.join(&file.file_name);
                    progress::set_sub_action(file.file_name.clone());
                    match crate::net::curseforge::download_mod_file(&client, &file, &dest, |cur, total| {
                        progress::set_progress(cur, total);
                    })
                    .await
                    {
                        Ok(true) => Ok(file.file_name),
                        Ok(false) => Err(format!(
                            "'{}' has third-party downloads disabled by its author - grab it manually from CurseForge",
                            file.display_name
                        )),
                        Err(e) => Err(e.to_string()),
                    }
                }
            };

            progress::clear();
            match result {
                Ok(file_name) => {
                    // record which project this file belongs to, and if a
                    // different file was previously installed for the same
                    // project (e.g. reinstalling a mod at a newer version
                    // whose filename changed), remove it now that the new
                    // one has downloaded successfully - otherwise both
                    // copies would sit in the folder at once.
                    if !params.key.is_empty() {
                        let old_file = crate::instance::content::installed_meta::record(
                            &dest_dir,
                            &params.key,
                            &file_name,
                        );
                        // resource packs don't replace an older installed
                        // version of the same project on reinstall - both
                        // copies are kept on disk. mods still get the
                        // superseded-file cleanup below.
                        if params.kind != content_browse::ContentKind::ResourcePack {
                            if let Some(old_file) = old_file {
                                // the old file may currently be disabled, which
                                // renames it on disk with a trailing suffix -
                                // try the plain name first, then that variant.
                                let candidates = [
                                    dest_dir.join(&old_file),
                                    dest_dir.join(format!("{old_file}.disabled")),
                                ];
                                for old_path in candidates {
                                    if old_path.exists()
                                        && let Err(e) = std::fs::remove_file(&old_path)
                                    {
                                        tracing::warn!(
                                            "Failed to remove superseded file {}: {}",
                                            old_path.display(),
                                            e
                                        );
                                    }
                                }
                            }
                        }
                    }
                    // the browse popup now stays open after an install (see
                    // content_browse/state.rs), so a success toast is the
                    // only feedback that the download actually finished.
                    error_buffer::push_error(error_buffer::ErrorEvent {
                        id: 0,
                        level: tracing::Level::INFO,
                        message: format!("Installed '{file_name}'"),
                        pushed_at: std::time::Instant::now(),
                    });
                }
                Err(e) => {
                    error_buffer::push_error(error_buffer::ErrorEvent {
                        id: 0,
                        level: tracing::Level::ERROR,
                        message: format!("Install failed: {e}"),
                        pushed_at: std::time::Instant::now(),
                    });
                }
            }
            crate::tui::request_redraw();
        });
    }

    // spawns $EDITOR/$VISUAL to edit a file. terminal editors (vim, nano,
    // ...) need us to leave the alternate screen first or they fight with
    // ratatui for the terminal; GUI editors just get spawned detached.
    fn run_editor(terminal: &mut ratatui::DefaultTerminal, path: &std::path::Path) -> bool {
        use ratatui::crossterm::{
            ExecutableCommand,
            terminal::{
                EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
            },
        };
        use std::io::stdout;

        let default_editor = if cfg!(windows) { "notepad" } else { "vi" };
        let editor = std::env::var("EDITOR")
            .or_else(|_| std::env::var("VISUAL"))
            .unwrap_or_else(|_| default_editor.to_owned());

        let editor_name = std::path::Path::new(&editor)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or(&editor);
        let is_tui_editor = matches!(
            editor_name,
            "vi" | "vim"
                | "nvim"
                | "neovim"
                | "nano"
                | "micro"
                | "helix"
                | "hx"
                | "emacs"
                | "ne"
                | "joe"
                | "mcedit"
        );

        if is_tui_editor {
            let _ = stdout().execute(LeaveAlternateScreen);
            let _ = disable_raw_mode();

            let result = std::process::Command::new(&editor)
                .arg(path)
                .stdin(std::process::Stdio::inherit())
                .stdout(std::process::Stdio::inherit())
                .stderr(std::process::Stdio::inherit())
                .status();

            let _ = stdout().execute(EnterAlternateScreen);
            let _ = enable_raw_mode();
            let _ = terminal.clear();

            if let Err(e) = result {
                tracing::error!("Failed to open editor: {}", e);
                return false;
            }
            true
        } else {
            if let Err(e) = std::process::Command::new(&editor)
                .arg(path)
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
            {
                tracing::error!("Failed to open editor: {}", e);
                return false;
            }
            false
        }
    }

    fn reload_edited_config(&mut self, path: &std::path::Path) {
        if path.file_name().and_then(|n| n.to_str()) != Some("instance.json") {
            return;
        }

        let Some(name) = path
            .parent()
            .and_then(|p| p.file_name())
            .and_then(|n| n.to_str())
        else {
            return;
        };

        match self.instance_manager.load_one(name) {
            Ok(config) => {
                self.instances_state.replace_instance(name, config);
            }
            Err(e) => {
                tracing::error!("Failed to reload edited instance '{}': {}", name, e);
                error_buffer::push_error(error_buffer::ErrorEvent {
                    id: 0,
                    level: tracing::Level::ERROR,
                    message: format!("Failed to reload edited instance '{name}': {e}"),
                    pushed_at: std::time::Instant::now(),
                });
            }
        }
    }

    pub(super) fn spawn_launch(&self, instance: crate::instance::InstanceConfig) {
        use crate::instance::launch;
        use crate::running;

        let instance = match self.instance_manager.load_one(&instance.name) {
            Ok(config) => config,
            Err(e) => {
                error_buffer::push_error(error_buffer::ErrorEvent {
                    id: 0,
                    level: tracing::Level::ERROR,
                    message: format!("Failed to load instance '{}': {e}", instance.name),
                    pushed_at: std::time::Instant::now(),
                });
                return;
            }
        };

        running::set_state(&instance.name, running::RunState::Authenticating);

        let instances_dir = self.instance_manager.instances_dir.clone();
        let meta_dir = self.instance_manager.meta_dir.clone();

        tokio::spawn(async move {
            if let Err(e) = launch::launch(&instance, &instances_dir, &meta_dir).await {
                tracing::error!("Failed to launch '{}': {}", instance.name, e);
                running::remove(&instance.name);
            }
        });
    }

    // pops errors from the front of the queue once they've been visible long enough.
    // loops because multiple errors could expire in the same frame
    fn dismiss_expired_errors(&self) {
        use crate::config::SETTINGS;
        loop {
            match error_buffer::peek_error() {
                Some(event)
                    if event.pushed_at.elapsed().as_millis()
                        >= SETTINGS.ui.error_auto_dismiss_ms as u128 =>
                {
                    let _ = error_buffer::pop_error();
                }
                _ => break,
            }
        }
    }

    // drains the one UiEvent channel and routes each event to its handler.
    fn drain_events(&mut self) {
        while let Ok(event) = self.ui_rx.try_recv() {
            self.apply_event(event);
        }
    }

    // single dispatch point for every background-task/popup notification.
    // a pure match over UiEvent, so it stays testable with no globals or
    // terminal.
    fn apply_event(&mut self, event: super::events::UiEvent) {
        match event {
            super::events::UiEvent::InstanceCreated(config) => {
                self.instances_state.add_instance(config);
            }
            super::events::UiEvent::WizardConfirmed(params) => self.spawn_create(params),
            super::events::UiEvent::ModpackConfirmed(params) => self.spawn_install_modpack(params),
            super::events::UiEvent::ContentInstallConfirmed(params) => {
                self.spawn_install_content(params)
            }
            // updates the in-memory list (for the UI) and the on-disk
            // config (so it survives a restart) for every ended session —
            // normal exit, manual kill, or an orphan reaped by
            // reap_dead_orphans(). centralizing the write here means every
            // path persists last_played the same way.
            super::events::UiEvent::LastPlayed(name, time) => {
                for inst in &mut self.instances_state.instances {
                    if inst.name == name {
                        inst.last_played = Some(time);
                        break;
                    }
                }
                if let Err(e) = self.instance_manager.touch_last_played(&name) {
                    tracing::warn!("Failed to persist last_played for '{}': {}", name, e);
                }
            }
        }
    }

}
