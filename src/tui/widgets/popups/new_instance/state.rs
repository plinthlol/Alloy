// state machine and input handling for the new instance wizard.
// flow: Mode → [Name → Version → Loader → LoaderVersion → Confirm]
//            → [ModpackBrowse → ModpackVersion → ModpackConfirm]
// Mode forks: "New Instance" runs the step-by-step flow, "Modpacks" opens
// the modpack browser. version lists (game versions, modpack results) are
// fetched lazily from the network when you reach the step that needs them.
// note: the game-version list always comes from the Vanilla/Mojang manifest,
// since Loader hasn't been picked yet — it's no longer filtered per-loader.
// a version incompatible with the later loader pick just shows an
// empty/short list at the LoaderVersion step.

use crate::instance::{
    loader::{GameVersion, get_installer},
    models::ModLoader,
};
use crate::net::curseforge;
use crate::net::modrinth::{ProjectVersion, SearchHit};
use crate::tui::widgets::instances;
use crate::tui::widgets::popups::description;
use crossterm::event::{KeyCode, KeyEvent};
use std::sync::LazyLock;
use std::sync::{Arc, Mutex};
use tui_prompts::{FocusState, State as PromptState, TextState};

pub(crate) static WIZARD_STATE: LazyLock<Arc<Mutex<WizardState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(WizardState::default())));

#[derive(Debug, Clone)]
pub struct WizardParams {
    pub name: String,
    pub game_version: String,
    pub loader: ModLoader,
    pub loader_version: Option<String>,
}

// which catalog the modpack browser is searching. CurseForge always needs
// an API key (see config::CurseForge::effective_api_key - falls back to a
// bundled default so this doesn't dead-end for users without their own key).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ModpackSource {
    #[default]
    Modrinth,
    CurseForge,
}

impl ModpackSource {
    fn toggled(self) -> Self {
        match self {
            ModpackSource::Modrinth => ModpackSource::CurseForge,
            ModpackSource::CurseForge => ModpackSource::Modrinth,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            ModpackSource::Modrinth => "Modrinth",
            ModpackSource::CurseForge => "CurseForge",
        }
    }

    // brand colors so the popup visibly re-themes on Tab between sources,
    // instead of every source looking identical but for the label.
    // modrinth green vs curseforge orange/red — distinct hues, never blend.
    pub fn accent(self) -> ratatui::style::Color {
        match self {
            ModpackSource::Modrinth => ratatui::style::Color::Rgb(0x1b, 0xd9, 0x6a),
            ModpackSource::CurseForge => ratatui::style::Color::Rgb(0xf1, 0x64, 0x36),
        }
    }
}

// one search result, from whichever catalog is active. wraps rather than
// flattens into a shared struct so nothing is invented for fields one
// source lacks (e.g. CurseForge doesn't echo loaders per search hit).
#[derive(Debug, Clone)]
pub enum ModpackHit {
    Modrinth(SearchHit),
    CurseForge(curseforge::Mod),
}

impl ModpackHit {
    pub fn title(&self) -> &str {
        match self {
            ModpackHit::Modrinth(h) => &h.title,
            ModpackHit::CurseForge(m) => &m.name,
        }
    }

    pub fn author(&self) -> String {
        match self {
            ModpackHit::Modrinth(h) => h.author.clone(),
            ModpackHit::CurseForge(m) => m
                .authors
                .first()
                .map(|a| a.name.clone())
                .unwrap_or_else(|| "unknown".to_string()),
        }
    }

    pub fn downloads(&self) -> u64 {
        match self {
            ModpackHit::Modrinth(h) => h.downloads,
            ModpackHit::CurseForge(m) => m.download_count,
        }
    }

    pub fn description(&self) -> &str {
        match self {
            ModpackHit::Modrinth(h) => &h.description,
            ModpackHit::CurseForge(m) => &m.summary,
        }
    }

    // project icon URL if the catalog gave one — Modrinth carries it
    // directly, CurseForge nests it under `logo`. `None` covers both
    // "didn't return one" and "logo is null", which the browse popups
    // treat the same way (skip the thumbnail slot).
    pub fn icon_url(&self) -> Option<&str> {
        match self {
            ModpackHit::Modrinth(h) => h.icon_url.as_deref(),
            ModpackHit::CurseForge(m) => m.logo.as_ref().map(|l| l.thumbnail_url.as_str()),
        }
    }

    // stable id for this project, namespaced by source so a Modrinth and a
    // CurseForge project can never collide. used by the content-browse
    // popup to track which projects are already installed in the
    // instance's content dir (see content_browse::state::installed_key).
    pub fn source_key(&self) -> String {
        match self {
            ModpackHit::Modrinth(h) => format!("modrinth:{}", h.project_id),
            ModpackHit::CurseForge(m) => format!("curseforge:{}", m.id),
        }
    }
}

