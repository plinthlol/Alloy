// tracks which catalog project (Modrinth/CurseForge, see ModpackHit::source_key)
// is responsible for which installed filename in a content dir. mod/resourcepack
// filenames aren't stable across versions (e.g. sodium-fabric-0.5.jar ->
// sodium-fabric-0.6.jar), so without this there's no way to tell "this hit is
// already installed" or to replace the old file on reinstall instead of just
// adding a second copy alongside it.
//
// stored as a small hidden JSON sidecar right in the content dir. it's
// filtered out of every content scan automatically since those only look at
// files matching a specific extension (.jar/.zip), never this one.

use std::collections::HashMap;
use std::path::Path;

const META_FILENAME: &str = ".alloy-installed.json";

// key -> installed filename, for every project this popup has installed
// into this content dir. missing/unreadable/corrupt file just means
// "nothing tracked yet" rather than an error - this is best-effort
// bookkeeping, not a source of truth (the actual files on disk are).
pub fn load(dir: &Path) -> HashMap<String, String> {
    let path = dir.join(META_FILENAME);
    let Ok(data) = std::fs::read_to_string(path) else {
        return HashMap::new();
    };
    serde_json::from_str(&data).unwrap_or_default()
}

// records that `key` is now installed as `filename`, returning the
// previously-recorded filename for that key if it differed (so the caller
// can delete the stale file left over from an older version).
pub fn record(dir: &Path, key: &str, filename: &str) -> Option<String> {
    let mut map = load(dir);
    let previous = map.insert(key.to_string(), filename.to_string());

    if let Ok(json) = serde_json::to_string_pretty(&map) {
        let path = dir.join(META_FILENAME);
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
