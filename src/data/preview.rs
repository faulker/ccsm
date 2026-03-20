use std::fs::File;
use std::io::{BufRead, BufReader};

use super::history::strip_xml_tags;
use super::io::{format_session_boundary_date, session_file_path};
use super::types::*;

/// Load all messages from a session JSONL without any turn cap.
/// Returns (meta, all_messages).
fn load_session_messages(project: &str, session_id: &str) -> (SessionMeta, Vec<PreviewMessage>) {
    let path = match session_file_path(project, session_id) {
        Some(p) => p,
        None => return (SessionMeta::default(), vec![PreviewMessage {
            role: "system".to_string(),
            text: "No session data available".to_string(),
        }]),
    };

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return (SessionMeta::default(), vec![PreviewMessage {
            role: "system".to_string(),
            text: "Failed to read session file".to_string(),
        }]),
    };

    let reader = BufReader::new(file);
    let mut messages = Vec::new();
    let mut meta = SessionMeta::default();
    meta.session_id = Some(session_id.to_string());

    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if line.trim().is_empty() {
            continue;
        }

        let entry: SessionEntry = match serde_json::from_str(&line) {
            Ok(e) => e,
            Err(_) => continue,
        };

        if let Some(cwd) = entry.cwd {
            if !cwd.is_empty() {
                meta.cwd = Some(cwd);
            }
        }
        if let Some(branch) = entry.git_branch {
            if !branch.is_empty() {
                meta.git_branch = Some(branch);
            }
        }

        let entry_type = entry.entry_type.as_deref().unwrap_or("");
        if entry_type == "custom-title" {
            if let Some(title) = entry.custom_title {
                meta.session_name = Some(title);
            }
            continue;
        }

        if entry.is_meta.unwrap_or(false) {
            continue;
        }

        if entry_type != "user" && entry_type != "assistant" {
            continue;
        }

        let message = match entry.message {
            Some(m) => m,
            None => continue,
        };

        let role = message.role.unwrap_or_else(|| entry_type.to_string());

        let text = match message.content {
            Some(ContentValue::Text(s)) => s,
            Some(ContentValue::Blocks(blocks)) => {
                let texts: Vec<String> = blocks
                    .iter()
                    .filter(|b| b.block_type.as_deref() == Some("text"))
                    .filter_map(|b| b.text.clone())
                    .collect();
                if texts.is_empty() {
                    continue;
                }
                texts.join("\n")
            }
            None => continue,
        };

        let text = strip_xml_tags(&text);

        if text.trim().is_empty() {
            continue;
        }

        messages.push(PreviewMessage { role, text });
    }

    (meta, messages)
}

/// Load messages from multiple chained sessions, combining them in chronological order
/// with session boundary markers between each session's messages.
pub fn load_chain_preview(sessions: &[&SessionInfo]) -> (SessionMeta, Vec<PreviewMessage>) {
    let mut sorted = sessions.to_vec();
    sorted.sort_by_key(|s| s.first_timestamp);

    let mut all_messages: Vec<PreviewMessage> = Vec::new();
    let mut combined_meta = SessionMeta::default();

    for session in &sorted {
        let (meta, messages) = load_session_messages(&session.project, &session.session_id);

        // Most recent session's cwd/branch wins (last wins)
        if let Some(cwd) = meta.cwd {
            combined_meta.cwd = Some(cwd);
        }
        if let Some(branch) = meta.git_branch {
            combined_meta.git_branch = Some(branch);
        }
        if let Some(name) = meta.session_name {
            combined_meta.session_name = Some(name);
        }

        // Insert boundary marker before each session (except the first)
        if !all_messages.is_empty() {
            let short_id: String = session.session_id.chars().take(8).collect();
            let date = format_session_boundary_date(session.first_timestamp);
            all_messages.push(PreviewMessage {
                role: "system".to_string(),
                text: format!("─── Session {} · {} ───", short_id, date),
            });
        }

        all_messages.extend(messages);
    }

    if let Some(last) = sorted.last() {
        combined_meta.session_id = Some(last.session_id.clone());
        combined_meta.all_session_ids = sorted.iter().map(|s| s.session_id.clone()).collect();
    }

    (combined_meta, all_messages)
}

/// Load the most recent 20 messages from a single session for display in the preview pane.
pub fn load_preview(project: &str, session_id: &str) -> (SessionMeta, Vec<PreviewMessage>) {
    let (meta, messages) = load_session_messages(project, session_id);
    // Keep last 20 turns
    let start = messages.len().saturating_sub(20);
    (meta, messages[start..].to_vec())
}