// one selectable release of a modpack, from whichever catalog it came from.
#[derive(Debug, Clone)]
pub enum ModpackVersionHit {
    Modrinth(ProjectVersion),
    CurseForge(curseforge::ModFile),
}

impl ModpackVersionHit {
    // the row's big text: the Minecraft version(s) this release targets.
    // installs from the content-browse popup see exactly one (filtered by
    // the instance's game version); unfiltered modpack listings can carry
    // a long tag list, so cap it.
    pub fn label(&self) -> String {
        let joined = self.game_versions();
        let versions: Vec<&str> = joined.split(", ").filter(|s| !s.is_empty()).collect();
        match versions.as_slice() {
            [] => self.version_name(),
            [one] => (*one).to_string(),
            _ => {
                const SHOWN: usize = 3;
                let mut out = versions.iter().take(SHOWN).map(|s| *s).collect::<Vec<_>>().join(", ");
                if versions.len() > SHOWN {
                    out.push_str(&format!(" +{}", versions.len() - SHOWN));
                }
                out
            }
        }
    }

    // the release's own name — Modrinth's version_number, or CurseForge's
    // display_name with a trailing archive extension stripped (some CF
    // authors set displayName to the raw file name, and "mod-1.2.3.jar"
    // is the file name, not something to headline the row with).
    pub fn version_name(&self) -> String {
        match self {
            ModpackVersionHit::Modrinth(v) => v.version_number.clone(),
            ModpackVersionHit::CurseForge(f) => strip_archive_extension(&f.display_name),
        }
    }

    // "beta"/"alpha" when the release isn't stable; None for releases (and
    // anything the catalogs don't classify), so stable versions render no
    // badge instead of a constant "release" tag on every row.
    pub fn channel(&self) -> Option<&'static str> {
        match self {
            ModpackVersionHit::Modrinth(v) => match v.version_type.as_str() {
                "beta" => Some("beta"),
                "alpha" => Some("alpha"),
                _ => None,
            },
            // CurseForge releaseType: 1 = release, 2 = beta, 3 = alpha.
            ModpackVersionHit::CurseForge(f) => match f.release_type {
                Some(2) => Some("beta"),
                Some(3) => Some("alpha"),
                _ => None,
            },
        }
    }

    pub fn game_versions(&self) -> String {
        match self {
            ModpackVersionHit::Modrinth(v) => v.game_versions.join(", "),
            ModpackVersionHit::CurseForge(f) => f.game_versions.join(", "),
        }
    }

    pub fn loaders(&self) -> String {
        match self {
            ModpackVersionHit::Modrinth(v) => v.loaders.join(", "),
            // CurseForge's file listing doesn't echo the loader per file
            // like Modrinth does, so there's nothing honest to show — it
            // only becomes known once the pack's manifest.json is read
            // during install.
            ModpackVersionHit::CurseForge(_) => "resolved during install".to_string(),
        }
    }
}

// trailing archive extension from a CurseForge display name, if any —
// case-insensitive, and only when the stem is left non-empty.
fn strip_archive_extension(name: &str) -> String {
    let lowered = name.to_lowercase();
    for ext in [".jar", ".zip"] {
        if let Some(stem) = lowered.strip_suffix(ext)
            && !stem.is_empty()
        {
            return name[..name.len() - ext.len()].to_string();
        }
    }
    name.to_string()
}

// what to fetch versions/files for, once a ModpackHit is picked - carries
// just the id(s) each source needs, not the whole hit.
pub(crate) enum ModpackVersionQuery {
    Modrinth(String),
    CurseForge(u32),
}

#[derive(Debug, Clone)]
pub enum ModpackInstallSource {
    Modrinth(ProjectVersion),
    CurseForge { file: curseforge::ModFile },
}

#[derive(Debug, Clone)]
pub struct ModpackInstallParams {
    pub name: String,
    pub source: ModpackInstallSource,
}

// PageUp/PageDown jump size for the wizard's long lists (game versions,
// modpack results, version picks) — without it, paging through the
// several-hundred-entry vanilla version list one row at a time with j/k is
// the "can't navigate" complaint this exists to fix.
const PAGE_STEP: usize = 10;

#[derive(Debug, Default, Clone, PartialEq)]
pub enum WizardStep {
    #[default]
    Mode,
    Name,
    Version,
    Loader,
    LoaderVersion,
    Confirm,
    ModpackBrowse,
    ModpackVersion,
    ModpackConfirm,
}

