use anyhow::{Context, Result};
use std::fs::File;
use std::io::{BufRead, BufReader};

use super::io::session_file_path;
use super::types::SessionEntry;

/// Load the custom title from a session JSONL file (last `custom-title` entry wins).
pub fn load_custom_title(project: &str, session_id: &str) -> Option<String> {
    let path = session_file_path(project, session_id)?;
    let file = File::open(&path).ok()?;
    let reader = BufReader::new(file);
    let mut title = None;

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        // Skip JSON parsing for lines that can't be custom-title entries
        if !line.contains("custom-title") {
            continue;
        }
        let entry: SessionEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };
        if entry.entry_type.as_deref() == Some("custom-title") {
            if let Some(t) = entry.custom_title {
                title = Some(t);
            }
        }
    }

    title
}

/// Save a custom title by appending a `custom-title` entry to the session JSONL file.
pub fn save_custom_title(project: &str, session_id: &str, title: &str) -> Result<()> {
    use std::io::Write;

    let path = session_file_path(project, session_id)
        .context("Session file not found")?;

    let entry = serde_json::json!({
        "type": "custom-title",
        "customTitle": title,
        "sessionId": session_id,
    });

    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .context("Failed to open session file for appending")?;

    writeln!(file, "{}", entry)?;
    Ok(())
}
