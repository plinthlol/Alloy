// CurseForge Core API (v1) client. unlike modrinth, every request needs an
// `x-api-key` header — no anonymous tier. the key comes from
// config::SETTINGS.curseforge.effective_api_key(); each fn also fails with
// CurseForgeError::MissingApiKey on an empty key so stale UI state can't
// fire a keyless request that would just 403.
//
// NOTE: the classId values and field names below are the commonly documented
// ones for Minecraft (gameId 432), written without live API access to verify
// against — sanity-check the first real response (`GET /v1/games/432/categories`
// confirms classIds) before trusting this in production.

use serde::Deserialize;
use std::collections::HashMap;
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use thiserror::Error;

use crate::instance::models::ModLoader;
use crate::net::{HttpClient, NetError, download_file};
use std::path::Path;

const CF_API_BASE: &str = "https://api.curseforge.com/v1";

const SEARCH_CACHE_TTL: Duration = Duration::from_secs(60);
const FILES_CACHE_TTL: Duration = Duration::from_secs(300);
const MAX_CACHE_ENTRIES: usize = 200;

static SEARCH_CACHE: LazyLock<Mutex<HashMap<String, (Instant, SearchResponse)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FILES_CACHE: LazyLock<Mutex<HashMap<String, (Instant, FilesResponse)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
static FILE_CACHE: LazyLock<Mutex<HashMap<String, (Instant, ModFile)>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

fn cache_get<T: Clone>(
    map: &Mutex<HashMap<String, (Instant, T)>>,
    key: &str,
    ttl: Duration,
) -> Option<T> {
    map.lock()
        .ok()?
        .get(key)
        .filter(|(at, _)| at.elapsed() < ttl)
        .map(|(_, v)| v.clone())
}

fn cache_put<T>(map: &Mutex<HashMap<String, (Instant, T)>>, key: String, value: T) {
    if let Ok(mut guard) = map.lock() {
        if guard.len() >= MAX_CACHE_ENTRIES {
            guard.clear();
        }
        guard.insert(key, (Instant::now(), value));
    }
}
pub const MINECRAFT_GAME_ID: u32 = 432;

pub const CLASS_ID_MODPACK: u32 = 4471;
pub const CLASS_ID_MOD: u32 = 6;
pub const CLASS_ID_RESOURCE_PACK: u32 = 12;
pub const CLASS_ID_WORLD: u32 = 17;