#[derive(Debug, Clone, Default)]
pub enum LoadState<T> {
    #[default]
    Idle,
    Loading,
    Loaded(T),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct WizardState {
    pub step: WizardStep,
    pub name_state: TextState<'static>,
    pub versions: LoadState<Vec<GameVersion>>,
    pub version_idx: usize,
    pub show_snapshots: bool,
    pub loader_idx: usize,
    pub loader_versions: LoadState<Vec<String>>,
    pub loader_version_idx: usize,
    pub version_search: crate::tui::widgets::search::SearchState,
    // Mode step: which of the two flows the user is picking between.
    pub mode_idx: usize,
    // Modpack browser: query box + search results + selected project's
    // versions. `query_focused` toggles between typing a search and
    // navigating the results list, since both live on the same step.
    pub modpack_source: ModpackSource,
    pub modpack_query: TextState<'static>,
    pub modpack_query_focused: bool,
    // live-search bookkeeping, mirroring content_browse: bump the
    // generation on every query edit so stale in-flight searches drop
    // their results, and remember the last fired query so Enter right
    // after the debounce doesn't re-fire.
    pub modpack_search_generation: u64,
    pub modpack_last_searched_query: String,
    pub modpack_results: LoadState<Vec<ModpackHit>>,
    pub modpack_idx: usize,
    pub modpack_versions: LoadState<Vec<ModpackVersionHit>>,
    pub modpack_version_idx: usize,
    pub modpack_name_state: TextState<'static>,
}

impl Default for WizardState {
    fn default() -> Self {
        Self {
            step: WizardStep::Mode,
            name_state: TextState::new().with_focus(FocusState::Focused),
            versions: LoadState::Idle,
            version_idx: 0,
            show_snapshots: false,
            loader_idx: 0,
            loader_versions: LoadState::Idle,
            loader_version_idx: 0,
            version_search: crate::tui::widgets::search::SearchState::default(),
            mode_idx: 0,
            modpack_source: ModpackSource::default(),
            modpack_query: TextState::new(),
            modpack_query_focused: false,
            modpack_search_generation: 0,
            modpack_last_searched_query: String::new(),
            modpack_results: LoadState::Idle,
            modpack_idx: 0,
            modpack_versions: LoadState::Idle,
            modpack_version_idx: 0,
            modpack_name_state: TextState::new(),
        }
    }
}

impl WizardState {
    pub fn reset(&mut self) {
        *self = WizardState::default();
    }

    pub fn selected_modpack(&self) -> Option<&ModpackHit> {
        if let LoadState::Loaded(ref results) = self.modpack_results {
            results.get(self.modpack_idx)
        } else {
            None
        }
    }

    pub fn selected_modpack_version(&self) -> Option<&ModpackVersionHit> {
        if let LoadState::Loaded(ref versions) = self.modpack_versions {
            versions.get(self.modpack_version_idx)
        } else {
            None
        }
    }

    pub fn selected_version(&self) -> Option<&GameVersion> {
        if let LoadState::Loaded(ref versions) = self.versions {
            let visible: Vec<_> = versions
                .iter()
                .filter(|v| self.show_snapshots || v.stable)
                .collect();
            visible.get(self.version_idx).copied()
        } else {
            None
        }
    }

    pub fn selected_loader(&self) -> ModLoader {
        const LOADERS: [ModLoader; 5] = [
            ModLoader::Vanilla,
            ModLoader::Fabric,
            ModLoader::Forge,
            ModLoader::NeoForge,
            ModLoader::Quilt,
        ];
        LOADERS[self.loader_idx % 5]
    }

    pub fn selected_loader_version(&self) -> Option<String> {
        if let LoadState::Loaded(ref versions) = self.loader_versions {
            versions.get(self.loader_version_idx).cloned()
        } else {
            None
        }
    }
}

pub fn handle_key(key_event: &KeyEvent, instances_state: &mut instances::State) {
    let mut state = match WIZARD_STATE.lock() {
        Ok(state) => state,
        Err(e) => {
            tracing::error!("Wizard state lock poisoned: {}", e);
            instances_state.show_popup = false;
            return;
        }
    };

    match state.step {
        WizardStep::Mode => handle_mode_key(&mut state, key_event, instances_state),
        WizardStep::Name => handle_name_key(&mut state, key_event, instances_state),
        WizardStep::Version => handle_version_key(&mut state, key_event, instances_state),
        WizardStep::Loader => handle_loader_key(&mut state, key_event, instances_state),
        WizardStep::LoaderVersion => {
            handle_loader_version_key(&mut state, key_event, instances_state)
        }
        WizardStep::Confirm => handle_confirm_key(&mut state, key_event, instances_state),
        WizardStep::ModpackBrowse => handle_modpack_browse_key(&mut state, key_event, instances_state),
        WizardStep::ModpackVersion => {
            handle_modpack_version_key(&mut state, key_event, instances_state)
        }
        WizardStep::ModpackConfirm => {
            handle_modpack_confirm_key(&mut state, key_event, instances_state)
        }
    }
}

// Mode step: pick "New Instance" (the original step wizard) or "Modpacks"
// (browse + install a pack). just a 2-item list; Enter commits to a branch.
fn handle_mode_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        // Up/k used to also do `+ 1 % 2` here - same as Down/j. harmless
        // by coincidence with exactly 2 items (+1 mod 2 == -1 mod 2), but
        // wrong on its face, and would silently break the moment a third
        // Mode option gets added. actually decrement (wrapping) now.
        KeyCode::Char('j') | KeyCode::Down => state.mode_idx = (state.mode_idx + 1) % 2,
        KeyCode::Char('k') | KeyCode::Up => state.mode_idx = (state.mode_idx + 2 - 1) % 2,
        KeyCode::Enter => {
            if state.mode_idx == 0 {
                state.step = WizardStep::Name;
                state.name_state = TextState::new().with_focus(FocusState::Focused);
            } else {
                state.step = WizardStep::ModpackBrowse;
                // unfocused: ensure_modpack_search below lands a browsable
                // listing immediately, so j/k/arrows should navigate it
                // right away instead of typing into an empty query box.
                // '/' (handle_modpack_browse_key) refocuses it to search.
                state.modpack_query = TextState::new();
                state.modpack_query_focused = false;
                ensure_modpack_search(state);
            }
        }
        _ => {}
    }
}

