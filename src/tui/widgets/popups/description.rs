// project description popup, ported from rmcl's discovery project page:
// fetches the long-form description (Modrinth markdown `body`, CurseForge
// HTML description — the markdown renderer's normalize_html converts it)
// and renders it with the full tui/widgets/markdown.rs pipeline, inline
// images included. opened from the browse popups with Enter on a search
// result; `v` opens the version list instead (the old Enter behavior).

use std::collections::HashMap;
use std::sync::{Arc, LazyLock, Mutex};

use image::DynamicImage;
use ratatui::{
    Frame,
    layout::Rect,
    widgets::{Paragraph, Widget, Wrap},
};

use crate::config::theme::THEME;
use crate::net::{MAX_PROVIDER_ASSET_BYTES, HttpClient};
use crate::tui::widgets::markdown::{self, Document};
use crate::tui::request_redraw;
use crate::tui::widgets::popups::keybind_line;
// which project to fetch a body for, in whichever API shape its catalog
// needs. the source_key doubles as the cache key for bodies and images.
#[derive(Debug, Clone)]
pub enum DescriptionSource {
    Modrinth { project_id: String },
    CurseForge { mod_id: u32 },
}

impl DescriptionSource {
    pub fn key(&self) -> String {
        match self {
            DescriptionSource::Modrinth { project_id } => format!("modrinth:{project_id}"),
            DescriptionSource::CurseForge { mod_id } => format!("curseforge:{mod_id}"),
        }
    }
}

pub struct DescriptionState {
    pub open: bool,
    // bumps on every open() so results from a superseded fetch (user closed
    // and reopened a different project quickly) are dropped instead of
    // clobbering the newer popup's contents.
    pub request_id: u64,
    pub source_key: String,
    pub title: String,
    pub document: Option<Document>,
    pub error: Option<String>,
    pub scroll: usize,
    pub max_scroll: usize,
}

impl Default for DescriptionState {
    fn default() -> Self {
        Self {
            open: false,
            request_id: 0,
            source_key: String::new(),
            title: String::new(),
            document: None,
            error: None,
            scroll: 0,
            max_scroll: 0,
        }
    }
}

static DESCRIPTION_STATE: LazyLock<Arc<Mutex<DescriptionState>>> =
    LazyLock::new(|| Arc::new(Mutex::new(DescriptionState::default())));

// fetched bodies, so re-opening a project you already viewed is instant.
// key = DescriptionSource::key() -> (title, markdown/html body).
static BODY_CACHE: LazyLock<Mutex<HashMap<String, (String, String)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

// decoded inline document images, keyed (source_key, image url) — same
// process-wide cache pattern as WEB_ICONS: a project's images are fetched
// once per session no matter how often the popup reopens.
static IMAGE_CACHE: LazyLock<Mutex<HashMap<(String, String), DynamicImage>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn is_open() -> bool {
    DESCRIPTION_STATE.lock().map(|s| s.open).unwrap_or(false)
}

pub fn open(source: DescriptionSource, fallback_title: &str) {
    let fallback_title = fallback_title.to_string();
    let key = source.key();
    let request_id = {
        let mut state = match DESCRIPTION_STATE.lock() {
            Ok(state) => state,
            Err(_) => return,
        };
        state.request_id = state.request_id.wrapping_add(1);
        state.open = true;
        state.source_key = key.clone();
        state.title = fallback_title.to_string();
        state.document = None;
        state.error = None;
        state.scroll = 0;
        state.max_scroll = 0;
        state.request_id
    };
    tokio::spawn(async move { fetch(request_id, key, source, fallback_title.to_string()) });
}

// fetch hardening: Document::new runs in a spawned task, where a panic
// would be swallowed silently and the popup would show "Loading..." forever.
// catch it, log it, and surface it as a visible error instead.
fn build_document(title: &str, body: &str) -> Result<Document, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        Document::new(title, body)
    }))
    .map_err(|panic| {
        let message = panic
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| panic.downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "unknown panic".to_string());
        tracing::error!("Description render panicked: {message}");
        format!("Failed to render description: {message}")
    })
}