#[derive(Debug, Error)]
pub enum CurseForgeError {
    #[error("no CurseForge API key configured — add one under [curseforge] in config.toml")]
    MissingApiKey,
    #[error(transparent)]
    Net(#[from] NetError),
}

fn loader_type(loader: ModLoader) -> Option<u32> {
    // CurseForge's ModLoaderType enum. 0 = Any, so omit rather than send it
    // (an unfiltered search should stay unfiltered).
    match loader {
        ModLoader::Vanilla => None,
        ModLoader::Forge => Some(1),
        ModLoader::Fabric => Some(4),
        ModLoader::Quilt => Some(5),
        ModLoader::NeoForge => Some(6),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub data: Vec<Mod>,
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pagination {
    pub index: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
    #[serde(rename = "resultCount")]
    pub result_count: u32,
    #[serde(rename = "totalCount")]
    pub total_count: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Mod {
    pub id: u32,
    pub name: String,
    pub slug: String,
    #[serde(default)]
    pub summary: String,
    #[serde(rename = "downloadCount", default)]
    pub download_count: u64,
    #[serde(rename = "classId", default)]
    pub class_id: Option<u32>,
    #[serde(default)]
    pub logo: Option<ModAsset>,
    #[serde(default)]
    pub authors: Vec<ModAuthor>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModAsset {
    #[serde(rename = "thumbnailUrl")]
    pub thumbnail_url: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModAuthor {
    pub name: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FilesResponse {
    pub data: Vec<ModFile>,
    pub pagination: Pagination,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ModFile {
    pub id: u32,
    #[serde(rename = "modId")]
    pub mod_id: u32,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(rename = "fileName")]
    pub file_name: String,
    // null when the author disabled third-party distribution — the file is
    // still listed, just not downloadable. callers fall back to a project
    // page link then.
    #[serde(rename = "downloadUrl")]
    pub download_url: Option<String>,
    #[serde(rename = "gameVersions", default)]
    pub game_versions: Vec<String>,
    #[serde(rename = "fileLength", default)]
    pub file_length: u64,
    // release channel per CurseForge's file model: 1 = release, 2 = beta,
    // 3 = alpha. Option + default so a missing field degrades to "release"
    // rather than failing the whole listing (the model was written without
    // live API verification, same caveat as the fields above).
    #[serde(rename = "releaseType", default)]
    pub release_type: Option<u32>,
}

// the project's long-form description, returned by CurseForge as HTML.
// the markdown renderer's normalize_html converts it, so the raw string is
// passed through untouched. needed as a separate call — the search hit and
// file listings never carry the body.
#[derive(Debug, Clone, Deserialize)]
struct DescriptionResponse {
    data: String,
}

pub async fn get_description(
    client: &HttpClient,
    api_key: &str,
    mod_id: u32,
) -> Result<String, CurseForgeError> {
    get_description_from(client, CF_API_BASE, api_key, mod_id).await
}

pub async fn get_description_from(
    client: &HttpClient,
    api_base: &str,
    api_key: &str,
    mod_id: u32,
) -> Result<String, CurseForgeError> {
    require_key(api_key)?;
    let url = format!("{api_base}/mods/{mod_id}/description?raw=true");
    tracing::debug!("Fetching CurseForge description for mod {}", mod_id);
    let resp: DescriptionResponse = client
        .get_json_with_headers(&url, &[("x-api-key", api_key)])
        .await?;
    Ok(resp.data)
}

fn require_key(api_key: &str) -> Result<(), CurseForgeError> {
    if api_key.is_empty() {
        Err(CurseForgeError::MissingApiKey)
    } else {
        Ok(())
    }
}

// searches CurseForge projects, mirroring modrinth::search's shape:
// `class_id` (CLASS_ID_*) plays the role of modrinth's `project_type`.
#[allow(clippy::too_many_arguments)]
pub async fn search(
    client: &HttpClient,
    api_key: &str,
    query: &str,
    class_id: u32,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
    index: u32,
    page_size: u32,
) -> Result<SearchResponse, CurseForgeError> {
    search_from(
        client,
        CF_API_BASE,
        api_key,
        query,
        class_id,
        game_version,
        loader,
        index,
        page_size,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn search_from(
    client: &HttpClient,
    api_base: &str,
    api_key: &str,
    query: &str,
    class_id: u32,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
    index: u32,
    page_size: u32,
) -> Result<SearchResponse, CurseForgeError> {
    require_key(api_key)?;

    let mut url = format!(
        "{}/mods/search?gameId={}&classId={}&searchFilter={}&index={}&pageSize={}&sortField=2&sortOrder=desc",
        api_base,
        MINECRAFT_GAME_ID,
        class_id,
        urlencode(query),
        index,
        page_size,
    );
    if let Some(gv) = game_version {
        url.push_str(&format!("&gameVersion={}", urlencode(gv)));
    }
    if let Some(lt) = loader.and_then(loader_type) {
        url.push_str(&format!("&modLoaderType={lt}"));
    }

    if let Some(cached) = cache_get(&SEARCH_CACHE, &url, SEARCH_CACHE_TTL) {
        return Ok(cached);
    }

    tracing::debug!("Searching CurseForge: {}", url);
    let resp: SearchResponse = client
        .get_json_with_headers(&url, &[("x-api-key", api_key)])
        .await?;
    tracing::debug!(
        "CurseForge search returned {} hit(s) of {} total",
        resp.data.len(),
        resp.pagination.total_count
    );
    cache_put(&SEARCH_CACHE, url, resp.clone());
    Ok(resp)
}

pub async fn get_files(
    client: &HttpClient,
    api_key: &str,
    mod_id: u32,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
) -> Result<FilesResponse, CurseForgeError> {
    get_files_from(client, CF_API_BASE, api_key, mod_id, game_version, loader).await
}

#[derive(Debug, Clone, Deserialize)]
struct FileResponse {
    data: ModFile,
}

// resolves one (modId, fileId) pair to file metadata — exactly what a
// modpack manifest gives you per entry, unlike `search` which finds mods by
// name. modrinth's equivalent is get_version; every entry in a pack's
// `files` list gets one call here to become a download URL.
pub async fn get_file(
    client: &HttpClient,
    api_key: &str,
    mod_id: u32,
    file_id: u32,
) -> Result<ModFile, CurseForgeError> {
    get_file_from(client, CF_API_BASE, api_key, mod_id, file_id).await
}

pub async fn get_file_from(
    client: &HttpClient,
    api_base: &str,
    api_key: &str,
    mod_id: u32,
    file_id: u32,
) -> Result<ModFile, CurseForgeError> {
    require_key(api_key)?;
    let url = format!("{api_base}/mods/{mod_id}/files/{file_id}");
    if let Some(cached) = cache_get(&FILE_CACHE, &url, FILES_CACHE_TTL) {
        return Ok(cached);
    }
    tracing::debug!("Fetching CurseForge file {} for mod {}", file_id, mod_id);
    let resp: FileResponse = client
        .get_json_with_headers(&url, &[("x-api-key", api_key)])
        .await?;
    cache_put(&FILE_CACHE, url, resp.data.clone());
    Ok(resp.data)
}

pub async fn get_files_from(
    client: &HttpClient,
    api_base: &str,
    api_key: &str,
    mod_id: u32,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
) -> Result<FilesResponse, CurseForgeError> {
    require_key(api_key)?;

    let mut url = format!("{}/mods/{}/files?", api_base, mod_id);
    if let Some(gv) = game_version {
        url.push_str(&format!("gameVersion={}&", urlencode(gv)));
    }
    if let Some(lt) = loader.and_then(loader_type) {
        url.push_str(&format!("modLoaderType={lt}&"));
    }

    if let Some(cached) = cache_get(&FILES_CACHE, &url, FILES_CACHE_TTL) {
        return Ok(cached);
    }

    tracing::debug!("Fetching CurseForge files for mod {}", mod_id);
    let resp: FilesResponse = client
        .get_json_with_headers(&url, &[("x-api-key", api_key)])
        .await?;
    cache_put(&FILES_CACHE, url, resp.clone());
    Ok(resp)
}

// downloads a file via its download_url. returns Ok(false) when the author
// disabled third-party downloads — normal on CurseForge, not an error; the
// caller shows an "open on CurseForge" link instead.
pub async fn download_mod_file(
    client: &HttpClient,
    file: &ModFile,
    dest: &Path,
    progress_cb: impl Fn(u64, u64),
) -> Result<bool, NetError> {
    let Some(url) = &file.download_url else {
        tracing::info!(
            "CurseForge file {} ({}) has no downloadUrl - author has disabled third-party distribution",
            file.id,
            file.file_name
        );
        return Ok(false);
    };
    tracing::info!("Downloading CurseForge file {} ({})", file.id, file.file_name);
    download_file(client, url, dest, progress_cb).await?;
    Ok(true)
}

fn urlencode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{:02X}", byte)),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn search_without_key_fails_fast_without_a_request() {
        let client = HttpClient::new();
        let err = search(&client, "", "sodium", CLASS_ID_MOD, None, None, 0, 20)
            .await
            .unwrap_err();
        assert!(matches!(err, CurseForgeError::MissingApiKey));
    }

    #[tokio::test]
    async fn get_files_without_key_fails_fast_without_a_request() {
        let client = HttpClient::new();
        let err = get_files(&client, "", 12345, None, None).await.unwrap_err();
        assert!(matches!(err, CurseForgeError::MissingApiKey));
    }

    #[tokio::test]
    async fn get_file_without_key_fails_fast_without_a_request() {
        let client = HttpClient::new();
        let err = get_file(&client, "", 12345, 67890).await.unwrap_err();
        assert!(matches!(err, CurseForgeError::MissingApiKey));
    }

    #[test]
    fn loader_type_maps_known_loaders() {
        assert_eq!(loader_type(ModLoader::Fabric), Some(4));
        assert_eq!(loader_type(ModLoader::Vanilla), None);
    }

    #[tokio::test]
    #[ignore = "hits live CurseForge API - needs a real key via CF_TEST_API_KEY"]
    async fn test_search_modpacks() {
        let key = std::env::var("CF_TEST_API_KEY").expect("set CF_TEST_API_KEY to run this test");
        let client = HttpClient::new();
        let resp = search(
            &client,
            &key,
            "",
            CLASS_ID_MODPACK,
            Some("1.20.1"),
            Some(ModLoader::Fabric),
            0,
            10,
        )
        .await
        .unwrap();
        assert!(!resp.data.is_empty());
    }
}
