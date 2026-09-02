// thumbnail cache for content browsed *from the web* (Modrinth/CurseForge
// search results in the wizard and browse popups). mirrors content/list.rs's
// local-icon pattern — bounded concurrency, decode off the render thread,
// StatefulProtocol built lazily once the picker is known — with a disk
// cache on top since these come over the network: once fetched, re-opening
// a popup or scrolling past the same project is a local read, not a
// re-download.
//
// process-wide (like BROWSE_STATE/WIZARD_STATE), so the same icon seen in
// both the wizard and the mod browser is only fetched once.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex};

use ratatui_image::protocol::StatefulProtocol;

// icons render at a handful of terminal cells — no point keeping (or
// transferring, for pixel-shipping protocols like Sixel/iTerm2) more.
// 192px keeps the thumbnails sharp at the current 3-row render height
// (and its ~6x3 cell area) even on high-DPI terminals where each cell
// covers more pixels.
const ICON_SIDE_PX: u32 = 192;
// caps in-flight fetches. a 40-hit search result requests every visible
// icon as it scrolls into view; without a cap that's 40 connections
// competing with the popup's own search/version calls.
const MAX_CONCURRENT_FETCHES: usize = 6;

// disk cache budget. these are small resized-on-write thumbnails (see
// ICON_SIDE_PX), so 10MB is thousands of icons — but it's a *cache* dir
// under the OS cache dir, not a data dir, so it shouldn't grow forever.
const MAX_CACHE_BYTES: i64 = 10 * 1024 * 1024;
// when evicting, free a bit more than the minimum so a burst of writes at
// the cap doesn't re-trigger a directory walk on every write.
const EVICT_TARGET_BYTES: i64 = MAX_CACHE_BYTES - (MAX_CACHE_BYTES / 10);

struct PendingIcon {
    url: String,
    image: image::DynamicImage,
}

pub struct WebIconCache {
    protocols: HashMap<String, StatefulProtocol>,
    requested: HashSet<String>,
    failed: HashSet<String>,
    pending: Arc<Mutex<Vec<PendingIcon>>>,
    semaphore: Arc<tokio::sync::Semaphore>,
}

impl Default for WebIconCache {
    fn default() -> Self {
        Self {
            protocols: HashMap::new(),
            requested: HashSet::new(),
            failed: HashSet::new(),
            pending: Arc::new(Mutex::new(Vec::new())),
            semaphore: Arc::new(tokio::sync::Semaphore::new(MAX_CONCURRENT_FETCHES)),
        }
    }
}

pub static WEB_ICONS: LazyLock<Mutex<WebIconCache>> = LazyLock::new(|| Mutex::new(WebIconCache::default()));

// one shared HTTP client instead of one per fetch: reqwest pools
// connections per-client, so a fresh client per download meant re-TLS and
// a cold pool for each in-flight fetch. safe to share — the client is
// immutable connection state (see net::HttpClient), never per-request data.
static ICON_HTTP: LazyLock<crate::net::HttpClient> = LazyLock::new(crate::net::HttpClient::new);

fn disk_cache_dir() -> PathBuf {
    dirs::cache_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("alloy")
        .join("icons")
}

// content-addressed by URL — that's all callers have (search hits carry a
// direct image URL, not a content hash). fine here, since project icons
// are small and essentially never change in place.
fn disk_cache_path(url: &str) -> PathBuf {
    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    url.hash(&mut hasher);
    disk_cache_dir().join(format!("{:x}", hasher.finish()))
}

// running disk-cache size, kept incrementally rather than re-walking the
// dir on every write — a cache hit or a single new icon stays O(1) no
// matter how many icons are cached.
//
// signed so a seed race (see seed_cache_size_once) can't underflow it into
// a huge unsigned wraparound; any real total fits an i64 fine.
static CACHE_BYTES: AtomicI64 = AtomicI64::new(0);
static CACHE_SEEDED: AtomicBool = AtomicBool::new(false);
// one eviction sweep at a time — concurrent writes can all notice the
// over-budget in the same tick, and running the walk twice is wasted work.
static EVICTING: AtomicBool = AtomicBool::new(false);

