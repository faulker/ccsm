//! Command queue used by the TUI (or any other client) to ask the watch
//! daemon to mutate the schedule. Commands are dropped as individual JSON
//! files into `commands/` so the daemon can pick them up on its next poll
//! without any IPC beyond the filesystem.

use super::*;
use crate::config::ccsm_dir;
use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use tempfile::NamedTempFile;

/// Process-local counter breaking ties when multiple commands are enqueued
/// within the same millisecond.
static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Partial update to a job's editable fields. Fields left as `None` are left
/// unchanged. For fields that are themselves optional on `Job` (like
/// `model`), `Some(None)` clears the value and `Some(Some(v))` sets it.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct JobPatch {
    /// New display name, if changing.
    #[serde(default)]
    pub name: Option<String>,
    /// New working directory, if changing.
    #[serde(default)]
    pub cwd: Option<String>,
    /// New initial prompt, if changing.
    #[serde(default)]
    pub prompt: Option<String>,
    /// New continue-prompt override, if changing.
    #[serde(default)]
    pub continue_prompt: Option<Option<String>>,
    /// New model override, if changing.
    #[serde(default)]
    pub model: Option<Option<String>>,
    /// New pause mode, if changing.
    #[serde(default)]
    pub pause_mode: Option<PauseMode>,
    /// New dangerous-mode flag, if changing.
    #[serde(default)]
    pub dangerous: Option<bool>,
    /// New auto-resume flag, if changing.
    #[serde(default)]
    pub auto_resume: Option<bool>,
}

/// A single request from a client (the TUI) to the watch daemon.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Command {
    /// Create a brand-new job.
    CreateJob {
        /// The job to add to the schedule.
        job: Job,
    },
    /// Apply a partial update to an existing job.
    UpdateJob {
        /// Id of the job to update.
        id: String,
        /// Fields to change.
        patch: JobPatch,
    },
    /// Remove a job from the schedule entirely.
    DeleteJob {
        /// Id of the job to delete.
        id: String,
    },
    /// Request that a running job be paused.
    PauseJob {
        /// Id of the job to pause.
        id: String,
    },
    /// Request that a paused job be resumed.
    ResumeJob {
        /// Id of the job to resume.
        id: String,
    },
    /// Request that a job be stopped and no longer auto-managed.
    StopJob {
        /// Id of the job to stop.
        id: String,
    },
    /// Mark a job as finished successfully.
    MarkDone {
        /// Id of the job to mark done.
        id: String,
    },
    /// Adopt an already-running tmux session as a managed job.
    AdoptLive {
        /// Id of the job to attach the live session to.
        id: String,
        /// Name of the existing tmux session being adopted.
        tmux_name: String,
    },
    /// Ask the watch daemon to shut itself down.
    StopWatcher,
    /// Ask the watch daemon to refresh its usage sample immediately.
    RefreshUsage,
}

/// Directory holding pending command files (`<ccsm_dir>/commands`).
pub fn commands_dir() -> Option<PathBuf> {
    Some(ccsm_dir()?.join("commands"))
}

/// Directory holding command files that failed to parse (`<commands_dir>/bad`).
fn bad_commands_dir(dir: &Path) -> PathBuf {
    dir.join("bad")
}

/// Enqueue a command for the watch daemon to process. Writes atomically via
/// a temp file plus rename, so a concurrent `drain` never observes a partial
/// file. Filenames are `{unix_millis:013}-{counter:04}-{uuid_prefix8}.json`
/// so a lexicographic sort of the directory is also a chronological sort.
pub fn enqueue(cmd: &Command) -> Result<()> {
    let dir = commands_dir().context("Could not determine ccsm config directory")?;
    std::fs::create_dir_all(&dir)?;
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let counter = COUNTER.fetch_add(1, Ordering::Relaxed) % 10_000;
    let uuid_prefix = uuid::Uuid::new_v4().simple().to_string();
    let filename = format!("{now_ms:013}-{counter:04}-{}.json", &uuid_prefix[..8]);
    let path = dir.join(filename);
    let json = serde_json::to_vec_pretty(cmd)?;

    let mut tmp = NamedTempFile::new_in(&dir)?;
    use std::io::Write;
    tmp.write_all(&json)?;
    tmp.as_file().sync_all()?;
    tmp.persist(&path)?;
    Ok(())
}

/// Read all pending command files **without removing them**, returning them
/// paired with their paths in creation order (oldest first), alongside
/// human-readable warnings for any files that could not be parsed.
/// Unparseable files are moved to `commands/bad/` rather than deleted, since
/// a bug here should not silently drop user intent.
///
/// The caller must apply the commands, persist the resulting schedule, and
/// only then call [`ack`] with the paths. Deleting on read instead would lose
/// queued user actions if the daemon crashed between reading and saving; this
/// way the worst case is a replayed command, and every command is keyed by a
/// caller-generated id so replays are idempotent.
pub fn read_pending() -> (Vec<(PathBuf, Command)>, Vec<String>) {
    let mut commands = Vec::new();
    let mut warnings = Vec::new();

    let Some(dir) = commands_dir() else {
        return (commands, warnings);
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return (commands, warnings);
    };

    let mut files: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();

    for path in files {
        let Ok(contents) = std::fs::read_to_string(&path) else {
            continue;
        };
        match serde_json::from_str::<Command>(&contents) {
            Ok(cmd) => commands.push((path, cmd)),
            Err(e) => {
                let bad_dir = bad_commands_dir(&dir);
                if std::fs::create_dir_all(&bad_dir).is_ok() {
                    if let Some(name) = path.file_name() {
                        let _ = std::fs::rename(&path, bad_dir.join(name));
                    }
                }
                warnings.push(format!(
                    "Could not parse command file {}: {e}",
                    path.display()
                ));
            }
        }
    }

    (commands, warnings)
}

/// Delete command files that have been applied and durably persisted. Call
/// this only after the resulting schedule has been saved successfully.
pub fn ack(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

/// Read and remove all pending command files in one step. Convenience wrapper
/// for callers with no durability requirement; the watch daemon uses
/// [`read_pending`] plus [`ack`] instead so a crash cannot lose commands.
#[allow(dead_code)]
pub fn drain() -> (Vec<Command>, Vec<String>) {
    let (pending, warnings) = read_pending();
    let paths: Vec<PathBuf> = pending.iter().map(|(p, _)| p.clone()).collect();
    let commands = pending.into_iter().map(|(_, c)| c).collect();
    ack(&paths);
    (commands, warnings)
}

/// Count pending commands, ignoring the `bad/` quarantine subdirectory.
pub fn pending_count() -> usize {
    let Some(dir) = commands_dir() else {
        return 0;
    };
    std::fs::read_dir(&dir)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.path().is_file()
                        && e.path().extension().and_then(|x| x.to_str()) == Some("json")
                })
                .count()
        })
        .unwrap_or(0)
}
