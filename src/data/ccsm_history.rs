use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::PathBuf;

use super::types::PreviewMessage;

/// Cross-module mutex shared by tests that mutate the `CCSM_HISTORY_DIR` env
/// var. Putting it here (instead of inside each `tests` submodule) ensures
/// app-side tests and data-side tests serialize against each other.
#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    use std::sync::Mutex;
    static M: Mutex<()> = Mutex::new(());
    M.lock().unwrap_or_else(|p| p.into_inner())
}

/// Identifies which CCSM launch path created or first observed this session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CcsmOrigin {
    Resume,
    Direct,
    AttachLive,
    NewLive,
    NewLiveDangerous,
    NewDirect,
}

/// One snapshot record written to CCSM's own history file. The latest record
/// per `session_id` wins on read, so multiple appends for the same session are
/// fine and let us capture updates (new messages, custom title, longer chain).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CcsmSessionRecord {
    pub session_id: String,
    pub project: String,
    pub project_name: String,
    pub first_timestamp: i64,
    pub last_timestamp: i64,
    pub entry_count: usize,
    pub name: Option<String>,
    pub slug: Option<String>,
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub ccsm_launched_at: i64,
    pub ccsm_origin: CcsmOrigin,
    #[serde(default)]
    pub preview_messages: Vec<PreviewMessage>,
    pub preview_cached_at: i64,
}

/// Path to the CCSM-owned history file. `CCSM_HISTORY_DIR` overrides the
/// default location, primarily for tests.
pub fn ccsm_history_path() -> Option<PathBuf> {
    if let Ok(override_dir) = std::env::var("CCSM_HISTORY_DIR") {
        return Some(PathBuf::from(override_dir).join("sessions.jsonl"));
    }
    let base = dirs::data_dir().or_else(|| dirs::home_dir().map(|h| h.join(".local/share")))?;
    Some(base.join("ccsm").join("sessions.jsonl"))
}

/// Read every record from CCSM's history file and return the most recent record
/// per `session_id`. A missing file is not an error — callers see an empty map.
pub fn load_ccsm_records() -> HashMap<String, CcsmSessionRecord> {
    let mut latest: HashMap<String, CcsmSessionRecord> = HashMap::new();
    let path = match ccsm_history_path() {
        Some(p) => p,
        None => return latest,
    };
    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return latest,
    };
    let reader = BufReader::new(file);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }
        let record: CcsmSessionRecord = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(_) => continue,
        };
        // Last write wins per session_id; replace any earlier snapshot.
        latest.insert(record.session_id.clone(), record);
    }
    latest
}

/// Append a single record as a JSON line to CCSM's history file. Creates the
/// parent directory on first write.
pub fn append_ccsm_record(record: &CcsmSessionRecord) -> io::Result<()> {
    let path = ccsm_history_path()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no data dir"))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    let line = serde_json::to_string(record)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    writeln!(file, "{}", line)?;
    Ok(())
}
