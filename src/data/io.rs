use chrono::{Local, TimeZone};
use std::path::PathBuf;

/// Convert an absolute project path to the directory name Claude uses on disk
/// by replacing every non-alphanumeric character with a hyphen.
pub(crate) fn project_to_dir_name(project: &str) -> String {
    project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// Returns the path to a session's JSONL file under `~/.claude/projects/`, or `None` if it does not exist.
pub(crate) fn session_file_path(project: &str, session_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    let dir_name = project_to_dir_name(project);
    let path = home
        .join(".claude/projects")
        .join(dir_name)
        .join(format!("{}.jsonl", session_id));
    if path.exists() {
        Some(path)
    } else {
        None
    }
}

/// Format a millisecond timestamp as a short local date string (e.g. `"Jan 02 15:04"`).
pub(crate) fn format_session_boundary_date(timestamp_ms: i64) -> String {
    match Local.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M").to_string(),
        _ => "unknown".to_string(),
    }
}