// walks the disk cache once so CACHE_BYTES counts icons written by a
// *previous* run too. runs at most once per process (guarded by
// CACHE_SEEDED), off the fetch path so it never slows the first icon.
fn seed_cache_size_once() {
    if CACHE_SEEDED.swap(true, Ordering::AcqRel) {
        return;
    }
    tokio::spawn(async move {
        let dir = disk_cache_dir();
        let total = tokio::task::spawn_blocking(move || {
            let mut total: i64 = 0;
            if let Ok(entries) = std::fs::read_dir(&dir) {
                for entry in entries.flatten() {
                    if let Ok(meta) = entry.metadata()
                        && meta.is_file()
                    {
                        total += meta.len() as i64;
                    }
                }
            }
            total
        })
        .await
        .unwrap_or(0);
        CACHE_BYTES.fetch_add(total, Ordering::AcqRel);
        maybe_evict();
    });
}

// after every disk-cache write: bump the running total O(1) and, only if
// that pushes the cache over budget, spawn an eviction sweep — the
// directory listing/sort never runs when there's nothing to evict.
fn record_cache_write(size: u64) {
    CACHE_BYTES.fetch_add(size as i64, Ordering::AcqRel);
    maybe_evict();
}

fn maybe_evict() {
    if CACHE_BYTES.load(Ordering::Acquire) <= MAX_CACHE_BYTES {
        return;
    }
    if EVICTING.swap(true, Ordering::AcqRel) {
        return; // another sweep is already running
    }
    tokio::spawn(async move {
        let dir = disk_cache_dir();
        let freed = tokio::task::spawn_blocking(move || evict_oldest_until_under_budget(&dir))
            .await
            .unwrap_or(0);
        CACHE_BYTES.fetch_sub(freed, Ordering::AcqRel);
        EVICTING.store(false, Ordering::Release);
    });
}

// removes least-recently-used icons (oldest mtime first — the fetch path
// touches mtime on every cache hit, see `request`, so it's a real LRU,
// not insertion order) until back at EVICT_TARGET_BYTES. blocking task,
// since it's sync fs IO; only invoked from maybe_evict, which already
// guarantees a single sweep at a time.
fn evict_oldest_until_under_budget(dir: &std::path::Path) -> i64 {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return 0;
    };
    let mut files: Vec<(PathBuf, u64, std::time::SystemTime)> = entries
        .flatten()
        .filter_map(|entry| {
            let meta = entry.metadata().ok()?;
            if !meta.is_file() {
                return None;
            }
            let modified = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
            Some((entry.path(), meta.len(), modified))
        })
        .collect();
    files.sort_by_key(|(_, _, modified)| *modified);

    let mut total: i64 = files.iter().map(|(_, len, _)| *len as i64).sum();
    let mut freed = 0i64;
    for (path, len, _) in files {
        if total <= EVICT_TARGET_BYTES {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len as i64;
            freed += len as i64;
        }
    }
    freed
}

// best-effort LRU touch: bump a cached file's mtime to now so a popular
// icon (scrolled past repeatedly) isn't evicted ahead of one written once
// and never looked at again. failures ignored — worst case eviction falls
// back to write order for that entry, still a fine approximation.
fn touch(path: &std::path::Path) {
    if let Ok(file) = std::fs::File::open(path) {
        let _ = file.set_modified(std::time::SystemTime::now());
    }
}

// shared by the disk-cache-hit and fresh-fetch paths: decode whatever
// bytes came in (load_from_memory auto-detects the format) and normalize
// to icon size. Triangle rather than Lanczos3: these are decorative
// thumbnails a couple of cells wide, where the sharper-but-slower filter
// is invisible — and this runs per hit per popup open/scroll, so the
// cheaper resize keeps fast scrolling from backing up behind image work.
fn decode_thumbnail(bytes: Vec<u8>) -> Option<image::DynamicImage> {
    image::load_from_memory(&bytes).ok().map(|img| {
        img.resize_exact(ICON_SIDE_PX, ICON_SIDE_PX, image::imageops::FilterType::Triangle)
    })
}

impl WebIconCache {
    /// already-decoded protocol ready to render, if this url has loaded.
    pub fn get(&mut self, url: &str) -> Option<&mut StatefulProtocol> {
        self.protocols.get_mut(url)
    }

