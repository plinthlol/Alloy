// SPDX-FileCopyrightText: 2026 Constantin Bauer
// SPDX-License-Identifier: GPL-3.0-only

// core instance types: loader, version, memory settings — persisted to
// instance.json.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModLoader {
    Vanilla,
    Fabric,
    Forge,
    NeoForge,
    Quilt,
}

impl fmt::Display for ModLoader {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModLoader::Vanilla => write!(f, "Vanilla"),
            ModLoader::Fabric => write!(f, "Fabric"),
            ModLoader::Forge => write!(f, "Forge"),
            ModLoader::NeoForge => write!(f, "NeoForge"),
            ModLoader::Quilt => write!(f, "Quilt"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    pub game_version: String,
    pub loader: ModLoader,
    pub loader_version: Option<String>,
    pub created: DateTime<Utc>,
    #[serde(default)]
    pub last_played: Option<DateTime<Utc>>,
    #[serde(default)]
    pub java_path: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_memory")]
    pub memory_max: Option<String>,
    #[serde(default, deserialize_with = "deserialize_optional_memory")]
    pub memory_min: Option<String>,
    #[serde(default)]
    pub jvm_args: Vec<String>,
    #[serde(default)]
    pub resolution: Option<(u32, u32)>,
    // linux-only: use the system's libglfw.so instead of the bundled
    // natives (e.g. wayland builds). ignored elsewhere.
    #[serde(default)]
    pub use_system_glfw: bool,
}

pub fn normalize_memory_value(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }

    let (number_part, suffix) = match trimmed.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => (
            &trimmed[..trimmed.len() - c.len_utf8()],
            Some(c.to_ascii_uppercase()),
        ),
        Some(_) => (trimmed, None),
        None => return None,
    };

    if number_part.is_empty() {
        return None;
    }

    // whole numbers: exact same behavior as before ("8" -> "8G", "4096" -> "4096M")
    if number_part.chars().all(|c| c.is_ascii_digit()) {
        let value = number_part.parse::<u64>().ok()?;
        if value == 0 {
            return None;
        }
        return match suffix {
            Some(unit @ ('K' | 'M' | 'G')) => Some(format!("{value}{unit}")),
            Some(_) => None,
            None => memory_number_to_string(value),
        };
    }

    // fractional values need an explicit unit (bare "2.5" is ambiguous).
    // -Xms/-Xmx take whole numbers, so scale down a unit (2.5G -> 2560M)
    // instead of rejecting outright.
    let unit = suffix?;
    if !matches!(unit, 'K' | 'M' | 'G') {
        return None;
    }
    let value: f64 = number_part.parse().ok()?;
    if !value.is_finite() || value <= 0.0 {
        return None;
    }

    match unit {
        'G' => scale_fraction_down(value, 'M'),
        'M' => scale_fraction_down(value, 'K'),
        'K' => None, // already the smallest unit we support; can't subdivide further
        _ => unreachable!(),
    }
}

fn scale_fraction_down(value: f64, target_unit: char) -> Option<String> {
    let scaled = (value * 1024.0).round() as u64;
    if scaled == 0 {
        return None;
    }
    Some(format!("{scaled}{target_unit}"))
}

fn deserialize_optional_memory<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Option::<Value>::deserialize(deserializer)?;
    match value {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(raw)) => Ok(normalize_memory_value(&raw)),
        Some(Value::Number(number)) => {
            let number = if let Some(value) = number.as_u64() {
                value
            } else if let Some(value) = number.as_i64() {
                let Ok(value) = u64::try_from(value) else {
                    return Ok(None);
                };
                value
            } else {
                let Some(value) = number.as_f64() else {
                    return Ok(None);
                };
                if !value.is_finite() || value.fract() != 0.0 || value < 0.0 {
                    return Ok(None);
                }
                value as u64
            };
            Ok(memory_number_to_string(number))
        }
        Some(_) => Ok(None),
    }
}

fn memory_number_to_string(value: u64) -> Option<String> {
    if value == 0 {
        return None;
    }
    if value < 128 {
        Some(format!("{value}G"))
    } else {
        Some(format!("{value}M"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_config_roundtrips_through_json() {
        let config = InstanceConfig {
            name: "test".to_string(),
            game_version: "1.20.1".to_string(),
            loader: ModLoader::Fabric,
            loader_version: Some("0.15.0".to_string()),
            created: Utc::now(),
            last_played: None,
            java_path: None,
            memory_max: Some("4G".to_string()),
            memory_min: Some("512M".to_string()),
            jvm_args: vec![],
            resolution: Some((1920, 1080)),
            use_system_glfw: false,
        };
        let json = serde_json::to_string_pretty(&config).expect("serialize");
        let parsed: InstanceConfig = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(parsed.name, config.name);
        assert_eq!(parsed.game_version, config.game_version);
        assert_eq!(parsed.loader, config.loader);
        assert_eq!(parsed.resolution, config.resolution);
    }

    #[test]
    fn instance_config_accepts_numeric_memory() {
        let json = r#"
        {
          "name": "test",
          "game_version": "1.7.10",
          "loader": "forge",
          "loader_version": "10.13.4.1614",
          "created": "2026-04-20T18:04:25.567993893Z",
          "memory_max": 8,
          "memory_min": 512
        }
        "#;
        let parsed: InstanceConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.memory_max.as_deref(), Some("8G"));
        assert_eq!(parsed.memory_min.as_deref(), Some("512M"));
    }

    #[test]
    fn normalize_memory_value_handles_bare_numbers() {
        assert_eq!(normalize_memory_value("8").as_deref(), Some("8G"));
        assert_eq!(normalize_memory_value("4096").as_deref(), Some("4096M"));
        assert_eq!(normalize_memory_value("8G").as_deref(), Some("8G"));
        assert_eq!(normalize_memory_value("2048m").as_deref(), Some("2048M"));
        assert_eq!(normalize_memory_value(""), None);
    }

    #[test]
    fn instance_config_ignores_invalid_memory_values() {
        let json = r#"
        {
          "name": "test",
          "game_version": "1.7.10",
          "loader": "forge",
          "loader_version": "10.13.4.1614",
          "created": "2026-04-20T18:04:25.567993893Z",
          "memory_max": ["8G"],
          "memory_min": "8GB"
        }
        "#;
        let parsed: InstanceConfig = serde_json::from_str(json).expect("deserialize");
        assert_eq!(parsed.memory_max, None);
        assert_eq!(parsed.memory_min, None);
    }

    #[test]
    fn normalize_memory_value_rejects_invalid_values() {
        assert_eq!(normalize_memory_value("0"), None);
        assert_eq!(normalize_memory_value("-1"), None);
        assert_eq!(normalize_memory_value("8GB"), None);
        assert_eq!(normalize_memory_value("banana"), None);
        // bare decimals are ambiguous without a unit
        assert_eq!(normalize_memory_value("1.5"), None);
        // K is already the smallest unit we support, so it can't be subdivided
        assert_eq!(normalize_memory_value("1.5K"), None);
    }

    #[test]
    fn normalize_memory_value_handles_fractions() {
        assert_eq!(normalize_memory_value("2.5G").as_deref(), Some("2560M"));
        assert_eq!(normalize_memory_value("2.7G").as_deref(), Some("2765M"));
        assert_eq!(normalize_memory_value("1.5G").as_deref(), Some("1536M"));
        assert_eq!(normalize_memory_value("0.5G").as_deref(), Some("512M"));
        assert_eq!(normalize_memory_value("1.5M").as_deref(), Some("1536K"));
        assert_eq!(normalize_memory_value("2.5g").as_deref(), Some("2560M"));
    }
}