fn handle_name_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => {
            close_popup(state, instances_state);
        }
        KeyCode::Enter => {
            if state.name_state.value().trim().is_empty() {
                return;
            }
            state.step = WizardStep::Version;
        }
        _ => {
            state.name_state.handle_key_event(*key_event);
        }
    }
}

fn handle_version_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    // Search mode: route char input to search query
    if state.version_search.active {
        match key_event.code {
            KeyCode::Esc => {
                state.version_search.deactivate();
                clamp_version_index(state);
                return;
            }
            KeyCode::Backspace => {
                state.version_search.pop();
                clamp_version_index(state);
                return;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                // fall through to navigation below
            }
            KeyCode::Char('k') | KeyCode::Up => {
                // fall through to navigation below
            }
            KeyCode::Char(c) => {
                state.version_search.push(c);
                state.version_idx = 0; // reset to top of filtered list
                return;
            }
            _ => {}
        }
    }

    let visible_count = visible_versions(state).len();

    match key_event.code {
        KeyCode::Esc => {
            close_popup(state, instances_state);
        }
        KeyCode::Left | KeyCode::Char('h') if !state.version_search.active => {
            state.step = WizardStep::Name;
        }
        KeyCode::Char('j') | KeyCode::Down if visible_count > 0 => {
            state.version_idx = (state.version_idx + 1).min(visible_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.version_idx = state.version_idx.saturating_sub(1);
        }
        KeyCode::PageDown if visible_count > 0 => {
            state.version_idx = (state.version_idx + PAGE_STEP).min(visible_count.saturating_sub(1));
        }
        KeyCode::PageUp => {
            state.version_idx = state.version_idx.saturating_sub(PAGE_STEP);
        }
        KeyCode::Home => state.version_idx = 0,
        KeyCode::End if visible_count > 0 => state.version_idx = visible_count - 1,
        KeyCode::Char('s') if !state.version_search.active => {
            state.show_snapshots = !state.show_snapshots;
            clamp_version_index(state);
        }
        KeyCode::Char('/') if !state.version_search.active => {
            state.version_search.activate();
            state.version_idx = 0;
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') if !state.version_search.active => {
            if state.selected_version().is_none() {
                return;
            }
            state.step = WizardStep::Loader;
        }
        KeyCode::Enter if state.version_search.active => {
            state.version_search.active = false;
        }
        _ => {}
    }
}

fn handle_loader_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => state.step = WizardStep::Version,
        KeyCode::Char('j') | KeyCode::Down => {
            state.loader_idx = (state.loader_idx + 1).min(4);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.loader_idx = state.loader_idx.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            state.loader_versions = LoadState::Idle;
            state.loader_version_idx = 0;
            if state.selected_loader() == ModLoader::Vanilla {
                state.step = WizardStep::Confirm;
            } else {
                state.step = WizardStep::LoaderVersion;
                let game_version = state.selected_version().map(|v| v.id.clone());
                let loader = state.selected_loader();
                if let Some(gv) = game_version {
                    ensure_loader_versions_loaded(state, loader, gv);
                }
            }
        }
        _ => {}
    }
}

fn handle_loader_version_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    if state.selected_loader() == ModLoader::Vanilla {
        state.step = WizardStep::Confirm;
        return;
    }

    let version_count = match &state.loader_versions {
        LoadState::Loaded(versions) => versions.len(),
        _ => 0,
    };

    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => state.step = WizardStep::Loader,
        KeyCode::Char('j') | KeyCode::Down if version_count > 0 => {
            state.loader_version_idx =
                (state.loader_version_idx + 1).min(version_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.loader_version_idx = state.loader_version_idx.saturating_sub(1);
        }
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if state.selected_loader_version().is_none() {
                return;
            }
            state.step = WizardStep::Confirm;
        }
        _ => {}
    }
}

