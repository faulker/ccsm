use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Controls how session entries are labelled in the list.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum DisplayMode {
    /// Show only the project directory's base name.
    #[default]
    Name,
    /// Show the last two path components (e.g. `Dev/ccsm`).
    ShortDir,
    /// Show the full absolute path.
    FullDir,
}

impl DisplayMode {
    /// Returns the short human-readable label shown in the UI title bar.
    pub fn label(self) -> &'static str {
        match self {
            Self::Name => "[name]",
            Self::ShortDir => "[short dir]",
            Self::FullDir => "[full dir]",
        }
    }
}

/// Persisted application configuration stored in `~/.config/ccsm/config.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Whether to use the tree view (true) or flat list view (false).
    pub tree_view: bool,
    /// How session labels are displayed in the list.
    pub display_mode: DisplayMode,
    /// When true, sessions with no JSONL data are hidden from the list.
    #[serde(default = "default_true")]
    pub hide_empty: bool,
    /// When true, sessions sharing a slug are grouped into a single chain entry.
    #[serde(default = "default_true")]
    pub group_chains: bool,
    /// Unix timestamp (seconds) of the last update check, or `None` if never checked.
    #[serde(default)]
    pub last_update_check: Option<i64>,
    /// When true, only projects with active live sessions are shown.
    #[serde(default)]
    pub live_filter: bool,
    /// Set of project paths that are pinned to the top of the list.
    #[serde(default)]
    pub favorites: HashSet<String>,
    /// Custom path to the `claude` binary (None = look up "claude" on PATH).
    #[serde(default)]
    pub claude_path: Option<String>,
    /// Custom path to the `tmux` binary (None = look up "tmux" on PATH).
    #[serde(default)]
    pub tmux_path: Option<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            tree_view: true,
            display_mode: DisplayMode::Name,
            hide_empty: true,
            group_chains: true,
            last_update_check: None,
            live_filter: false,
            favorites: HashSet::new(),
            claude_path: None,
            tmux_path: None,
        }
    }
}

/// Serde default helper that returns `true`.
fn default_true() -> bool {
    true
}

/// Returns the platform-specific path to `ccsm/config.json` inside the user's config directory.
fn config_path() -> Option<PathBuf> {
    let base = dirs::config_dir().or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("ccsm").join("config.json"))
}

impl Config {
    /// Load the config from disk, returning `Config::default()` if the file does not exist or cannot be parsed.
    pub fn load() -> Self {
        config_path()
            .and_then(|path| std::fs::read_to_string(path).ok())
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    }

    /// Returns true if no update check has been performed in the last 24 hours.
    #[allow(dead_code)]
    pub fn should_check_for_update(&self) -> bool {
        match self.last_update_check {
            None => true,
            Some(ts) => {
                let now = chrono::Utc::now().timestamp();
                now - ts > 24 * 60 * 60
            }
        }
    }

    /// Records the current time as the last update check timestamp and saves the config.
    pub fn mark_update_checked(&mut self) -> anyhow::Result<()> {
        self.last_update_check = Some(chrono::Utc::now().timestamp());
        self.save()
    }

    /// Returns the configured claude binary path, or `"claude"` if unset.
    pub fn claude_bin(&self) -> &str {
        self.claude_path.as_deref().unwrap_or("claude")
    }

    /// Returns the configured tmux binary path, or `"tmux"` if unset.
    pub fn tmux_bin(&self) -> &str {
        self.tmux_path.as_deref().unwrap_or("tmux")
    }

    /// Returns true if the given binary name/path is findable on the system.
    pub fn is_bin_available(bin: &str) -> bool {
        if Path::new(bin).is_absolute() {
            Path::new(bin).exists()
        } else {
            // Run the binary directly with --version to avoid shell injection.
            std::process::Command::new(bin)
                .arg("--version")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .is_ok()
        }
    }

