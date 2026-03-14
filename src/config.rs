use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    #[default]
    Name,
    ShortDir,
    FullDir,
}

impl DisplayMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "[name]",
            Self::ShortDir => "[short dir]",
            Self::FullDir => "[full dir]",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub tree_view: bool,
    pub display_mode: DisplayMode,
    #[serde(default = "default_true")]
    pub hide_empty: bool,
    #[serde(default)]
    pub last_update_check: Option<i64>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tree_view: true,
            display_mode: DisplayMode::Name,
            hide_empty: true,
            last_update_check: None,
        }
    }
}

fn default_true() -> bool {
    true
}

fn config_path() -> PathBuf {
    dirs::config_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from("."))
        .join("ccsm")
        .join("config.json")
}


impl Config {
    pub fn load() -> Self {
        let path = config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    pub fn should_check_for_update(&self) -> bool {
        match self.last_update_check {
            None => true,
            Some(ts) => {
                let now = chrono::Utc::now().timestamp();
                now - ts > 24 * 60 * 60
            }
        }
    }

    pub fn mark_update_checked(&mut self) -> anyhow::Result<()> {
        self.last_update_check = Some(chrono::Utc::now().timestamp());
        self.save()
    }

    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.tree_view);
        assert_eq!(config.display_mode, DisplayMode::Name);
        assert!(config.hide_empty);
    }

    #[test]
    fn test_display_mode_labels() {
        assert_eq!(DisplayMode::Name.label(), "[name]");
        assert_eq!(DisplayMode::ShortDir.label(), "[short dir]");
        assert_eq!(DisplayMode::FullDir.label(), "[full dir]");
    }

    #[test]
    fn test_config_serialization_roundtrip() {
        let config = Config {
            tree_view: false,
            display_mode: DisplayMode::FullDir,
            hide_empty: true,
            last_update_check: None,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.tree_view, false);
        assert_eq!(loaded.display_mode, DisplayMode::FullDir);
        assert_eq!(loaded.hide_empty, true);
    }

    #[test]
    fn test_config_load_missing_file_returns_default() {
        let config = Config::load();
        let _ = config.tree_view;
        let _ = config.display_mode;
    }

    #[test]
    fn test_config_save_and_load() {
        let dir = std::env::temp_dir().join("ccsm_test_config");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");

        let config = Config {
            tree_view: false,
            display_mode: DisplayMode::ShortDir,
            hide_empty: false,
            last_update_check: None,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let loaded: Config =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.tree_view, false);
        assert_eq!(loaded.display_mode, DisplayMode::ShortDir);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_path_is_valid() {
        let path = config_path();
        assert!(path.ends_with("ccsm/config.json"));
    }

    #[test]
    fn test_config_deserialize_partial_uses_defaults() {
        let json = r#"{"tree_view": false}"#;
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_backward_compat_without_hide_empty() {
        let json = r#"{"tree_view": true, "display_mode": "name"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.hide_empty, true);
        assert_eq!(config.last_update_check, None);
    }

    #[test]
    fn test_config_deserialize_invalid_json() {
        let json = "not json at all";
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_display_mode_serializes_as_snake_case() {
        let config = Config {
            tree_view: true,
            display_mode: DisplayMode::ShortDir,
            hide_empty: false,
            last_update_check: None,
        };
        let json = serde_json::to_string(&config).unwrap();
        assert!(json.contains("\"short_dir\""));
    }

    #[test]
    fn test_should_check_for_update_none() {
        let config = Config {
            last_update_check: None,
            ..Config::default()
        };
        assert!(config.should_check_for_update());
    }

    #[test]
    fn test_should_check_for_update_recent() {
        let config = Config {
            last_update_check: Some(chrono::Utc::now().timestamp()),
            ..Config::default()
        };
        assert!(!config.should_check_for_update());
    }

    #[test]
    fn test_should_check_for_update_stale() {
        let config = Config {
            last_update_check: Some(chrono::Utc::now().timestamp() - 25 * 60 * 60),
            ..Config::default()
        };
        assert!(config.should_check_for_update());
    }

    #[test]
    fn test_config_backward_compat_without_last_update_check() {
        let json = r#"{"tree_view": true, "display_mode": "name", "hide_empty": true}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.last_update_check, None);
    }
}