fn handle_confirm_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => {
            if state.selected_loader() == ModLoader::Vanilla {
                state.step = WizardStep::Loader;
            } else {
                state.step = WizardStep::LoaderVersion;
            }
        }
        KeyCode::Enter => {
            let selected_version = match state.selected_version() {
                Some(version) => version.id.clone(),
                None => return,
            };

            let params = WizardParams {
                name: state.name_state.value().trim().to_string(),
                game_version: selected_version,
                loader: state.selected_loader(),
                loader_version: if state.selected_loader() == ModLoader::Vanilla {
                    None
                } else {
                    state.selected_loader_version()
                },
            };

            crate::tui::events::emit(crate::tui::events::UiEvent::WizardConfirmed(params));

            close_popup(state, instances_state);
        }
        _ => {}
    }
}

// Modpack browse step: a search box (modpack_query) plus a results list
// sharing one step — no room for a separate "type your search" step.
// `modpack_query_focused` decides which half keys go to: while focused,
// typed chars edit the query and results update live (debounced, see
// schedule_modpack_search); Enter commits focus back to the list. '/'
// jumps back to refining the query.
fn handle_modpack_browse_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    if state.modpack_query_focused {
        match key_event.code {
            // Esc blurs back to the results list first (the search box
            // "disappears"); a second Esc closes the wizard via the
            // unfocused branch below.
            KeyCode::Esc => state.modpack_query_focused = false,
            KeyCode::Tab => toggle_modpack_source(state),
            // search is live, so Enter just commits focus back to the list.
            // if the debounce hasn't fired yet, bump the generation and
            // fire immediately; a no-op if the exact query already fired.
            KeyCode::Enter => {
                state.modpack_search_generation += 1;
                ensure_modpack_search(state);
                state.modpack_query_focused = false;
            }
            _ => {
                state.modpack_query.handle_key_event(*key_event);
                schedule_modpack_search(state);
            }
        }
        return;
    }

    let result_count = match &state.modpack_results {
        LoadState::Loaded(results) => results.len(),
        _ => 0,
    };

    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Tab => toggle_modpack_source(state),
        KeyCode::Char('/') => {
            state.modpack_query_focused = true;
        }
        KeyCode::Char('j') | KeyCode::Down if result_count > 0 => {
            state.modpack_idx = (state.modpack_idx + 1).min(result_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.modpack_idx = state.modpack_idx.saturating_sub(1);
        }
        KeyCode::PageDown if result_count > 0 => {
            state.modpack_idx = (state.modpack_idx + PAGE_STEP).min(result_count.saturating_sub(1));
        }
        KeyCode::PageUp => {
            state.modpack_idx = state.modpack_idx.saturating_sub(PAGE_STEP);
        }
        // 'h' is a vim-flavored alias for Home - jumps straight back to the
        // top of the results without reaching for a dedicated Home key,
        // which not every keyboard/terminal sends reliably.
        KeyCode::Home | KeyCode::Char('h') => state.modpack_idx = 0,
        KeyCode::End if result_count > 0 => state.modpack_idx = result_count - 1,
        // skips straight to Confirm with the newest version pre-selected
        // once it loads, instead of Search -> Version -> pick -> Confirm.
        // by the time the user hits Enter on the (pre-filled) name field,
        // the fetch below has almost always already landed.
        KeyCode::Char('i') => {
            if state.selected_modpack().is_none() {
                return;
            }
            state.modpack_versions = LoadState::Idle;
            state.modpack_version_idx = 0;
            let query = match state.selected_modpack() {
                Some(ModpackHit::Modrinth(hit)) => {
                    Some(ModpackVersionQuery::Modrinth(hit.project_id.clone()))
                }
                Some(ModpackHit::CurseForge(m)) => Some(ModpackVersionQuery::CurseForge(m.id)),
                None => None,
            };
            let Some(query) = query else { return };
            if state.modpack_name_state.value().is_empty()
                && let Some(hit) = state.selected_modpack()
            {
                let mut name_state = TextState::new()
                    .with_value(hit.title().to_string())
                    .with_focus(FocusState::Focused);
                // with_value() only sets the text - it leaves the cursor
                // at position 0, so typing would insert at the start
                // instead of appending. move it to the end to match what
                // the field visually shows (the fake cursor glyph always
                // renders after the text).
                name_state.move_end();
                state.modpack_name_state = name_state;
            }
            state.step = WizardStep::ModpackConfirm;
            ensure_modpack_versions_loaded(state, query);
        }
        // 'v' keeps the old Enter behavior (version list); Enter itself
        // opens the full project description, same as the content-browse
        // popup. 'i' still fast-tracks to Confirm with the newest version.
        KeyCode::Enter => {
            let Some(hit) = state.selected_modpack().cloned() else {
                return;
            };
            let source = match &hit {
                ModpackHit::Modrinth(h) => description::DescriptionSource::Modrinth {
                    project_id: h.project_id.clone(),
                },
                ModpackHit::CurseForge(m) => description::DescriptionSource::CurseForge { mod_id: m.id },
            };
            description::open(source, hit.title());
        }
        KeyCode::Char('v') => {
            state.modpack_versions = LoadState::Idle;
            state.modpack_version_idx = 0;
            let query = match state.selected_modpack() {
                Some(ModpackHit::Modrinth(hit)) => {
                    Some(ModpackVersionQuery::Modrinth(hit.project_id.clone()))
                }
                Some(ModpackHit::CurseForge(m)) => Some(ModpackVersionQuery::CurseForge(m.id)),
                None => None,
            };
            let Some(query) = query else { return };
            state.step = WizardStep::ModpackVersion;
            ensure_modpack_versions_loaded(state, query);
        }
        _ => {}
    }
}

