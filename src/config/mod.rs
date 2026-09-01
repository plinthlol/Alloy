// loads config.toml from the platform config dir, creating defaults if
// missing. everything lands in SETTINGS so the rest of the app can grab it.
//
// plain toml::from_str rather than the `config` crate — we read exactly one
// hardcoded path and every field has a serde default, so the crate's extra
// parsers (json5/ron/ini/yaml) would be dead weight.

use std::fs;
use std::path::PathBuf;
use std::sync::LazyLock;
use thiserror::Error;

pub mod settings;
pub mod theme;

pub use settings::Config;

#[derive(Debug, Error)]
pub enum ConfigLoadError {
    #[error("failed to read config file: {0}")]
    Io(#[from] std::io::Error),
    #[error("failed to parse config: {0}")]
    Toml(#[from] toml::de::Error),
    #[error("failed to serialize config: {0}")]
    TomlSer(#[from] toml::ser::Error),
}

#[must_use]
pub fn get_config_path() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("alloy")
}

// seeds the config file from the bundled default on first run
fn ensure_config_exists() -> PathBuf {
    let config_path = get_config_path().join("config.toml");
    if !config_path.exists() {
        if let Some(parent) = config_path.parent()
            && let Err(e) = fs::create_dir_all(parent)
        {
            tracing::warn!(
                "Failed to create config directory {}: {}",
                parent.display(),
                e
            );
        }
        // first run: auto-select a java runtime instead of leaving java_path
        // blank; prefer the newest install.
        let mut default_content = include_str!("../../assets/config.toml").to_string();
        if let Some(java_path) = crate::net::best_installed_java() {
            tracing::info!("First run: auto-selected Java runtime at {}", java_path);
            default_content =
                default_content.replacen("java_path = \"\"", &format!("java_path = {java_path:?}"), 1);
        } else {
            tracing::debug!("First run: no installed Java runtime detected, leaving java_path blank");
        }

        match fs::write(&config_path, default_content) {
            Ok(()) => tracing::debug!("Wrote default config to {}", config_path.display()),
            Err(e) => tracing::warn!(
                "Failed to write default config to {}: {}",
                config_path.display(),
                e
            ),
        }
    } else {
        tracing::trace!("Using existing config at {}", config_path.display());
    }
    config_path
}

pub fn load_config(config_path: &std::path::Path) -> Result<Config, ConfigLoadError> {
    tracing::debug!("Loading config from {}", config_path.display());
    // a missing file isn't an error — every field has a serde default, so
    // an empty string just parses to the default config.
    let content = match fs::read_to_string(config_path) {
        Ok(content) => content,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(ConfigLoadError::Io(e)),
    };
    toml::from_str(&content).map_err(ConfigLoadError::Toml)
}

// writes a full Config back to config.toml (e.g. from the global settings
// screen). SETTINGS is read once at startup, so this only touches disk —
// the running process keeps its loaded values until restarted.
pub fn save_config(config: &Config) -> Result<(), ConfigLoadError> {
    let path = get_config_path().join("config.toml");
    let toml_str = toml::to_string_pretty(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, toml_str)?;
    Ok(())
}

pub static SETTINGS: LazyLock<Config> = LazyLock::new(|| {
    let path = ensure_config_exists();
    load_config(&path).unwrap_or_else(|e| {
        tracing::error!("Config load failed, using defaults: {}", e);
        Config {
            general: settings::General::default(),
            paths: settings::Paths::default(),
            defaults: settings::Defaults::default(),
            ui: settings::Ui::default(),
            curseforge: settings::CurseForge::default(),
        }
    })
});

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn load_config_from_valid_toml() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [defaults]
            memory_max = "4G"
            "#,
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.defaults.memory_max, "4G");
        assert_eq!(config.defaults.memory_min, "512M");
    }

    #[test]
    fn load_config_from_empty_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "").unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.defaults.memory_max, "2G");
    }

    #[test]
    fn load_config_missing_file_uses_defaults() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("nonexistent.toml");
        load_config(&path).unwrap();
    }

    #[test]
    fn load_config_partial_sections() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            r#"
            [paths]
            instances_dir = "/custom/path"
            "#,
        )
        .unwrap();
        let config = load_config(&path).unwrap();
        assert_eq!(config.paths.instances_dir, "/custom/path");
        assert!(config.paths.java_path.is_none());
    }
}
