// system-detection helpers shared by launch and install paths. mojang names
// things differently from std::env::consts (macOS is "osx" in profile
// rules), so this is the single source of truth for translating.

pub fn mojang_os_name() -> &'static str {
    match std::env::consts::OS {
        "macos" => "osx",
        other => other,
    }
}

pub fn mojang_arch_name() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86",
        "x86_64" => "x86_64",
        "aarch64" => "arm64",
        other => other,
    }
}

// host OS version string. mojang rules occasionally constrain natives on
// os.version with a regex (e.g. macOS 10.x-only). stdlib doesn't expose
// this, so read it where it's cheap: linux via /proc/sys/kernel/osrelease,
// elsewhere empty. an empty string means version-gated rules don't match —
// the conservative default, and fine since os.version rules are rare.
pub fn mojang_os_version() -> String {
    #[cfg(target_os = "linux")]
    {
        if let Ok(s) = std::fs::read_to_string("/proc/sys/kernel/osrelease") {
            return s.trim().to_string();
        }
    }
    String::new()
}
