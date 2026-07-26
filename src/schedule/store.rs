//! Persistence for the job schedule and the watch daemon's heartbeat state.
//! All writes go through `write_atomic` so a reader (the TUI, or a
//! concurrently-running daemon) never observes a partially-written file.

use super::*;
use crate::config::ccsm_dir;
use anyhow::{Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// Snapshot of the running watch daemon's state, persisted so the TUI can
/// show daemon health without needing an IPC channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WatchState {
    /// OS process id of the running daemon.
    #[serde(default)]
    pub pid: u32,
    /// Epoch ms when the daemon started.
    #[serde(default)]
    pub started_at_ms: i64,
    /// Epoch ms of the daemon's most recent heartbeat write.
    #[serde(default)]
    pub heartbeat_ms: i64,
    /// Most recently observed usage percentage, if any.
    #[serde(default)]
    pub last_usage_pct: Option<f64>,
    /// Epoch ms when `last_usage_pct` was sampled.
    #[serde(default)]
    pub last_usage_at_ms: Option<i64>,
    /// Epoch ms when the active usage window resets, if known.
    #[serde(default)]
    pub reset_at_ms: Option<i64>,
    /// Most recent error encountered while fetching usage, if any.
    #[serde(default)]
    pub usage_error: Option<String>,
}

/// A lightweight fingerprint of a file's contents, used to detect changes
/// without re-parsing. Two files with equal mtime and length but different
/// content still produce different stamps because of the hash component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stamp {
    /// File modification time in epoch milliseconds.
    pub mtime_ms: i64,
    /// File length in bytes.
    pub len: u64,
    /// First 16 hex characters of the file's SHA-256 digest.
    pub sha_prefix: String,
}

/// Path to the persisted job schedule (`<ccsm_dir>/schedule.json`).
pub fn schedule_path() -> Option<PathBuf> {
    Some(ccsm_dir()?.join("schedule.json"))
}

/// Path to the persisted watch daemon state (`<ccsm_dir>/watch_state.json`).
pub fn watch_state_path() -> Option<PathBuf> {
    Some(ccsm_dir()?.join("watch_state.json"))
}

/// Load the schedule from disk, returning an empty schedule if the file does
/// not exist or cannot be parsed. Use `load_or_quarantine` when a corrupt
/// file should not be silently discarded.
pub fn load() -> Schedule {
    schedule_path()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Load the schedule from disk. If the file exists but fails to parse, it is
/// renamed to `schedule.json.corrupt-<epoch_ms>` (never deleted, since the
/// job list represents user intent that a silent default would lose) and an
/// empty schedule is returned alongside a human-readable warning.
pub fn load_or_quarantine() -> (Schedule, Option<String>) {
    let Some(path) = schedule_path() else {
        return (Schedule::default(), None);
    };
    let Ok(contents) = std::fs::read_to_string(&path) else {
        return (Schedule::default(), None);
    };
    match serde_json::from_str::<Schedule>(&contents) {
        Ok(schedule) => (schedule, None),
        Err(e) => {
            let now_ms = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_millis())
                .unwrap_or(0);
            let quarantine_path = path.with_file_name(format!("schedule.json.corrupt-{now_ms}"));
            let warning = match std::fs::rename(&path, &quarantine_path) {
                Ok(()) => format!(
                    "Schedule file was corrupt ({e}); moved to {}",
                    quarantine_path.display()
                ),
                Err(rename_err) => format!(
                    "Schedule file was corrupt ({e}) and could not be quarantined: {rename_err}"
                ),
            };
            (Schedule::default(), Some(warning))
        }
    }
}

/// Serialize and atomically write the schedule to disk.
pub fn save(schedule: &Schedule) -> Result<()> {
    let path = schedule_path().context("Could not determine ccsm config directory")?;
    let json = serde_json::to_string_pretty(schedule)?;
    write_atomic(&path, json.as_bytes())
}

/// Load the watch daemon's persisted state, or `None` if it has never run or
/// the file is missing/corrupt.
pub fn load_watch_state() -> Option<WatchState> {
    let path = watch_state_path()?;
    let contents = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&contents).ok()
}

/// Serialize and atomically write the watch daemon's state to disk.
pub fn save_watch_state(state: &WatchState) -> Result<()> {
    let path = watch_state_path().context("Could not determine ccsm config directory")?;
    let json = serde_json::to_string_pretty(state)?;
    write_atomic(&path, json.as_bytes())
}

/// Compute a fingerprint of the given file's contents, or `None` if it
/// cannot be read.
pub fn stamp(path: &Path) -> Option<Stamp> {
    let metadata = std::fs::metadata(path).ok()?;
    let mtime_ms = metadata
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis() as i64;
    let len = metadata.len();
    let bytes = std::fs::read(path).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    let digest = hasher.finalize();
    let sha_prefix = digest.iter().take(8).map(|b| format!("{b:02x}")).collect();
    Some(Stamp {
        mtime_ms,
        len,
        sha_prefix,
    })
}

/// Atomically write `bytes` to `path`. Writes to a temp file in the same
/// directory as `path` (NOT the system temp dir, which may be a different
/// filesystem and would degrade `persist` into a non-atomic copy), flushes
/// it to disk, then renames it into place.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let parent = path
        .parent()
        .context("Target path has no parent directory")?;
    std::fs::create_dir_all(parent)?;
    let mut tmp = NamedTempFile::new_in(parent)?;
    use std::io::Write;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}
