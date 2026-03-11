use anyhow::{Context, Result};
use serde::Deserialize;
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub session_id: String,
    pub project: String,
    pub project_name: String,
    pub first_timestamp: i64,
    pub last_timestamp: i64,
    pub entry_count: usize,
}

#[derive(Debug, Clone)]
pub struct PreviewMessage {
    pub role: String,
    pub text: String,
}

#[derive(Deserialize)]
struct HistoryEntry {
    #[serde(default)]
    #[allow(dead_code)]
    display: Option<String>,
    timestamp: Option<i64>,
    project: Option<String>,
    #[serde(rename = "sessionId")]
    session_id: Option<String>,
}

#[derive(Deserialize)]
struct SessionEntry {
    #[serde(rename = "type")]
    entry_type: Option<String>,
    #[serde(rename = "isMeta")]
    is_meta: Option<bool>,
    message: Option<MessageData>,
}

#[derive(Deserialize)]
struct MessageData {
    role: Option<String>,
    content: Option<ContentValue>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ContentValue {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Deserialize)]
struct ContentBlock {
    #[serde(rename = "type")]
    block_type: Option<String>,
    text: Option<String>,
}

pub fn load_sessions() -> Result<Vec<SessionInfo>> {
    let history_path = dirs::home_dir()
        .context("No home directory")?
        .join(".claude/history.jsonl");

    if !history_path.exists() {
        return Ok(Vec::new());
    }

    let file = File::open(&history_path).context("Failed to open history.jsonl")?;
    let reader = BufReader::new(file);

    let mut sessions: HashMap<String, SessionInfo> = HashMap::new();

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
        let timestamp = entry.timestamp.unwrap_or(0);

        let project_name = Path::new(&project)
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| project.clone());

        sessions
            .entry(session_id.clone())
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
            });
    }

    let mut result: Vec<SessionInfo> = sessions.into_values().collect();
    result.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));
    Ok(result)
}

fn project_to_dir_name(project: &str) -> String {
    project.replace('/', "-")
}

fn session_file_path(project: &str, session_id: &str) -> Option<PathBuf> {
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

pub fn load_preview(project: &str, session_id: &str) -> Vec<PreviewMessage> {
    let path = match session_file_path(project, session_id) {
        Some(p) => p,
        None => return vec![PreviewMessage {
            role: "system".to_string(),
            text: "No session data available".to_string(),
        }],
    };

    let file = match File::open(&path) {
        Ok(f) => f,
        Err(_) => return vec![PreviewMessage {
            role: "system".to_string(),
            text: "Failed to read session file".to_string(),
        }],
    };

    let reader = BufReader::new(file);
    let mut messages = Vec::new();

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

        if entry.is_meta.unwrap_or(false) {
            continue;
        }

        let entry_type = entry.entry_type.as_deref().unwrap_or("");
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

        if text.trim().is_empty() {
            continue;
        }

        messages.push(PreviewMessage { role, text });
    }

    // Keep last 20 turns
    let start = messages.len().saturating_sub(20);
    messages[start..].to_vec()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_to_dir_name() {
        assert_eq!(
            project_to_dir_name("/Users/sane/Dev/foo"),
            "-Users-sane-Dev-foo"
        );
    }

    #[test]
    fn test_parse_history_entry() {
        let line = r#"{"display":"test","timestamp":1000,"project":"/Users/sane/Dev/foo","sessionId":"abc-123"}"#;
        let entry: HistoryEntry = serde_json::from_str(line).unwrap();
        assert_eq!(entry.session_id.unwrap(), "abc-123");
        assert_eq!(entry.project.unwrap(), "/Users/sane/Dev/foo");
        assert_eq!(entry.timestamp.unwrap(), 1000);
    }

    #[test]
    fn test_parse_user_message() {
        let line = r#"{"type":"user","message":{"role":"user","content":"hello world"}}"#;
        let entry: SessionEntry = serde_json::from_str(line).unwrap();
        assert_eq!(entry.entry_type.as_deref(), Some("user"));
        if let Some(ContentValue::Text(t)) = entry.message.unwrap().content {
            assert_eq!(t, "hello world");
        } else {
            panic!("Expected text content");
        }
    }

    #[test]
    fn test_parse_assistant_message_blocks() {
        let line = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"hi there"},{"type":"thinking","thinking":"hmm"}]}}"#;
        let entry: SessionEntry = serde_json::from_str(line).unwrap();
        if let Some(ContentValue::Blocks(blocks)) = entry.message.unwrap().content {
            let texts: Vec<_> = blocks
                .iter()
                .filter(|b| b.block_type.as_deref() == Some("text"))
                .filter_map(|b| b.text.clone())
                .collect();
            assert_eq!(texts, vec!["hi there"]);
        } else {
            panic!("Expected blocks content");
        }
    }

    #[test]
    fn test_load_preview_missing_file() {
        let msgs = load_preview("/nonexistent/path", "fake-id");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "system");
    }
}
