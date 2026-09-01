// downloads a Temurin (Eclipse Adoptium) Java runtime when no compatible
// java is installed. instance::manager::create_inner decides *when* to call
// this; the actual fetching lives here in net/ because it's just HTTP.

use std::path::{Path, PathBuf};

use serde::Deserialize;
use thiserror::Error;

use super::{HttpClient, NetError, download_file};

const ADOPTIUM_API_BASE: &str = "https://api.adoptium.net/v3/assets/latest";

#[derive(Debug, Error)]
pub enum JavaProvisionError {
    #[error("network error while provisioning Java: {0}")]
    Net(#[from] NetError),
    #[error("IO error while provisioning Java: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to extract Java archive: {0}")]
    Extract(String),
    #[error("no Adoptium build found for Java {feature_version} ({image_type}) on {os}/{arch}")]
    NoRelease {
        feature_version: u32,
        image_type: &'static str,
        os: &'static str,
        arch: &'static str,
    },
    #[error("unsupported CPU architecture for automatic Java downloads: {0}")]
    UnsupportedArch(String),
    #[error("downloaded Java archive but couldn't find a java binary inside it")]
    BinaryNotFound,
}

// JRE is enough to run the game, but Forge/NeoForge installers sometimes
// shell out to javac, so manager picks Jdk for those loaders and Jre
// otherwise.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageType {
    Jre,
    Jdk,
}

impl ImageType {
    pub fn as_str(self) -> &'static str {
        match self {
            ImageType::Jre => "jre",
            ImageType::Jdk => "jdk",
        }
    }
}

fn adoptium_os() -> &'static str {
    if cfg!(target_os = "windows") {
        "windows"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    }
}

fn adoptium_arch() -> Result<&'static str, JavaProvisionError> {
    match std::env::consts::ARCH {
        "x86_64" => Ok("x64"),
        "x86" => Ok("x86"),
        "aarch64" => Ok("aarch64"),
        "arm" => Ok("arm"),
        other => Err(JavaProvisionError::UnsupportedArch(other.to_string())),
    }
}

#[derive(Debug, Deserialize)]
struct AdoptiumRelease {
    binary: AdoptiumBinary,
}

#[derive(Debug, Deserialize)]
struct AdoptiumBinary {
    package: AdoptiumPackage,
}

#[derive(Debug, Deserialize)]
struct AdoptiumPackage {
    link: String,
}

// runtimes unpack to <data_dir>/alloy/bin/<jre|jdk>-<major>, shared across
// every instance that needs that combo — each one is only downloaded once.
#[must_use]
pub fn runtime_install_dir(feature_version: u32, image_type: ImageType) -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("alloy")
        .join("bin")
        .join(format!("{}-{}", image_type.as_str(), feature_version))
}

// finds an already-provisioned runtime for this (major, image_type) combo,
// if we downloaded one before. doesn't touch the network.
#[must_use]
pub fn find_provisioned_java(feature_version: u32, image_type: ImageType) -> Option<String> {
    find_java_binary(&runtime_install_dir(feature_version, image_type))
}

// downloads and unpacks a runtime for the given major Java version and
// returns the path to its `java` binary. idempotent: reuses an existing
// install for the same (major, image_type) combo.
pub async fn provision_java(
    client: &HttpClient,
    feature_version: u32,
    image_type: ImageType,
) -> Result<String, JavaProvisionError> {
    let install_dir = runtime_install_dir(feature_version, image_type);
    if let Some(existing) = find_provisioned_java(feature_version, image_type) {
        tracing::debug!(
            "Reusing already-provisioned Java {} ({}) at {}",
            feature_version,
            image_type.as_str(),
            existing
        );
        return Ok(existing);
    }

    let os = adoptium_os();
    let arch = adoptium_arch()?;
    let url = format!(
        "{ADOPTIUM_API_BASE}/{feature_version}/hotspot?architecture={arch}&image_type={}&os={os}&vendor=eclipse",
        image_type.as_str(),
    );

    tracing::info!(
        "Provisioning Java {} ({}) for {}/{}",
        feature_version,
        image_type.as_str(),
        os,
        arch
    );
    crate::tui::progress::set_action(format!(
        "Downloading Java {feature_version} ({})",
        image_type.as_str()
    ));

    let releases: Vec<AdoptiumRelease> = client.get_json(&url).await.map_err(|e| match e {
        NetError::StatusError { status, .. } if status == 404 => JavaProvisionError::NoRelease {
            feature_version,
            image_type: image_type.as_str(),
            os,
            arch,
        },
        other => JavaProvisionError::Net(other),
    })?;

    let package_url = releases
        .into_iter()
        .next()
        .map(|r| r.binary.package.link)
        .ok_or(JavaProvisionError::NoRelease {
            feature_version,
            image_type: image_type.as_str(),
            os,
            arch,
        })?;

    // stage into a sibling `.download` dir first, so a failed provision
    // never leaves a half-extracted runtime at the real install path.
    let staging_dir = install_dir.with_extension("download");
    if staging_dir.exists() {
        std::fs::remove_dir_all(&staging_dir)?;
    }
    std::fs::create_dir_all(&staging_dir)?;

    let archive_name = if os == "windows" { "java.zip" } else { "java.tar.gz" };
    let archive_path = staging_dir.join(archive_name);

    download_file(client, &package_url, &archive_path, |downloaded, total| {
        if total > 0 {
            crate::tui::progress::set_progress(downloaded, total);
        }
        crate::tui::progress::set_sub_action(format!(
            "Java runtime ({} / {})",
            format_bytes(downloaded),
            if total > 0 {
                format_bytes(total)
            } else {
                "?".to_string()
            }
        ));
    })
    .await?;

    extract_archive(&archive_path, &staging_dir, os == "windows")
        .map_err(|e| JavaProvisionError::Extract(e.to_string()))?;
    let _ = std::fs::remove_file(&archive_path);

    if install_dir.exists() {
        std::fs::remove_dir_all(&install_dir)?;
    }
    if let Some(parent) = install_dir.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::rename(&staging_dir, &install_dir)?;

    crate::tui::progress::clear();

    find_java_binary(&install_dir).ok_or(JavaProvisionError::BinaryNotFound)
}

