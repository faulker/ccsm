use anyhow::{Context, Result};
use chrono::{Local, TimeZone};
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
    pub has_data: bool,
    pub name: Option<String>,
    pub slug: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    pub cwd: Option<String>,
    pub git_branch: Option<String>,
    pub session_id: Option<String>,
    pub session_name: Option<String>,
    pub all_session_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct PreviewMessage {
    pub role: String,
    pub text: String,
}

#[derive(Deserialize)]
struct SlugLine {
    slug: Option<String>,
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
    cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    git_branch: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
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
            session.has_data = true;
            session.slug = read_slug(&path);
        }
    }
    result.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));

    Ok(result)
}

fn project_to_dir_name(project: &str) -> String {
    project.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
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

/// Read at most the first 10 lines of a session JSONL and return the first non-empty `slug` found.
fn read_slug(path: &Path) -> Option<String> {
    let file = File::open(path).ok()?;
    let reader = BufReader::new(file);
    for (i, line) in reader.lines().enumerate() {
        if i >= 10 {
            break;
        }
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        if !line.contains("\"slug\"") {
            continue;
        }
        if let Ok(entry) = serde_json::from_str::<SlugLine>(&line) {
            if let Some(slug) = entry.slug {
                if !slug.is_empty() {
                    return Some(slug);
                }
            }
        }
    }
    None
}

fn format_session_boundary_date(timestamp_ms: i64) -> String {
    match Local.timestamp_millis_opt(timestamp_ms) {
        chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M").to_string(),
        _ => "unknown".to_string(),
    }
}

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

pub fn load_preview(project: &str, session_id: &str) -> (SessionMeta, Vec<PreviewMessage>) {
    let (meta, messages) = load_session_messages(project, session_id);
    // Keep last 20 turns
    let start = messages.len().saturating_sub(20);
    (meta, messages[start..].to_vec())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_project_to_dir_name() {
        assert_eq!(
            project_to_dir_name("/Users/sane/Dev/foo"),
            "-Users-sane-Dev-foo"
        );
        assert_eq!(
            project_to_dir_name("/Users/sane/My Drive/Dev/foo"),
            "-Users-sane-My-Drive-Dev-foo"
        );
        assert_eq!(
            project_to_dir_name("/Users/sane/.claude"),
            "-Users-sane--claude"
        );
        assert_eq!(
            project_to_dir_name("/Users/sane/Dev/reki_base"),
            "-Users-sane-Dev-reki-base"
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
    fn test_strip_xml_tags_basic() {
        assert_eq!(strip_xml_tags("<command-name>foo</command-name>"), "foo");
        assert_eq!(strip_xml_tags("hello <tag>world</tag> end"), "hello world end");
    }

    #[test]
    fn test_strip_xml_tags_preserves_non_tags() {
        assert_eq!(strip_xml_tags("a < b and c > d"), "a < b and c > d");
        assert_eq!(strip_xml_tags("no tags here"), "no tags here");
        assert_eq!(strip_xml_tags("<123>not a tag</123>"), "<123>not a tag</123>");
    }

    #[test]
    fn test_strip_xml_tags_nested() {
        assert_eq!(
            strip_xml_tags("<outer>hello <inner>world</inner></outer>"),
            "hello world"
        );
    }

    #[test]
    fn test_strip_xml_tags_self_closing_not_matched() {
        // Our parser only matches <tag> and </tag>, not <tag/>
        assert_eq!(strip_xml_tags("before <br/> after"), "before <br/> after");
    }

    #[test]
    fn test_strip_xml_tags_with_hyphens_underscores() {
        assert_eq!(strip_xml_tags("<my-tag>content</my-tag>"), "content");
        assert_eq!(strip_xml_tags("<my_tag>content</my_tag>"), "content");
    }

    #[test]
    fn test_strip_xml_tags_empty_input() {
        assert_eq!(strip_xml_tags(""), "");
    }

    #[test]
    fn test_load_preview_missing_file() {
        let (meta, msgs) = load_preview("/nonexistent/path", "fake-id");
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].role, "system");
        assert!(meta.cwd.is_none());
        assert!(meta.git_branch.is_none());
    }

    #[test]
    fn test_read_slug_finds_slug() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("ccsm_test_slug");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("test_slug.jsonl");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"system","slug":"happy-flying-penguin","content":"init"}}"#).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#).unwrap();

        let slug = read_slug(&path);
        assert_eq!(slug, Some("happy-flying-penguin".to_string()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_slug_missing_returns_none() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("ccsm_test_slug2");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("no_slug.jsonl");

        let mut f = std::fs::File::create(&path).unwrap();
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#).unwrap();

        let slug = read_slug(&path);
        assert_eq!(slug, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_read_slug_only_checks_first_10_lines() {
        use std::io::Write;
        let dir = std::env::temp_dir().join("ccsm_test_slug3");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("late_slug.jsonl");

        let mut f = std::fs::File::create(&path).unwrap();
        // 10 lines without slug
        for _ in 0..10 {
            writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"x"}}}}"#).unwrap();
        }
        // slug on line 11 (index 10) — should not be found
        writeln!(f, r#"{{"type":"system","slug":"late-slug","content":"init"}}"#).unwrap();

        let slug = read_slug(&path);
        assert_eq!(slug, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    fn make_session_info(id: &str, first_ts: i64, last_ts: i64, slug: Option<&str>) -> SessionInfo {
        SessionInfo {
            session_id: id.to_string(),
            project: "/test/project".to_string(),
            project_name: "project".to_string(),
            first_timestamp: first_ts,
            last_timestamp: last_ts,
            entry_count: 2,
            has_data: false,
            name: None,
            slug: slug.map(|s| s.to_string()),
        }
    }

    #[test]
    fn test_load_chain_preview_orders_by_first_timestamp() {
        // Sessions without actual files will return "No session data available" messages
        let s1 = make_session_info("session-aaa", 1000, 2000, Some("test-slug"));
        let s2 = make_session_info("session-bbb", 500, 1500, Some("test-slug"));
        let s3 = make_session_info("session-ccc", 2000, 3000, Some("test-slug"));

        let sessions = vec![&s1, &s2, &s3];
        let (meta, _msgs) = load_chain_preview(&sessions);

        // all_session_ids should be sorted by first_timestamp: s2 (500), s1 (1000), s3 (2000)
        assert_eq!(
            meta.all_session_ids,
            vec!["session-bbb", "session-aaa", "session-ccc"]
        );
        // session_id should be the most recent (s3)
        assert_eq!(meta.session_id.as_deref(), Some("session-ccc"));
    }
}
