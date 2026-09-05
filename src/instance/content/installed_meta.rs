// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// tracks which catalog project (Modrinth/CurseForge, see ModpackHit::source_key)
// is responsible for which installed filename in an instance. mod/resourcepack
// filenames aren't stable across versions (e.g. sodium-fabric-0.5.jar ->
// sodium-fabric-0.6.jar), so without this there's no way to tell "this hit is
// already installed" or to replace the old file on reinstall instead of just
// adding a second copy alongside it.
//
// stored as a small hidden JSON sidecar at the instance root (the .minecraft
// dir) so mods and resource packs share one record and the file doesn't sit
// inside the content folders users actually look at. callers hand us the
// content dir they're working with; the root is its parent. sidecars written
// directly into a content dir by older builds are still read (and merged into
// the root file on the next write) so existing badges survive the move.
//
// it's filtered out of every content scan automatically since those only look
// at files matching a specific extension (.jar/.zip), never this one.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

const META_FILENAME: &str = ".alloy-installed.json";

// the instance root that owns the sidecar for `content_dir`
// (.minecraft/mods or .minecraft/resourcepacks -> .minecraft).
fn root_dir(content_dir: &Path) -> &Path {
    content_dir.parent().unwrap_or(content_dir)
}

fn sidecar_path(content_dir: &Path) -> PathBuf {
    root_dir(content_dir).join(META_FILENAME)
}

// where older builds kept the sidecar: directly in the content dir.
fn legacy_path(content_dir: &Path) -> PathBuf {
    content_dir.join(META_FILENAME)
}

fn read_map(path: &Path) -> HashMap<String, String> {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|data| serde_json::from_str(&data).unwrap_or_default())
        .unwrap_or_default()
}

// missing/unreadable/corrupt file just means "nothing tracked yet" rather
// than an error - this is best-effort bookkeeping, not a source of truth
// (the actual files on disk are).
pub fn load(content_dir: &Path) -> HashMap<String, String> {
    let mut map = read_map(&legacy_path(content_dir));
    // the root sidecar wins where keys overlap (it's the newer record).
    map.extend(read_map(&sidecar_path(content_dir)));
    map
}

// records that `key` is now installed as `filename`, returning the
// previously-recorded filename for that key if it differed (so the caller
// can delete the stale file left over from an older version).
pub fn record(content_dir: &Path, key: &str, filename: &str) -> Option<String> {
    let path = sidecar_path(content_dir);
    let mut map = read_map(&path);
    let previous = map.insert(key.to_string(), filename.to_string());

    // migrate: fold any legacy per-dir sidecar into the root record and
    // remove it, so the old location doesn't linger as a stale second copy.
    let legacy = legacy_path(content_dir);
    if legacy != path {
        for (k, v) in read_map(&legacy) {
            map.entry(k).or_insert(v);
        }
        if let Err(e) = std::fs::remove_file(&legacy) {
            tracing::debug!("No legacy sidecar to clean up at {}: {}", legacy.display(), e);
        }
    }

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        if let Err(e) = std::fs::write(&path, json) {
            tracing::warn!(
                "Failed to write installed-content metadata at {}: {}",
                path.display(),
                e
            );
        }
    }

    previous.filter(|old| old != filename)
}

#[cfg(test)]
mod tests {
    use super::*;

    // load()/record() take the content dir but must place the sidecar at
    // the instance root (its parent) and migrate legacy per-dir sidecars.

    #[test]
    fn record_writes_to_instance_root_not_content_dir() {
        let tmp = tempfile::TempDir::new().unwrap();
        let content_dir = tmp.path().join(".minecraft").join("mods");
        std::fs::create_dir_all(&content_dir).unwrap();

        record(&content_dir, "modrinth:abc", "sodium-0.6.jar");

        assert!(
            !content_dir.join(META_FILENAME).exists(),
            "sidecar must not sit in the content dir"
        );
        let map = load(&content_dir);
        assert_eq!(map.get("modrinth:abc").map(String::as_str), Some("sodium-0.6.jar"));
    }

    #[test]
    fn record_returns_previous_filename_for_same_key() {
        let tmp = tempfile::TempDir::new().unwrap();
        let content_dir = tmp.path().join(".minecraft").join("resourcepacks");
        std::fs::create_dir_all(&content_dir).unwrap();

        assert_eq!(record(&content_dir, "curseforge:1", "pack-v1.zip"), None);
        assert_eq!(
            record(&content_dir, "curseforge:1", "pack-v2.zip"),
            Some("pack-v1.zip".to_string())
        );
    }

    #[test]
    fn legacy_sidecar_is_merged_and_removed_on_next_write() {
        let tmp = tempfile::TempDir::new().unwrap();
        let content_dir = tmp.path().join(".minecraft").join("mods");
        std::fs::create_dir_all(&content_dir).unwrap();

        // simulate an older build's sidecar in the content dir
        let legacy_json = r#"{"modrinth:old": "old.jar"}"#;
        std::fs::write(content_dir.join(META_FILENAME), legacy_json).unwrap();

        // a fresh install lands in the root file and migrates the old one
        record(&content_dir, "modrinth:new", "new.jar");

        assert!(!content_dir.join(META_FILENAME).exists(), "legacy file removed");
        let map = load(&content_dir);
        assert_eq!(map.get("modrinth:old").map(String::as_str), Some("old.jar"));
        assert_eq!(map.get("modrinth:new").map(String::as_str), Some("new.jar"));
    }

    #[test]
    fn load_prefers_root_record_over_stale_legacy_entries() {
        let tmp = tempfile::TempDir::new().unwrap();
        let content_dir = tmp.path().join(".minecraft").join("mods");
        std::fs::create_dir_all(&content_dir).unwrap();

        std::fs::write(
            content_dir.join(META_FILENAME),
            r#"{"modrinth:x": "legacy.jar"}"#,
        )
        .unwrap();
        std::fs::write(
            tmp.path().join(".minecraft").join(META_FILENAME),
            r#"{"modrinth:x": "current.jar"}"#,
        )
        .unwrap();

        let map = load(&content_dir);
        assert_eq!(map.get("modrinth:x").map(String::as_str), Some("current.jar"));
    }
}
