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

/// How a managed session is paused when account usage crosses the threshold.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum PauseMode {
    /// Send Escape to interrupt claude, leaving the tmux session alive at its prompt.
    #[default]
    Soft,
    /// Kill the tmux session; relaunch later with `claude --resume <id>`.
    Hard,
}

impl PauseMode {
    /// Returns the short human-readable label shown in the UI.
    #[allow(dead_code)]
    pub fn label(self) -> &'static str {
        match self {
            Self::Soft => "Soft (Escape)",
            Self::Hard => "Hard (kill + resume)",
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
    /// Path to Claude Desktop's `plan-usage-history.json`. `None` means use the
    /// standard location for the platform.
    #[serde(default)]
    pub usage_history_path: Option<String>,
    /// Pause managed sessions when 5-hour usage reaches this percentage.
    #[serde(default = "default_pause_percent")]
    pub usage_pause_percent: f64,
    /// Resume paused sessions once usage falls to or below this percentage.
    #[serde(default = "default_resume_percent")]
    pub usage_resume_percent: f64,
    /// Seconds between usage polls while a job is active.
    #[serde(default = "default_usage_poll_seconds")]
    pub usage_poll_seconds: u64,
    /// A usage sample older than this many seconds is treated as stale. A
    /// stale sample can still trigger a pause, but never a resume.
    #[serde(default = "default_usage_max_age_seconds")]
    pub usage_max_age_seconds: u64,
    /// Which usage source to read: `"auto"`, `"local"`, or `"api"`.
    #[serde(default = "default_usage_source")]
    pub usage_source: String,
    /// Default pause strategy for new jobs.
    #[serde(default)]
    pub pause_mode: PauseMode,
    /// Also pause on the 7-day usage window, not just the 5-hour one.
    #[serde(default = "default_true")]
    pub watch_seven_day: bool,
    /// Start the watcher daemon automatically when a job is created.
    #[serde(default = "default_true")]
    pub watch_autostart: bool,
    /// Skip automated key sends while a tmux client is attached to the session.
    #[serde(default = "default_true")]
    pub defer_while_attached: bool,
    /// Text pasted into a paused session to make it continue.
    #[serde(default = "default_continue_prompt")]
    pub continue_prompt: String,
    /// Give up relaunching a job after this many consecutive failures.
    #[serde(default = "default_max_restart_attempts")]
    pub max_restart_attempts: u32,
    /// Mark a running job done once its pane has looked idle this long without
    /// the agent ever emitting the completion marker. `0` disables the
    /// fallback, leaving the marker as the only way a job finishes by itself.
    #[serde(default = "default_idle_complete_seconds")]
    pub idle_complete_seconds: u64,
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
            usage_history_path: None,
            usage_pause_percent: default_pause_percent(),
            usage_resume_percent: default_resume_percent(),
            usage_poll_seconds: default_usage_poll_seconds(),
            usage_max_age_seconds: default_usage_max_age_seconds(),
            usage_source: default_usage_source(),
            pause_mode: PauseMode::default(),
            watch_seven_day: default_true(),
            watch_autostart: default_true(),
            defer_while_attached: default_true(),
            continue_prompt: default_continue_prompt(),
            max_restart_attempts: default_max_restart_attempts(),
            idle_complete_seconds: default_idle_complete_seconds(),
        }
    }
}

/// Serde default helper that returns `true`.
fn default_true() -> bool {
    true
}

/// Serde default helper for `usage_pause_percent`.
fn default_pause_percent() -> f64 {
    95.0
}

/// Serde default helper for `usage_resume_percent`.
fn default_resume_percent() -> f64 {
    50.0
}

/// Serde default helper for `usage_poll_seconds`.
fn default_usage_poll_seconds() -> u64 {
    60
}

/// Serde default helper for `usage_max_age_seconds`: 5 minutes. A paused job
/// can only resume on a fresh sample, so this doubles as the longest a job
/// waits on stale usage data before a newer reading can release it. Raising it
/// tolerates a slower-updating local history file; lowering it makes the `auto`
/// source fall through to the API sooner.
fn default_usage_max_age_seconds() -> u64 {
    300
}

/// Serde default helper for `usage_source`.
fn default_usage_source() -> String {
    "auto".to_string()
}

/// Serde default helper for `continue_prompt`.
fn default_continue_prompt() -> String {
    "Continue where you left off.".to_string()
}

/// Serde default helper for `max_restart_attempts`.
fn default_max_restart_attempts() -> u32 {
    5
}

/// Serde default helper for `idle_complete_seconds`: 15 minutes. Long enough
/// that a slow tool call or a pattern `live::detect_activity` does not
/// recognise cannot be mistaken for a finished job.
fn default_idle_complete_seconds() -> u64 {
    900
}

