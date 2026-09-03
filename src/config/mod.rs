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
    save_config_to(&get_config_path().join("config.toml"), config)
}

fn save_config_to(path: &std::path::Path, config: &Config) -> Result<(), ConfigLoadError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let existing = fs::read_to_string(path).unwrap_or_default();
    let out = match existing.parse::<toml_edit::DocumentMut>() {
        Ok(mut doc) => {
            apply_config_to_doc(&mut doc, config);
            doc.to_string()
        }
        Err(e) => {
            tracing::warn!(
                "Config file at {} failed to parse ({}); rewriting without preserving comments",
                path.display(),
                e
            );
            toml::to_string_pretty(config)?
        }
    };
    fs::write(path, out)?;
    Ok(())
}

fn ensure_table(doc: &mut toml_edit::DocumentMut, name: &str) {
    if !doc.contains_key(name) {
        doc.insert(name, toml_edit::Item::Table(toml_edit::Table::new()));
    }
}

fn set_str(doc: &mut toml_edit::DocumentMut, table: &str, key: &str, value: &str) {
    ensure_table(doc, table);
    doc[table][key] = toml_edit::value(value);
}

fn set_opt_str(doc: &mut toml_edit::DocumentMut, table: &str, key: &str, value: &Option<String>) {
    ensure_table(doc, table);
    match value {
        Some(v) if !v.is_empty() => doc[table][key] = toml_edit::value(v.as_str()),
        _ => {
            doc[table][key] = toml_edit::Item::None;
        }
    }
}

fn set_int(doc: &mut toml_edit::DocumentMut, table: &str, key: &str, value: u64) {
    ensure_table(doc, table);
    doc[table][key] = toml_edit::value(value as i64);
}

fn apply_config_to_doc(doc: &mut toml_edit::DocumentMut, config: &Config) {
    set_str(doc, "paths", "instances_dir", &config.paths.instances_dir);
    set_str(doc, "paths", "meta_dir", &config.paths.meta_dir);
    set_opt_str(doc, "paths", "java_path", &config.paths.java_path);

    set_str(doc, "defaults", "memory_min", &config.defaults.memory_min);
    set_str(doc, "defaults", "memory_max", &config.defaults.memory_max);

    let protocol = match config.ui.image_protocol {
        settings::ImageProtocol::Halfblocks => "halfblocks",
        settings::ImageProtocol::Quadrants => "quadrants",
        settings::ImageProtocol::Sixel => "sixel",
        settings::ImageProtocol::Kitty => "kitty",
        settings::ImageProtocol::Iterm2 => "iterm2",
    };
    set_str(doc, "ui", "image_protocol", protocol);
    set_int(doc, "ui", "error_auto_dismiss_ms", config.ui.error_auto_dismiss_ms);
    set_int(doc, "ui", "error_slide_start_ms", config.ui.error_slide_start_ms);
    set_int(doc, "ui", "error_fly_out_ms", config.ui.error_fly_out_ms);
    set_int(doc, "ui", "max_error_events", config.ui.max_error_events as u64);

    set_opt_str(doc, "curseforge", "api_key", &config.curseforge.api_key);
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

    #[test]
    fn save_config_preserves_comments_and_formatting() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(
            &path,
            "# my precious comments\n[paths]\n# about java\njava_path = \"/old/java\"\n",
        )
        .unwrap();

        let mut config = load_config(&path).unwrap();
        config.paths.java_path = Some("/new/java".to_string());
        save_config_to(&path, &config).unwrap();

        let written = std::fs::read_to_string(&path).unwrap();
        assert!(written.contains("# my precious comments"));
        assert!(written.contains("# about java"));
        assert!(written.contains("java_path = \"/new/java\""));
        assert!(!written.contains("/old/java"));
    }

    #[test]
    fn save_config_falls_back_to_rewrite_on_unparseable_file() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("config.toml");
        std::fs::write(&path, "not [ valid toml").unwrap();

        let config: Config = toml::from_str("").unwrap();
        save_config_to(&path, &config).unwrap();

        let reloaded = load_config(&path).unwrap();
        assert_eq!(reloaded.defaults.memory_max, "2G");
    }
}
