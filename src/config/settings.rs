// config structs mapped from config.toml. everything has sane defaults so
// a blank (or missing) file still works.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize, Default, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ImageProtocol {
    Halfblocks,
    Quadrants,
    Sixel,
    #[default]
    Kitty,
    Iterm2,
}

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct General {}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Paths {
    #[serde(default = "default_instances_dir")]
    pub instances_dir: String,
    #[serde(default = "default_meta_dir")]
    pub meta_dir: String,
    #[serde(default)]
    pub java_path: Option<String>,
}

fn default_instances_dir() -> String {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("alloy")
        .join("instances")
        .to_string_lossy()
        .into_owned()
}

fn default_meta_dir() -> String {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("alloy")
        .join("meta")
        .to_string_lossy()
        .into_owned()
}

impl Default for Paths {
    fn default() -> Self {
        Self {
            instances_dir: default_instances_dir(),
            meta_dir: default_meta_dir(),
            java_path: None,
        }
    }
}

// expand ~ in paths since toml doesn't do that for us
pub fn resolve_path(raw: &str) -> PathBuf {
    if let Some(stripped) = raw.strip_prefix("~/") {
        if let Some(home) = dirs::home_dir() {
            return home.join(stripped);
        }
    } else if raw == "~"
        && let Some(home) = dirs::home_dir()
    {
        return home;
    }
    PathBuf::from(raw)
}

impl Paths {
    pub fn effective_java_path(&self) -> Option<&str> {
        self.java_path.as_deref().filter(|s| !s.is_empty())
    }

    pub fn resolve_instances_dir(&self) -> PathBuf {
        resolve_path(&self.instances_dir)
    }

