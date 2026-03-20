use anyhow::{Context, Result};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use super::io::session_file_path;
use super::types::*;

/// Strip XML-style tags like `<command-name>` and `</command-name>` from text,
/// keeping inner content. Only matches tags with lowercase letters, digits, hyphens, underscores.
pub fn strip_xml_tags(input: &str) -> String {
    let mut result = String::with_capacity(input.len());
    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if chars[i] == '<' {
            // Try to match a tag
            let start = i;
            i += 1;
            // Optional closing slash
            if i < len && chars[i] == '/' {
                i += 1;
            }
            // Must start with lowercase letter
            if i < len && chars[i].is_ascii_lowercase() {
                i += 1;
                // Continue with [a-z0-9_-]
                while i < len
                    && (chars[i].is_ascii_lowercase()
                        || chars[i].is_ascii_digit()
                        || chars[i] == '-'
                        || chars[i] == '_')
                {
                    i += 1;
                }
                // Must end with >
                if i < len && chars[i] == '>' {
                    i += 1;
                    // Successfully matched a tag, skip it
                    continue;
                }
            }
            // Not a valid tag, emit everything from start
            for &c in &chars[start..i] {
                result.push(c);
            }
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Load all sessions from `~/.claude/history.jsonl`, optionally restricting to entries
/// whose project path starts with `filter_path`. Returns sessions sorted by most recent activity.
pub fn load_sessions(filter_path: Option<&str>) -> Result<Vec<SessionInfo>> {
    let history_path = dirs::home_dir()
        .context("No home directory")?
        .join(".claude/history.jsonl");

    if !history_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&history_path).context("Failed to open history.jsonl")?;
    let reader = BufReader::new(file);

    let mut sessions: HashMap<(String, String), SessionInfo> = HashMap::new();

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let entry: HistoryEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let session_id = match entry.session_id {
            Some(id) => id,
            None => continue,
        };
        let project = entry.project.unwrap_or_default();

        if let Some(fp) = filter_path {
            if !project.starts_with(fp) {
                continue;
            }
        }
        let timestamp = entry.timestamp.unwrap_or(0);

        let project_name = Path::new(&project)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| project.clone());

        let key = (session_id.clone(), project.clone());
        sessions
            .entry(key)
            .and_modify(|s| {
                if timestamp < s.first_timestamp {
                    s.first_timestamp = timestamp;
                }
                if timestamp > s.last_timestamp {
                    s.last_timestamp = timestamp;
                }
                s.entry_count += 1;
            })
            .or_insert(SessionInfo {
                session_id,
                project: project.clone(),
                project_name,
                first_timestamp: timestamp,
                last_timestamp: timestamp,
                entry_count: 1,
                has_data: false,
                name: None,
                slug: None,
            });
    }

    let mut result: Vec<SessionInfo> = sessions.into_values().collect();
    for session in &mut result {
        if let Some(path) = session_file_path(&session.project, &session.session_id) {
            let (slug, exit_only) = read_session_meta(&path);
            if !exit_only {
                session.has_data = true;
                session.slug = slug;
            }
        }
    }
    result.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));

    Ok(result)
}

/// Read at most the first 20 lines of a session JSONL and return `(slug, exit_only)`.
///
/// `exit_only` is true when the session contains no assistant messages and the only
/// user message is the `/exit` command — these sessions should be treated as empty.
pub(crate) fn read_session_meta(path: &Path) -> (Option<String>, bool) {
    let file = match File::open(path).ok() {
        Some(f) => f,
        None => return (None, false),
    };
    let reader = BufReader::new(file);
    let mut slug: Option<String> = None;
    let mut has_assistant = false;
    let mut has_non_exit_user = false;
    let mut has_any_user = false;

    for (i, line) in reader.lines().enumerate() {
        if i >= 20 {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };

        // Slug detection (existing logic)
        if slug.is_none() && line.contains("\"slug\"") {
            if let Ok(entry) = serde_json::from_str::<SlugLine>(&line) {
                if let Some(s) = entry.slug {
                    if !s.is_empty() {
                        slug = Some(s);
                    }
                }
            }
        }

        // Exit-only detection: parse the line to check message types
        if let Ok(meta) = serde_json::from_str::<MetaLine>(&line) {
            match meta.entry_type.as_deref() {
                Some("assistant") => {
                    has_assistant = true;
                }
                Some("user") => {
                    if meta.is_meta.unwrap_or(false) {
                        continue;
                    }
                    let content = meta.message.and_then(|m| m.content);
                    if let Some(ref text) = content {
                        if text.contains("local-command-stdout") || text.contains("local-command-caveat") {
                            continue;
                        }
                    }
                    has_any_user = true;
                    if let Some(ref text) = content {
                        let stripped = strip_xml_tags(text);
                        // Check if every non-empty line is just /exit or exit
                        let is_exit = stripped.lines().all(|l| {
                            let t = l.trim();
                            t.is_empty() || t == "/exit" || t == "exit"
                        });
                        if !is_exit {
                            has_non_exit_user = true;
                        }
                    } else {
                        has_non_exit_user = true;
                    }
                }
                _ => {}
            }
        }
    }

    let exit_only = has_any_user && !has_assistant && !has_non_exit_user;
    (slug, exit_only)
}
