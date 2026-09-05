// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// integration tests for the public log_files API.
// these tests touch the filesystem and exercise the module as an external
// consumer would.

use std::io::Write;
use std::path::{Path, PathBuf};

use alloy::instance::log_files::{log_dir, read_log_file, scan_log_files};

fn setup_log_dir(tmp: &Path, instance: &str) -> PathBuf {
    let dir = log_dir(tmp, instance);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_gzip(path: &Path, content: &str) {
    let file = std::fs::File::create(path).unwrap();
    let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
    encoder.write_all(content.as_bytes()).unwrap();
    encoder.finish().unwrap();
}

#[test]
fn scan_log_files_empty_dir() {
    let tmp = tempfile::tempdir().unwrap();
    setup_log_dir(tmp.path(), "inst");
    let entries = scan_log_files(tmp.path(), "inst");
    assert!(entries.is_empty());
}

#[test]
fn scan_log_files_finds_latest_and_rotated_archives() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = setup_log_dir(tmp.path(), "inst");
    std::fs::write(dir.join("latest.log"), "current session").unwrap();
    write_gzip(&dir.join("2026-08-01-1.log.gz"), "yesterday's session");
    write_gzip(&dir.join("2026-07-31-1.log.gz"), "the day before");

    let entries = scan_log_files(tmp.path(), "inst");
    assert_eq!(entries.len(), 3);
    // latest.log always sorts first, then rotated archives newest-first
    assert_eq!(entries[0].name, "latest.log");
    assert!(!entries[0].compressed);
    assert_eq!(entries[1].name, "2026-08-01-1.log.gz");
    assert!(entries[1].compressed);
    assert_eq!(entries[2].name, "2026-07-31-1.log.gz");
    assert!(entries[2].compressed);
}

#[test]
fn scan_log_files_ignores_debug_log_and_non_log_files() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = setup_log_dir(tmp.path(), "inst");
    std::fs::write(dir.join("notes.txt"), "not a log").unwrap();
    std::fs::write(dir.join("debug.log"), "forge/neoforge debug output").unwrap();
    write_gzip(&dir.join("debug-1.log.gz"), "rotated debug output");
    std::fs::write(dir.join("latest.log"), "log line").unwrap();

    let entries = scan_log_files(tmp.path(), "inst");
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].name, "latest.log");
}

#[test]
fn scan_log_files_missing_dir_returns_empty() {
    let tmp = tempfile::tempdir().unwrap();
    let entries = scan_log_files(tmp.path(), "ghost");
    assert!(entries.is_empty());
}

#[test]
fn read_log_file_returns_lines() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("latest.log");
    std::fs::write(&path, "alpha\nbeta\ngamma").unwrap();
    let lines = read_log_file(&path);
    assert_eq!(lines, vec!["alpha", "beta", "gamma"]);
}

#[test]
fn read_log_file_missing_returns_empty() {
    let lines = read_log_file(Path::new("/nonexistent/latest.log"));
    assert!(lines.is_empty());
}

#[test]
fn read_log_file_decompresses_gzip_archives() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("2026-08-01-1.log.gz");
    write_gzip(&path, "rotated line one\nrotated line two");

    let lines = read_log_file(&path);
    assert_eq!(lines, vec!["rotated line one", "rotated line two"]);
}

#[test]
fn read_log_file_gzip_missing_returns_empty() {
    let lines = read_log_file(Path::new("/nonexistent/2026-08-01-1.log.gz"));
    assert!(lines.is_empty());
}
