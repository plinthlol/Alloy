// reads Minecraft's own log directory instead of keeping a launcher-owned
// copy: `latest.log` is always the current session, and log4j rotates and
// gzips everything older into `<yyyy-MM-dd>-<n>.log.gz` on its own. we
// don't write anything here, just read what the game already writes.

use std::io::Read;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct LogFileEntry {
    pub name: String,
    pub path: PathBuf,
    pub compressed: bool,
}

pub fn log_dir(instances_dir: &Path, instance_name: &str) -> PathBuf {
    instances_dir
        .join(instance_name)
        .join(".minecraft")
        .join("logs")
}

// `latest.log` plus log4j's rotated archives (`<date>-<n>.log.gz`).
// `debug.log` (Forge/NeoForge only) is deliberately excluded for now — it's
// much noisier and not every loader produces it, so it'd clutter the list.
fn is_recognized_log_name(name: &str) -> bool {
    if name.starts_with("debug") {
        return false;
    }
    name == "latest.log" || name.ends_with(".log.gz")
}

pub fn scan_log_files(instances_dir: &Path, instance_name: &str) -> Vec<LogFileEntry> {
    let dir = log_dir(instances_dir, instance_name);

    let read_dir = match std::fs::read_dir(&dir) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    let mut entries: Vec<LogFileEntry> = read_dir
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.to_string();
            if is_recognized_log_name(&name) {
                let compressed = name.ends_with(".gz");
                Some(LogFileEntry {
                    name,
                    path,
                    compressed,
                })
            } else {
                None
            }
        })
        .collect();

    // descending string sort: "latest.log" outranks any "<date>-<n>.log.gz"
    // on its own (ASCII 'l' > any digit), and the ISO-ish date prefixes sort
    // the archives by recency after that.
    entries.sort_by(|a, b| b.name.cmp(&a.name));
    entries
}

pub fn read_log_file(path: &Path) -> Vec<String> {
    if path.extension().is_some_and(|e| e == "gz") {
        return read_gzip_log_file(path);
    }

    match std::fs::read_to_string(path) {
        Ok(content) => content.lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

fn read_gzip_log_file(path: &Path) -> Vec<String> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut decoder = flate2::read::GzDecoder::new(std::io::BufReader::new(file));
    let mut content = String::new();
    match decoder.read_to_string(&mut content) {
        Ok(_) => content.lines().map(|l| l.to_string()).collect(),
        Err(_) => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_dir_builds_correct_path() {
        let p = log_dir(Path::new("/instances"), "my-world");
        assert_eq!(p, PathBuf::from("/instances/my-world/.minecraft/logs"));
    }

    #[test]
    fn recognizes_latest_and_rotated_archives_only() {
        assert!(is_recognized_log_name("latest.log"));
        assert!(is_recognized_log_name("2026-08-01-1.log.gz"));
        assert!(!is_recognized_log_name("debug.log"));
        assert!(!is_recognized_log_name("debug-1.log.gz"));
        assert!(!is_recognized_log_name("notes.txt"));
    }
}