// switching catalogs mid-browse invalidates whatever's on screen — a
// Modrinth project id means nothing to the CurseForge API and vice versa
// — so clear results/versions rather than try to translate them.
fn toggle_modpack_source(state: &mut WizardState) {
    state.modpack_source = state.modpack_source.toggled();
    state.modpack_results = LoadState::Idle;
    state.modpack_idx = 0;
    state.modpack_versions = LoadState::Idle;
    state.modpack_version_idx = 0;
    // any pending debounced search belongs to the old source - invalidate
    // it so it can't fire against the newly-switched catalog.
    state.modpack_search_generation += 1;
    // stays unfocused - same reasoning as entering ModpackBrowse: the
    // re-fired search lands a fresh browsable listing, so keep it
    // immediately navigable instead of re-stealing focus into the box.
    state.modpack_query_focused = false;
    ensure_modpack_search(state);
}

fn handle_modpack_version_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    let version_count = match &state.modpack_versions {
        LoadState::Loaded(versions) => versions.len(),
        _ => 0,
    };

    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        KeyCode::Left | KeyCode::Char('h') => state.step = WizardStep::ModpackBrowse,
        KeyCode::Char('j') | KeyCode::Down if version_count > 0 => {
            state.modpack_version_idx =
                (state.modpack_version_idx + 1).min(version_count.saturating_sub(1));
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.modpack_version_idx = state.modpack_version_idx.saturating_sub(1);
        }
        KeyCode::PageDown if version_count > 0 => {
            state.modpack_version_idx =
                (state.modpack_version_idx + PAGE_STEP).min(version_count.saturating_sub(1));
        }
        KeyCode::PageUp => {
            state.modpack_version_idx = state.modpack_version_idx.saturating_sub(PAGE_STEP);
        }
        KeyCode::Home => state.modpack_version_idx = 0,
        KeyCode::End if version_count > 0 => state.modpack_version_idx = version_count - 1,
        KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
            if state.selected_modpack_version().is_none() {
                return;
            }
            // pre-fill the instance name from the pack's title so most
            // installs are a single Enter away, but leave it editable in
            // case the name collides with an existing instance.
            if state.modpack_name_state.value().is_empty()
                && let Some(hit) = state.selected_modpack()
            {
                let mut name_state = TextState::new()
                    .with_value(hit.title().to_string())
                    .with_focus(FocusState::Focused);
                // see the matching comment in handle_modpack_browse_key:
                // without this the cursor stays at position 0 and typing
                // inserts at the start of the pre-filled name instead of
                // the end.
                name_state.move_end();
                state.modpack_name_state = name_state;
            }
            state.step = WizardStep::ModpackConfirm;
        }
        _ => {}
    }
}

fn handle_modpack_confirm_key(
    state: &mut WizardState,
    key_event: &KeyEvent,
    instances_state: &mut instances::State,
) {
    match key_event.code {
        KeyCode::Esc => close_popup(state, instances_state),
        // only the arrow key goes "back" here, unlike the standard steps
        // that also accept 'h' — this step has a live text field (the
        // instance name), and 'h' must stay typeable in it.
        KeyCode::Left => state.step = WizardStep::ModpackVersion,
        KeyCode::Enter => {
            let name = state.modpack_name_state.value().trim().to_string();
            if name.is_empty() {
                return;
            }
            let source = match state.selected_modpack_version() {
                Some(ModpackVersionHit::Modrinth(v)) => ModpackInstallSource::Modrinth(v.clone()),
                Some(ModpackVersionHit::CurseForge(file)) => {
                    ModpackInstallSource::CurseForge { file: file.clone() }
                }
                None => return,
            };

            crate::tui::events::emit(crate::tui::events::UiEvent::ModpackConfirmed(
                ModpackInstallParams { name, source },
            ));

            close_popup(state, instances_state);
        }
        _ => {
            state.modpack_name_state.handle_key_event(*key_event);
        }
    }
}