    /// queue a fetch for `url` if it isn't already loaded/loading/failed.
    /// cheap (a couple of hash-set lookups) so it's fine to call this every
    /// frame for every currently-visible row - only actually spawns work
    /// the first time a given url is seen.
    pub fn request(&mut self, url: &str) {
        if url.is_empty()
            || self.protocols.contains_key(url)
            || self.failed.contains(url)
            || !self.requested.insert(url.to_string())
        {
            return;
        }

        seed_cache_size_once();

        let url_owned = url.to_string();
        let pending = self.pending.clone();
        let semaphore = self.semaphore.clone();

        tokio::spawn(async move {
            let _permit = semaphore
                .acquire_owned()
                .await
                .expect("icon fetch semaphore is never closed");

            let disk_path = disk_cache_path(&url_owned);

            // disk cache hit: the cached file is (since the resize-on-write
            // below) a 96px PNG thumbnail; older-version entries are still
            // full-res originals, which load_from_memory auto-detects the
            // same way. decode_thumbnail's resize is idempotent either way.
            if let Ok(bytes) = tokio::fs::read(&disk_path).await {
                // count as "used" for LRU purposes - a re-fetch of the same
                // url (popup reopened, list scrolled back) shouldn't leave
                // the file looking untouched since whichever run first
                // wrote it.
                let touch_path = disk_path.clone();
                tokio::task::spawn_blocking(move || touch(&touch_path));

                let decoded = tokio::task::spawn_blocking(move || decode_thumbnail(bytes))
                    .await
                    .ok()
                    .flatten();
                match decoded {
                    Some(image) => {
                        if let Ok(mut slot) = pending.lock() {
                            slot.push(PendingIcon { url: url_owned, image });
                            crate::tui::request_redraw();
                        }
                    }
                    None => {
                        tracing::debug!("Failed to decode icon {}", url_owned);
                    }
                }
                return;
            }

            // disk cache miss: fetch the raw original once, then decode +
            // resize + re-encode to a small PNG in one blocking task and
            // write *that* — not the raw bytes. decode+resize is the
            // expensive part, so doing it at fetch time means a hit only
            // decodes a small thumbnail, and the budget holds far more
            // thumbnails than originals.
            let fetched = match ICON_HTTP.get_bytes(&url_owned).await {
                Ok(bytes) => bytes,
                Err(e) => {
                    tracing::debug!("Failed to fetch icon {}: {}", url_owned, e);
                    return;
                }
            };

            let (image, png) = match tokio::task::spawn_blocking(move || {
                let resized = decode_thumbnail(fetched)?;
                let mut png = Vec::new();
                resized
                    .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
                    .ok()?;
                Some((resized, png))
            })
            .await
            .ok()
            .flatten()
            {
                Some(processed) => processed,
                None => {
                    tracing::debug!("Failed to decode icon {}", url_owned);
                    return;
                }
            };

            if let Some(parent) = disk_path.parent() {
                let _ = tokio::fs::create_dir_all(parent).await;
            }
            // best-effort: a failed write just means re-fetch next launch.
            // report the *thumbnail* size so the budget matches what's on
            // disk, not the raw fetch size.
            if tokio::fs::write(&disk_path, &png).await.is_ok() {
                record_cache_write(png.len() as u64);
            }

            if let Ok(mut slot) = pending.lock() {
                slot.push(PendingIcon { url: url_owned, image });
                crate::tui::request_redraw();
            }
        });
    }

    /// turn any freshly-decoded images into terminal protocols. call once
    /// per tick from the event loop, same as ContentListState::drain_image_loads
    /// - protocol construction touches the picker/terminal state so it has
    /// to happen on the main thread, not the background fetch task.
    pub fn drain(&mut self, picker: &ratatui_image::picker::Picker) {
        let items = match self.pending.lock() {
            Ok(mut pending) => std::mem::take(&mut *pending),
            Err(_) => return,
        };
        for item in items {
            self.protocols.insert(item.url, picker.new_resize_protocol(item.image));
        }
    }
}

/// terminal-cell width for a square icon given its row height — keeps
/// icons visually square though terminal cells aren't. same math as
/// content/list.rs's square_icon_columns, duplicated since that one is
/// tied to ContentEntry's icon_lines and isn't worth the coupling.
pub fn square_icon_columns(rows: u16, font_size: (u16, u16)) -> u16 {
    let width = u32::from(font_size.0.max(1));
    let height = u32::from(font_size.1.max(1));
    ((u32::from(rows) * height + width / 2) / width).max(1) as u16
}