async fn fetch(request_id: u64, key: String, source: DescriptionSource, fallback_title: String) {
    tracing::info!("Fetching description for {key}");
    let client = HttpClient::shared();

    // body: cache hit is instant; miss goes to the catalog API
    let (title, body) = match BODY_CACHE.lock().ok().and_then(|c| c.get(&key).cloned()) {
        Some(cached) => cached,
        None => {
            let result = match &source {
                DescriptionSource::Modrinth { project_id } => {
                    crate::net::modrinth::get_project(&client, project_id)
                        .await
                        .map(|p| (p.title, p.body))
                        .map_err(|e| e.to_string())
                }
                DescriptionSource::CurseForge { mod_id } => {
                    let api_key = crate::config::SETTINGS
                        .curseforge
                        .effective_api_key()
                        .unwrap_or("")
                        .to_string();
                    crate::net::curseforge::get_description(&client, &api_key, *mod_id)
                        .await
                        .map(|body| (fallback_title.clone(), body))
                        .map_err(|e| e.to_string())
                }
            };
            match result {
                Ok(pair) => {
                    if let Ok(mut cache) = BODY_CACHE.lock() {
                        cache.insert(key.clone(), pair.clone());
                    }
                    pair
                }
                Err(e) => {
                    apply(request_id, &key, |state| state.error = Some(e.to_string()));
                    return;
                }
            }
        }
    };

    let mut document = match build_document(&title, &body) {
        Ok(document) => document,
        Err(message) => {
            apply(request_id, &key, |state| state.error = Some(message));
            request_redraw();
            return;
        }
    };

    // satisfy what we can from the image cache before fetching the rest
    let mut missing: Vec<String> = Vec::new();
    if let Ok(cache) = IMAGE_CACHE.lock() {
        for url in document.image_urls() {
            match cache.get(&(key.clone(), url.clone())) {
                Some(image) => document.set_image(&url, Ok(image.clone())),
                None => missing.push(url),
            }
        }
    } else {
        missing = document.image_urls();
    }
    missing.sort();
    missing.dedup();

    apply(request_id, &key, |state| {
        state.title = title.clone();
        state.document = Some(document);
        state.error = None;
        state.scroll = 0;
        state.max_scroll = 0;
    });
    tracing::info!("Description for {key} ready ({} image(s))", missing.len());
    if missing.is_empty() {
        request_redraw();
        return;
    }

    // bounded fetch of inline images: same 4-at-a-time shape as rmcl's
    // discovery page, results applied only if this request still owns the
    // popup (not superseded by another open()).
    let semaphore = Arc::new(tokio::sync::Semaphore::new(4));
    let mut tasks = tokio::task::JoinSet::new();
    for url in missing {
        let client = client.clone();
        let semaphore = semaphore.clone();
        tasks.spawn(async move {
            let result = async {
                let _permit = semaphore
                    .acquire_owned()
                    .await
                    .map_err(|e| e.to_string())?;
                let bytes = client
                    .get_bytes_limited(&url, MAX_PROVIDER_ASSET_BYTES)
                    .await
                    .map_err(|e| e.to_string())?;
                tokio::task::spawn_blocking(move || markdown::decode_image(&bytes))
                    .await
                    .map_err(|e| e.to_string())?
            }
            .await;
            (url, result)
        });
    }
    while let Some(task) = tasks.join_next().await {
        let Ok((url, result)) = task else { continue };
        if let Ok(image) = &result
            && let Ok(mut cache) = IMAGE_CACHE.lock()
        {
            cache.insert((key.clone(), url.clone()), image.clone());
        }
        apply(request_id, &key, |state| {
            if let Some(document) = state.document.as_mut() {
                document.set_image(&url, result.clone());
            }
        });
        request_redraw();
    }
}

// applies a mutation to the popup state only if the popup is still open and
// still showing this request's project (dropped otherwise, like rmcl's
// request_id filter).
fn apply(request_id: u64, key: &str, f: impl FnOnce(&mut DescriptionState)) {
    if let Ok(mut state) = DESCRIPTION_STATE.lock()
        && state.open
        && state.request_id == request_id
        && state.source_key == key
    {
        f(&mut state);
    }
}

// the description renders *in place* of the browse popup's content — the
// host popup swaps its title/keybinds (see title()/keybinds()) and calls
// render_content() with its content area after drawing its frame. no
// separate overlay popup.

// closes the description view and scrolls it; returns true while still open.
// routed from input.rs before the host popup sees any keys, so the view is
// modal over the browse popup it replaced.
pub fn handle_key(key_event: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::KeyCode;
    let mut state = match DESCRIPTION_STATE.lock() {
        Ok(state) => state,
        Err(_) => return false,
    };
    match key_event.code {
        KeyCode::Esc | KeyCode::Left | KeyCode::Char('h') => state.open = false,
        KeyCode::Char('j') | KeyCode::Down => {
            state.scroll = state.scroll.saturating_add(1).min(state.max_scroll);
        }
        KeyCode::Char('k') | KeyCode::Up => {
            state.scroll = state.scroll.saturating_sub(1);
        }
        KeyCode::PageDown | KeyCode::Char('d') => {
            state.scroll = state.scroll.saturating_add(10).min(state.max_scroll);
        }
        KeyCode::PageUp | KeyCode::Char('u') => {
            state.scroll = state.scroll.saturating_sub(10);
        }
        KeyCode::Char('g') | KeyCode::Home => state.scroll = 0,
        KeyCode::Char('G') | KeyCode::End => state.scroll = state.max_scroll,
        _ => {}
    }
    state.open
}

/// current description title, for the host popup's title bar.
pub fn title() -> String {
    DESCRIPTION_STATE
        .lock()
        .map(|s| s.title.clone())
        .unwrap_or_default()
}

/// keybind footer while the description view is showing.
pub fn keybinds() -> ratatui::text::Line<'static> {
    keybind_line(&[
        ("j/k", " scroll"),
        ("PgUp/PgDn", " page"),
        ("g/G", " top/end"),
        ("h/Esc", " back"),
    ])
}

/// draws the description (or its loading/error state) into `inner`.
/// called by the host browse popup after its frame is drawn —
/// markdown::render needs &mut Frame for the inline images.
pub fn render_content(frame: &mut Frame, inner: Rect, picker: &ratatui_image::picker::Picker) {
    let mut state = match DESCRIPTION_STATE.lock() {
        Ok(state) => state,
        Err(_) => return,
    };
    let theme = THEME.as_ref();

    if let Some(error) = &state.error {
        Paragraph::new(error.as_str())
            .style(ratatui::style::Style::default().fg(theme.error()))
            .wrap(Wrap { trim: true })
            .render(inner, frame.buffer_mut());
        return;
    }
    if state.document.is_none() {
        Paragraph::new(format!("Loading {}...", state.title))
            .style(ratatui::style::Style::default().fg(theme.text_dim()))
            .render(inner, frame.buffer_mut());
        return;
    }
    // markdown::render clamps scroll against the real content height and
    // returns it — read scroll before taking the document borrow, write back
    // after it ends.
    let mut scroll = state.scroll;
    let height = if let Some(document) = state.document.as_mut() {
        markdown::render(frame, inner, document, &mut scroll, picker)
    } else {
        return;
    };
    state.scroll = scroll;
    state.max_scroll = height;
}
