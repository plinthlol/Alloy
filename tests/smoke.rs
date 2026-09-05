// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// verifies the lib/main split worked: integration tests can see crate items
// through the public API.

#[test]
fn lib_target_is_importable() {
    // touch one pure function from each major module so the linker fails if
    // any module went private by mistake during the split.
    assert!(alloy::net::maven_coord_to_path("a:b:1.0").is_some());
    let _ = alloy::config::SETTINGS.paths.effective_java_path();
}