    pub fn resolve_meta_dir(&self) -> PathBuf {
        resolve_path(&self.meta_dir)
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Defaults {
    #[serde(default = "default_memory_min")]
    pub memory_min: String,
    #[serde(default = "default_memory_max")]
    pub memory_max: String,
}

fn default_memory_min() -> String {
    "512M".to_owned()
}
fn default_memory_max() -> String {
    "2G".to_owned()
}

impl Default for Defaults {
    fn default() -> Self {
        Self {
            memory_min: default_memory_min(),
            memory_max: default_memory_max(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
// error-toast timing: show 5s, start sliding at 3.5s, fly off over 300ms.
// tweak if toasts feel too fast or slow.
pub struct Ui {
    #[serde(default)]
    pub image_protocol: ImageProtocol,
    #[serde(default = "default_error_auto_dismiss_ms")]
    pub error_auto_dismiss_ms: u64,
    #[serde(default = "default_error_slide_start_ms")]
    pub error_slide_start_ms: u64,
    #[serde(default = "default_error_fly_out_ms")]
    pub error_fly_out_ms: u64,
    #[serde(default = "default_max_error_events")]
    pub max_error_events: usize,
}

fn default_error_auto_dismiss_ms() -> u64 {
    5000
}
fn default_error_slide_start_ms() -> u64 {
    3500
}
fn default_error_fly_out_ms() -> u64 {
    300
}
fn default_max_error_events() -> usize {
    50
}

impl Default for Ui {
    fn default() -> Self {
        Self {
            image_protocol: ImageProtocol::default(),
            error_auto_dismiss_ms: default_error_auto_dismiss_ms(),
            error_slide_start_ms: default_error_slide_start_ms(),
            error_fly_out_ms: default_error_fly_out_ms(),
            max_error_events: default_max_error_events(),
        }
    }
}

// CurseForge has no keyless/anonymous tier - every request needs a key.
// users can set their own in Global Settings (see `api_key` above); when
// unset, we fall back to the bundled default below so browsing works out
// of the box for casual/test use without registering an app key at
// console.curseforge.com first.
// the bundled key is a *shared* key, not a per-user secret: it ships in the
// repo and the compiled binary, so anyone can read it, and CurseForge can
// revoke or rate-limit it at any time since it isn't tied to any one user's
// account. treat it as a convenience default, not a guarantee — set your
// own key in Global Settings if it stops working or you hit rate limits.
const DEFAULT_CURSEFORGE_API_KEY: &str = "$2a$10$UG3pEQS/455W6eyE1ww5qu8ORkZ1/ejAqqkTVAbAM9tKlTtR0ZN12";

#[derive(Debug, Deserialize, Serialize, Default, Clone)]
pub struct CurseForge {
    #[serde(default)]
    pub api_key: Option<String>,
}

impl CurseForge {
    /// the key actually used for requests: the user's own key if they've set
    /// one, otherwise the bundled default. never `None` in practice — use
    /// `is_using_default_key` to tell the two cases apart for display.
    pub fn effective_api_key(&self) -> Option<&str> {
        self.api_key
            .as_deref()
            .filter(|s| !s.is_empty())
            .or(Some(DEFAULT_CURSEFORGE_API_KEY))
    }

    /// true when no user-supplied key is set and we're falling back to the
    /// bundled default — lets the settings UI say so instead of silently
    /// pretending nothing is configured.
    pub fn is_using_default_key(&self) -> bool {
        self.api_key.as_deref().filter(|s| !s.is_empty()).is_none()
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct Config {
    #[serde(default)]
    pub general: General,
    #[serde(default)]
    pub paths: Paths,
    #[serde(default)]
    pub defaults: Defaults,
    #[serde(default)]
    pub ui: Ui,
    #[serde(default)]
    pub curseforge: CurseForge,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn effective_java_path_none_when_absent() {
        let paths = Paths {
            java_path: None,
            ..Paths::default()
        };
        assert!(paths.effective_java_path().is_none());
    }

    #[test]
    fn effective_java_path_none_when_empty() {
        let paths = Paths {
            java_path: Some(String::new()),
            ..Paths::default()
        };
        assert!(paths.effective_java_path().is_none());
    }

    #[test]
    fn effective_java_path_some_when_set() {
        let paths = Paths {
            java_path: Some("/usr/bin/java".to_owned()),
            ..Paths::default()
        };
        assert_eq!(paths.effective_java_path(), Some("/usr/bin/java"));
    }

    #[test]
    fn resolve_path_absolute() {
        assert_eq!(resolve_path("/opt/alloy"), PathBuf::from("/opt/alloy"));
    }

    #[test]
    fn resolve_path_tilde_prefix() {
        let resolved = resolve_path("~/games/alloy");
        assert!(!resolved.to_string_lossy().starts_with('~'));
        assert!(resolved.to_string_lossy().ends_with("games/alloy"));
    }

    #[test]
    fn resolve_path_bare_tilde() {
        let resolved = resolve_path("~");
        assert!(!resolved.to_string_lossy().starts_with('~'));
    }

    #[test]
    fn curseforge_api_key_falls_back_to_default_when_absent() {
        let cf = CurseForge::default();
        assert_eq!(
            cf.effective_api_key(),
            Some(DEFAULT_CURSEFORGE_API_KEY)
        );
        assert!(cf.is_using_default_key());
    }

    #[test]
    fn curseforge_api_key_falls_back_to_default_when_empty() {
        let cf = CurseForge {
            api_key: Some(String::new()),
        };
        assert_eq!(
            cf.effective_api_key(),
            Some(DEFAULT_CURSEFORGE_API_KEY)
        );
        assert!(cf.is_using_default_key());
    }

    #[test]
    fn curseforge_api_key_uses_users_key_when_set() {
        let cf = CurseForge {
            api_key: Some("abc123".to_string()),
        };
        assert_eq!(cf.effective_api_key(), Some("abc123"));
        assert!(!cf.is_using_default_key());
    }

    #[test]
    fn config_deserializes_from_empty_toml() {
        let config: Config = toml::from_str("").unwrap();
        assert_eq!(config.defaults.memory_max, "2G");
    }

    #[test]
    fn config_deserializes_partial_toml() {
        let toml_str = r#"
[general]
debug = true

[defaults]
memory_max = "8G"
"#;
        let config: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(config.defaults.memory_max, "8G");
        assert_eq!(config.defaults.memory_min, "512M");
    }
}
