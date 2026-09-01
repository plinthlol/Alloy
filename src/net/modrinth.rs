// modrinth API v2 client: search, list versions, download files. keyless,
// unlike curseforge. like fabric.rs/quilt.rs, every public fn has a
// `_from(base)` variant so tests can point at a wiremock server.

use serde::{Deserialize, Serialize};

use crate::instance::models::ModLoader;
use crate::net::{HttpClient, NetError, download_file};
use std::path::Path;

const MODRINTH_API_BASE: &str = "https://api.modrinth.com/v2";

// modrinth loader facet strings. vanilla has no loader tag, so it maps to
// None and callers skip the filter.
fn loader_facet(loader: ModLoader) -> Option<&'static str> {
    match loader {
        ModLoader::Vanilla => None,
        ModLoader::Fabric => Some("fabric"),
        ModLoader::Forge => Some("forge"),
        ModLoader::NeoForge => Some("neoforge"),
        ModLoader::Quilt => Some("quilt"),
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchResponse {
    pub hits: Vec<SearchHit>,
    pub total_hits: u64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SearchHit {
    pub project_id: String,
    pub slug: String,
    pub title: String,
    pub description: String,
    #[serde(default)]
    pub author: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub icon_url: Option<String>,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub versions: Vec<String>,
    #[serde(default)]
    pub project_type: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ProjectVersion {
    pub id: String,
    pub project_id: String,
    pub name: String,
    pub version_number: String,
    pub game_versions: Vec<String>,
    pub loaders: Vec<String>,
    pub files: Vec<VersionFile>,
    #[serde(default)]
    pub dependencies: Vec<VersionDependency>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionFile {
    pub url: String,
    pub filename: String,
    #[serde(default)]
    pub primary: bool,
    #[serde(default)]
    pub size: u64,
    #[serde(default)]
    pub hashes: VersionHashes,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
pub struct VersionHashes {
    pub sha1: Option<String>,
    pub sha512: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VersionDependency {
    pub project_id: Option<String>,
    pub version_id: Option<String>,
    pub dependency_type: String,
}

// searches modrinth projects. `project_type` is "modpack" or "mod";
// game_version/loader narrow the results, offset/limit page through them.
#[allow(clippy::too_many_arguments)]
pub async fn search(
    client: &HttpClient,
    query: &str,
    project_type: &str,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
    offset: u32,
    limit: u32,
) -> Result<SearchResponse, NetError> {
    search_from(
        client,
        MODRINTH_API_BASE,
        query,
        project_type,
        game_version,
        loader,
        offset,
        limit,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn search_from(
    client: &HttpClient,
    api_base: &str,
    query: &str,
    project_type: &str,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
    offset: u32,
    limit: u32,
) -> Result<SearchResponse, NetError> {
    let mut facet_groups: Vec<String> = vec![format!("[\"project_type:{}\"]", project_type)];
    if let Some(gv) = game_version {
        facet_groups.push(format!("[\"versions:{}\"]", gv));
    }
    if let Some(l) = loader.and_then(loader_facet) {
        facet_groups.push(format!("[\"categories:{}\"]", l));
    }
    let facets = format!("[{}]", facet_groups.join(","));

    // an empty query (plain browse) has nothing to score relevance against,
    // so sort by downloads instead — a top-mods listing beats arbitrary order.
    let index = if query.trim().is_empty() { "downloads" } else { "relevance" };

    let url = format!(
        "{}/search?query={}&facets={}&offset={}&limit={}&index={}",
        api_base,
        urlencode(query),
        urlencode(&facets),
        offset,
        limit,
        index,
    );
    tracing::debug!("Searching Modrinth: {}", url);
    let resp: SearchResponse = client.get_json(&url).await?;
    tracing::debug!(
        "Modrinth search returned {} hit(s) of {} total",
        resp.hits.len(),
        resp.total_hits
    );
    Ok(resp)
}

// lists a project's versions, optionally filtered by game version/loader.
// the endpoint takes the same filters as query params, so we reuse them.
pub async fn get_project_versions(
    client: &HttpClient,
    project_id: &str,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
) -> Result<Vec<ProjectVersion>, NetError> {
    get_project_versions_from(client, MODRINTH_API_BASE, project_id, game_version, loader).await
}

pub async fn get_project_versions_from(
    client: &HttpClient,
    api_base: &str,
    project_id: &str,
    game_version: Option<&str>,
    loader: Option<ModLoader>,
) -> Result<Vec<ProjectVersion>, NetError> {
    let mut url = format!("{}/project/{}/version?", api_base, project_id);
    if let Some(gv) = game_version {
        url.push_str(&format!("game_versions=[\"{}\"]&", urlencode(gv)));
    }
    if let Some(l) = loader.and_then(loader_facet) {
        url.push_str(&format!("loaders=[\"{}\"]&", l));
    }
    tracing::debug!("Fetching Modrinth versions for project {}", project_id);
    let versions: Vec<ProjectVersion> = client.get_json(&url).await?;
    tracing::debug!(
        "Fetched {} Modrinth version(s) for project {}",
        versions.len(),
        project_id
    );
    Ok(versions)
}

pub async fn get_version(client: &HttpClient, version_id: &str) -> Result<ProjectVersion, NetError> {
    get_version_from(client, MODRINTH_API_BASE, version_id).await
}

pub async fn get_version_from(
    client: &HttpClient,
    api_base: &str,
    version_id: &str,
) -> Result<ProjectVersion, NetError> {
    let url = format!("{}/version/{}", api_base, version_id);
    tracing::debug!("Fetching Modrinth version {}", version_id);
    client.get_json(&url).await
}

// downloads a version's primary file (first file as fallback — modrinth
// always flags one, but don't trust that blindly) to `dest`. used for plain
// mod jars and `.mrpack` files alike.
pub async fn download_primary_file(
    client: &HttpClient,
    version: &ProjectVersion,
    dest: &Path,
    progress_cb: impl Fn(u64, u64),
) -> Result<(), NetError> {
    let file = version
        .files
        .iter()
        .find(|f| f.primary)
        .or_else(|| version.files.first())
        .ok_or_else(|| NetError::Parse(format!("Version {} has no files", version.id)))?;

    tracing::info!("Downloading Modrinth file {} for {}", file.filename, version.id);
    download_file(client, &file.url, dest, progress_cb).await
}

// minimal percent-encoding: our query params only hold ascii we produce
// plus user search text, so a full RFC 3986 impl would be overkill.
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

    #[test]
    fn urlencode_leaves_safe_chars_alone() {
        assert_eq!(urlencode("fabric-1.20.1"), "fabric-1.20.1");
    }

    #[test]
    fn urlencode_escapes_special_chars() {
        assert_eq!(urlencode("[\"a\"]"), "%5B%22a%22%5D");
        assert_eq!(urlencode("hello world"), "hello%20world");
    }

    #[test]
    fn loader_facet_maps_known_loaders() {
        assert_eq!(loader_facet(ModLoader::Fabric), Some("fabric"));
        assert_eq!(loader_facet(ModLoader::Vanilla), None);
    }

    #[tokio::test]
    #[ignore = "hits live Modrinth API"]
    async fn test_search_modpacks() {
        let client = HttpClient::new();
        let resp = search(&client, "", "modpack", Some("1.20.1"), Some(ModLoader::Fabric), 0, 10)
            .await
            .unwrap();
        assert!(!resp.hits.is_empty());
    }
}