fn close_popup(state: &mut WizardState, instances_state: &mut instances::State) {
    state.reset();
    instances_state.show_popup = false;
}

pub(crate) fn visible_versions(state: &WizardState) -> Vec<GameVersion> {
    let q = state.version_search.query.to_lowercase();
    match &state.versions {
        LoadState::Loaded(versions) => versions
            .iter()
            .filter(|v| state.show_snapshots || v.stable)
            .filter(|v| q.is_empty() || v.id.to_lowercase().contains(&q))
            .cloned()
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn clamp_version_index(state: &mut WizardState) {
    let count = visible_versions(state).len();
    if count == 0 {
        state.version_idx = 0;
    } else if state.version_idx >= count {
        state.version_idx = count.saturating_sub(1);
    }
}

pub(crate) fn clamp_loader_version_index(state: &mut WizardState) {
    if let LoadState::Loaded(versions) = &state.loader_versions {
        if versions.is_empty() {
            state.loader_version_idx = 0;
        } else if state.loader_version_idx >= versions.len() {
            state.loader_version_idx = versions.len().saturating_sub(1);
        }
    } else {
        state.loader_version_idx = 0;
    }
}

// only fires on the Idle -> Loading transition to avoid spamming requests.
// the spawned task writes results back into WIZARD_STATE when done.
pub(crate) fn ensure_versions_loaded(state: &mut WizardState) {
    if !matches!(state.versions, LoadState::Idle) {
        return;
    }

    state.versions = LoadState::Loading;
    let versions_arc = WIZARD_STATE.clone();
    let loader = state.selected_loader();
    tokio::spawn(async move {
        let client = crate::net::HttpClient::shared();
        let installer = get_installer(loader);
        match installer.get_game_versions(&client).await {
            Ok(mut versions) => match versions_arc.lock() {
                Ok(mut s) => {
                    sort_versions_semver(&mut versions);
                    s.versions = LoadState::Loaded(versions);
                    clamp_version_index(&mut s);
                }
                Err(e) => {
                    tracing::error!("Wizard state lock poisoned: {}", e);
                }
            },
            Err(e) => match versions_arc.lock() {
                Ok(mut s) => {
                    s.versions = LoadState::Error(e.to_string());
                }
                Err(lock_error) => {
                    tracing::error!("Wizard state lock poisoned: {}", lock_error);
                }
            },
        }
    });
}

pub(crate) fn ensure_loader_versions_loaded(
    state: &mut WizardState,
    loader: ModLoader,
    game_version: String,
) {
    if !matches!(state.loader_versions, LoadState::Idle) {
        return;
    }

    state.loader_versions = LoadState::Loading;
    let versions_arc = WIZARD_STATE.clone();
    tokio::spawn(async move {
        let client = crate::net::HttpClient::shared();
        let installer = get_installer(loader);
        match installer.get_versions(&client, &game_version).await {
            Ok(versions) => match versions_arc.lock() {
                Ok(mut s) => {
                    s.loader_versions = LoadState::Loaded(versions);
                    clamp_loader_version_index(&mut s);
                }
                Err(e) => {
                    tracing::error!("Wizard state lock poisoned: {}", e);
                }
            },
            Err(e) => match versions_arc.lock() {
                Ok(mut s) => {
                    s.loader_versions = LoadState::Error(e.to_string());
                }
                Err(lock_error) => {
                    tracing::error!("Wizard state lock poisoned: {}", lock_error);
                }
            },
        }
    });
}

// how long to wait after the last keystroke before firing a search, so
// typing a name fans out one API call instead of one per character.
const SEARCH_DEBOUNCE_MS: u64 = 300;

// schedules a debounced modpack search: bumps the generation counter
// (invalidating any earlier pending debounce) and spawns a task that fires
// only if it still holds the latest generation when it wakes up.
fn schedule_modpack_search(state: &mut WizardState) {
    state.modpack_search_generation += 1;
    let generation = state.modpack_search_generation;
    let state_arc = WIZARD_STATE.clone();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(SEARCH_DEBOUNCE_MS)).await;
        if let Ok(mut s) = state_arc.lock() {
            if s.modpack_search_generation == generation {
                ensure_modpack_search(&mut s);
            }
        }
    });
}