/// Root directory for all ccsm state files. `CCSM_CONFIG_DIR` overrides the
/// default location, primarily for tests.
pub fn ccsm_dir() -> Option<PathBuf> {
    if let Ok(override_dir) = std::env::var("CCSM_CONFIG_DIR") {
        return Some(PathBuf::from(override_dir));
    }
    let base = dirs::config_dir().or_else(|| dirs::home_dir().map(|h| h.join(".config")))?;
    Some(base.join("ccsm"))
}

/// Returns the platform-specific path to `ccsm/config.json` inside the user's config directory.
fn config_path() -> Option<PathBuf> {
    Some(ccsm_dir()?.join("config.json"))
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

    /// Returns the configured usage-history override, or `None` to use the
    /// platform's standard Claude Desktop location.
    pub fn usage_history_override(&self) -> Option<&str> {
        self.usage_history_path.as_deref().filter(|p| !p.is_empty())
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
        let path =
            config_path().ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(self)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}

/// Serializes tests that mutate the `CCSM_CONFIG_DIR` environment variable,
/// since env vars are process-global and tests run concurrently.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static M: Mutex<()> = Mutex::new(());
    M.lock().unwrap_or_else(|p| p.into_inner())
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
            ..Config::default()
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
            ..Config::default()
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
        // `config_path` honours CCSM_CONFIG_DIR, which other tests set. Hold
        // the shared lock and clear the override so this asserts the real
        // default rather than whichever temp dir happened to be active.
        let _guard = test_lock();
        std::env::remove_var("CCSM_CONFIG_DIR");
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
            ..Config::default()
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
        assert_eq!(
            loaded.claude_path,
            Some("/usr/local/bin/claude".to_string())
        );
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

    #[test]
    fn test_config_backward_compat_without_scheduler_fields() {
        let json = r#"{"tree_view": true, "display_mode": "name"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.usage_history_path, None);
        assert_eq!(config.usage_pause_percent, 95.0);
        assert_eq!(config.usage_resume_percent, 50.0);
        assert_eq!(config.usage_poll_seconds, 60);
        assert_eq!(config.usage_max_age_seconds, 300);
        assert_eq!(config.usage_source, "auto");
        assert_eq!(config.pause_mode, PauseMode::Soft);
        assert!(config.watch_seven_day);
        assert!(config.watch_autostart);
        assert!(config.defer_while_attached);
        assert_eq!(config.continue_prompt, "Continue where you left off.");
        assert_eq!(config.max_restart_attempts, 5);
    }

    #[test]
    fn test_pause_mode_serde_roundtrip() {
        let soft_json = serde_json::to_string(&PauseMode::Soft).unwrap();
        assert_eq!(soft_json, "\"soft\"");
        let hard_json = serde_json::to_string(&PauseMode::Hard).unwrap();
        assert_eq!(hard_json, "\"hard\"");

        let soft: PauseMode = serde_json::from_str("\"soft\"").unwrap();
        assert_eq!(soft, PauseMode::Soft);
        let hard: PauseMode = serde_json::from_str("\"hard\"").unwrap();
        assert_eq!(hard, PauseMode::Hard);
    }

    #[test]
    fn test_pause_mode_labels() {
        assert_eq!(PauseMode::Soft.label(), "Soft (Escape)");
        assert_eq!(PauseMode::Hard.label(), "Hard (kill + resume)");
    }

    #[test]
    fn test_usage_history_override_default() {
        assert_eq!(Config::default().usage_history_override(), None);
    }

    #[test]
    fn test_usage_history_override_custom() {
        let mut config = Config::default();
        config.usage_history_path = Some("/opt/plan-usage-history.json".to_string());
        assert_eq!(
            config.usage_history_override(),
            Some("/opt/plan-usage-history.json")
        );
    }

    #[test]
    fn test_usage_history_override_treats_an_empty_string_as_unset() {
        let mut config = Config::default();
        config.usage_history_path = Some(String::new());
        assert_eq!(config.usage_history_override(), None);
    }

    #[test]
    fn test_config_drops_the_legacy_usage_binary_path() {
        // Configs written before usage was built in carry a claude-usage binary
        // path. It has no meaning now, so it must be ignored, not adopted as a
        // history-file path.
        let json = r#"{"tree_view":true,"display_mode":"name","usage_path":"/usr/local/bin/claude-usage"}"#;
        let config: Config = serde_json::from_str(json).unwrap();
        assert_eq!(config.usage_history_path, None);
    }

    #[test]
    fn test_ccsm_dir_honors_override() {
        let _guard = test_lock();
        std::env::set_var("CCSM_CONFIG_DIR", "/tmp/ccsm_dir_override_test");
        let dir = ccsm_dir().expect("ccsm_dir should return Some when override is set");
        assert_eq!(dir, PathBuf::from("/tmp/ccsm_dir_override_test"));
        std::env::remove_var("CCSM_CONFIG_DIR");
    }
}