    /// Serialize the config to pretty-printed JSON and write it to the config file path.
    pub fn save(&self) -> anyhow::Result<()> {
        let path = config_path().ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
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
        assert!(config.group_chains);
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
            group_chains: false,
            last_update_check: None,
            live_filter: false,
            favorites: HashSet::new(),
            claude_path: None,
            tmux_path: None,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.tree_view, false);
        assert_eq!(loaded.display_mode, DisplayMode::FullDir);
        assert_eq!(loaded.hide_empty, true);
        assert_eq!(loaded.group_chains, false);
    }

    #[test]
    fn test_config_load_returns_valid_config() {
        // Config::load() returns defaults when no file exists,
        // or the user's saved config if present — either way it should be valid
        let config = Config::load();
        // Verify fields are accessible and display_mode is a known variant
        let _ = config.tree_view;
        assert!(matches!(
            config.display_mode,
            DisplayMode::Name | DisplayMode::ShortDir | DisplayMode::FullDir
        ));
    }

    #[test]
    fn test_config_serialization_to_file() {
        let dir = std::env::temp_dir().join("ccsm_test_config");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.json");

        let config = Config {
            tree_view: false,
            display_mode: DisplayMode::ShortDir,
            hide_empty: false,
            group_chains: true,
            last_update_check: None,
            live_filter: false,
            favorites: HashSet::new(),
            claude_path: None,
            tmux_path: None,
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(json.as_bytes()).unwrap();

        let loaded: Config =
            serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(loaded.tree_view, false);
        assert_eq!(loaded.display_mode, DisplayMode::ShortDir);
        assert_eq!(loaded.hide_empty, false);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_config_path_is_valid() {
        let path = config_path().expect("config_path should return Some on supported platforms");
        assert!(path.ends_with("ccsm/config.json"));
    }

    #[test]
    fn test_config_deserialize_missing_required_field_fails() {
        let json = r#"{"tree_view": false}"#;
        let result: Result<Config, _> = serde_json::from_str(json);
        assert!(result.is_err());
    }

    #[test]
    fn test_config_backward_compat_without_hide_empty() {
        let json = r#"{"tree_view": true, "display_mode": "name"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.hide_empty, true);
        assert_eq!(config.group_chains, true);
        assert_eq!(config.last_update_check, None);
    }

    #[test]
    fn test_config_backward_compat_without_group_chains() {
        let json = r#"{"tree_view": true, "display_mode": "name", "hide_empty": true}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.group_chains, true);
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
            group_chains: true,
            last_update_check: None,
            live_filter: false,
            favorites: HashSet::new(),
            claude_path: None,
            tmux_path: None,
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

    #[test]
    fn test_config_backward_compat_without_favorites() {
        let json = r#"{"tree_view": true, "display_mode": "name"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert!(config.favorites.is_empty());
    }

    #[test]
    fn test_config_favorites_roundtrip() {
        let mut favorites = HashSet::new();
        favorites.insert("/Users/sane/Dev/ccsm".to_string());
        favorites.insert("/Users/sane/Dev/other".to_string());
        let config = Config {
            favorites: favorites.clone(),
            ..Config::default()
        };
        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.favorites, favorites);
    }

    #[test]
    fn test_config_default_has_empty_favorites() {
        let config = Config::default();
        assert!(config.favorites.is_empty());
    }

    #[test]
    fn test_config_backward_compat_without_paths() {
        let json = r#"{"tree_view": true, "display_mode": "name"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.claude_path, None);
        assert_eq!(config.tmux_path, None);
    }

    #[test]
    fn test_claude_bin_default() {
        let config = Config::default();
        assert_eq!(config.claude_bin(), "claude");
    }

    #[test]
    fn test_tmux_bin_default() {
        let config = Config::default();
        assert_eq!(config.tmux_bin(), "tmux");
    }

    #[test]
    fn test_claude_bin_custom() {
        let mut config = Config::default();
        config.claude_path = Some("/usr/local/bin/claude".to_string());
        assert_eq!(config.claude_bin(), "/usr/local/bin/claude");
    }

    #[test]
    fn test_tmux_bin_custom() {
        let mut config = Config::default();
        config.tmux_path = Some("/opt/bin/tmux".to_string());
        assert_eq!(config.tmux_bin(), "/opt/bin/tmux");
    }

    #[test]
    fn test_config_paths_roundtrip() {
        let mut config = Config::default();
        config.claude_path = Some("/usr/local/bin/claude".to_string());
        config.tmux_path = Some("/opt/bin/tmux".to_string());
        let json = serde_json::to_string_pretty(&config).unwrap();
        let loaded: Config = serde_json::from_str(&json).unwrap();
        assert_eq!(loaded.claude_path, Some("/usr/local/bin/claude".to_string()));
        assert_eq!(loaded.tmux_path, Some("/opt/bin/tmux".to_string()));
    }

    #[test]
    fn test_is_bin_available_absolute_nonexistent() {
        assert!(!Config::is_bin_available("/nonexistent/path/to/binary"));
    }

    #[cfg(unix)]
    #[test]
    fn test_is_bin_available_bare_name_sh() {
        // `sh` should be available on Unix systems
        assert!(Config::is_bin_available("sh"));
    }
}