// fires a modpack search against the active catalog for the current query.
// fires on every query change (the debounce collapses a keystroke burst
// into one) and on the entry/toggle empty-query "browse everything"
// listing — unlike ensure_versions_loaded's Idle-only guard, since this is
// an explicit user-initiated search, not a lazy one-shot load.
pub(crate) fn ensure_modpack_search(state: &mut WizardState) {
    let query = state.modpack_query.value().trim().to_string();
    // live-search dedup: skip when the identical query just fired for this
    // source (Enter after the debounce, or a trailing-space-only edit).
    if query == state.modpack_last_searched_query
        && !matches!(state.modpack_results, LoadState::Idle)
    {
        return;
    }
    state.modpack_last_searched_query = query.clone();
    let generation = state.modpack_search_generation;
    let source = state.modpack_source;
    state.modpack_results = LoadState::Loading;
    state.modpack_idx = 0;
    let state_arc = WIZARD_STATE.clone();
    tokio::spawn(async move {
        let client = crate::net::HttpClient::shared();
        let outcome: Result<Vec<ModpackHit>, String> = match source {
            ModpackSource::Modrinth => {
                crate::net::modrinth::search(&client, &query, "modpack", None, None, 0, 40)
                    .await
                    .map(|resp| resp.hits.into_iter().map(ModpackHit::Modrinth).collect())
                    .map_err(|e| e.to_string())
            }
            ModpackSource::CurseForge => {
                let api_key = crate::config::SETTINGS
                    .curseforge
                    .effective_api_key()
                    .unwrap_or("")
                    .to_string();
                curseforge::search(
                    &client,
                    &api_key,
                    &query,
                    curseforge::CLASS_ID_MODPACK,
                    None,
                    None,
                    0,
                    40,
                )
                .await
                .map(|resp| resp.data.into_iter().map(ModpackHit::CurseForge).collect())
                .map_err(|e| e.to_string())
            }
        };
        match state_arc.lock() {
            Ok(mut s) => {
                // a newer search has superseded this one (more typing,
                // source switch, etc.) - drop the stale results rather
                // than clobbering the newer fetch's output.
                if s.modpack_search_generation != generation {
                    return;
                }
                s.modpack_results = match outcome {
                    Ok(hits) => LoadState::Loaded(hits),
                    Err(e) => LoadState::Error(e),
                };
            }
            Err(e) => tracing::error!("Wizard state lock poisoned: {}", e),
        }
    });
}

// loads every published version/file of a modpack, newest first, so the
// version step can list them for the user to pick which release to install.
pub(crate) fn ensure_modpack_versions_loaded(state: &mut WizardState, query: ModpackVersionQuery) {
    if !matches!(state.modpack_versions, LoadState::Idle) {
        return;
    }
    state.modpack_versions = LoadState::Loading;
    let state_arc = WIZARD_STATE.clone();
    tokio::spawn(async move {
        let client = crate::net::HttpClient::shared();
        let outcome: Result<Vec<ModpackVersionHit>, String> = match query {
            ModpackVersionQuery::Modrinth(project_id) => {
                crate::net::modrinth::get_project_versions(&client, &project_id, None, None)
                    .await
                    .map(|versions| {
                        versions
                            .into_iter()
                            .map(ModpackVersionHit::Modrinth)
                            .collect()
                    })
                    .map_err(|e| e.to_string())
            }
            ModpackVersionQuery::CurseForge(mod_id) => {
                let api_key = crate::config::SETTINGS
                    .curseforge
                    .effective_api_key()
                    .unwrap_or("")
                    .to_string();
                curseforge::get_files(&client, &api_key, mod_id, None, None)
                    .await
                    .map(|resp| {
                        resp.data
                            .into_iter()
                            .map(ModpackVersionHit::CurseForge)
                            .collect()
                    })
                    .map_err(|e| e.to_string())
            }
        };
        match state_arc.lock() {
            Ok(mut s) => {
                s.modpack_versions = match outcome {
                    Ok(v) => {
                        if s.modpack_version_idx >= v.len() {
                            s.modpack_version_idx = 0;
                        }
                        LoadState::Loaded(v)
                    }
                    Err(e) => LoadState::Error(e),
                };
            }
            Err(e) => tracing::error!("Wizard state lock poisoned: {}", e),
        }
    });
}

// quick-and-dirty semver compare: splits on dots, compares numerically.
// no pre-release handling — good enough for mc versions.
//
// each dot-separated segment can carry a non-numeric suffix (the "3" in
// "26.3-snapshot-6", the "2" in "26.2-rc-2"), so parse only the leading
// run of digits rather than requiring a whole numeric segment. parsing the
// whole segment used to fail on every snapshot/rc/pre id and silently
// default to 0, sorting them as if all were "x.0" — e.g.
// "26.3-snapshot-6" compared as [26, 0], sorting *below* "26.1" ([26, 1]).
fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    let leading_number = |s: &str| -> u64 {
        s.chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0)
    };
    let parse_parts = |s: &str| -> Vec<u64> { s.split('.').map(leading_number).collect() };
    let a_parts = parse_parts(a);
    let b_parts = parse_parts(b);
    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        match ap.cmp(bp) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }
    a_parts.len().cmp(&b_parts.len())
}

fn sort_versions_semver(versions: &mut [GameVersion]) {
    versions.sort_by(|a, b| compare_semver(&b.id, &a.id));
}