fn extract_archive(archive_path: &Path, dest: &Path, is_zip: bool) -> Result<(), std::io::Error> {
    if is_zip {
        let file = std::fs::File::open(archive_path)?;
        let mut archive = zip::ZipArchive::new(file).map_err(|e| std::io::Error::other(e.to_string()))?;
        archive
            .extract(dest)
            .map_err(|e| std::io::Error::other(e.to_string()))?;
    } else {
        let file = std::fs::File::open(archive_path)?;
        let decoder = flate2::read::GzDecoder::new(file);
        tar::Archive::new(decoder).unpack(dest)?;
    }
    Ok(())
}

// adoptium archives extract to a top-level dir like `jdk-21.0.4+7-jre`
// (macOS puts the binary under `Contents/Home/bin/java`). walk a level or
// two down instead of assuming a fixed layout.
pub(crate) fn find_java_binary(root: &Path) -> Option<String> {
    let java_name = if cfg!(windows) { "java.exe" } else { "java" };

    if let Some(found) = check_java_layout(root, java_name) {
        return Some(found);
    }

    let entries = std::fs::read_dir(root).ok()?;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir()
            && let Some(found) = check_java_layout(&path, java_name)
        {
            return Some(found);
        }
    }
    None
}

fn check_java_layout(dir: &Path, java_name: &str) -> Option<String> {
    let direct = dir.join("bin").join(java_name);
    if direct.is_file() {
        return Some(ensure_executable(direct));
    }
    let mac_bundle = dir.join("Contents").join("Home").join("bin").join(java_name);
    if mac_bundle.is_file() {
        return Some(ensure_executable(mac_bundle));
    }
    None
}

#[cfg(unix)]
fn ensure_executable(path: PathBuf) -> String {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(&path) {
        let mut perms = meta.permissions();
        perms.set_mode(perms.mode() | 0o755);
        let _ = std::fs::set_permissions(&path, perms);
    }
    path.to_string_lossy().into_owned()
}

#[cfg(not(unix))]
fn ensure_executable(path: PathBuf) -> String {
    path.to_string_lossy().into_owned()
}

fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut size = bytes as f64;
    let mut unit = 0;
    while size >= 1024.0 && unit < UNITS.len() - 1 {
        size /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{size:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_install_dir_includes_image_type_and_version() {
        let dir = runtime_install_dir(21, ImageType::Jdk);
        assert_eq!(dir.file_name().unwrap(), "jdk-21");
    }

    #[test]
    fn runtime_install_dir_differs_by_image_type() {
        let jre = runtime_install_dir(17, ImageType::Jre);
        let jdk = runtime_install_dir(17, ImageType::Jdk);
        assert_ne!(jre, jdk);
    }

    #[test]
    fn format_bytes_scales_units() {
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(2048), "2.0 KB");
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MB");
    }

    #[test]
    fn find_java_binary_none_when_empty_dir() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(find_java_binary(tmp.path()).is_none());
    }

    #[test]
    fn find_java_binary_finds_direct_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let bin = tmp.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let java_name = if cfg!(windows) { "java.exe" } else { "java" };
        std::fs::write(bin.join(java_name), b"#!/bin/sh\n").unwrap();
        assert!(find_java_binary(tmp.path()).is_some());
    }

    #[test]
    fn find_java_binary_finds_nested_archive_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp.path().join("jdk-21.0.4+7-jre").join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        let java_name = if cfg!(windows) { "java.exe" } else { "java" };
        std::fs::write(nested.join(java_name), b"#!/bin/sh\n").unwrap();
        assert!(find_java_binary(tmp.path()).is_some());
    }

    #[test]
    fn find_java_binary_finds_macos_bundle_layout() {
        let tmp = tempfile::tempdir().unwrap();
        let nested = tmp
            .path()
            .join("jdk-21.0.4+7-jre")
            .join("Contents")
            .join("Home")
            .join("bin");
        std::fs::create_dir_all(&nested).unwrap();
        let java_name = if cfg!(windows) { "java.exe" } else { "java" };
        std::fs::write(nested.join(java_name), b"#!/bin/sh\n").unwrap();
        assert!(find_java_binary(tmp.path()).is_some());
    }
}
