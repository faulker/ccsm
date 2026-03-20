use serde::Deserialize;

/// Summary record for one Claude session, built from `~/.claude/history.jsonl`.
#[derive(Debug, Clone)]
pub struct SessionInfo {
    /// Unique session identifier (UUID-like string from Claude).
    pub session_id: String,
    /// Absolute path of the project directory.
    pub project: String,
    /// Base name of the project directory, used for display.
    pub project_name: String,
    /// Earliest entry timestamp seen for this session (milliseconds).
    pub first_timestamp: i64,
    /// Most recent entry timestamp seen for this session (milliseconds).
    pub last_timestamp: i64,
    /// Number of history entries belonging to this session.
    pub entry_count: usize,
    /// True if a corresponding JSONL session file exists on disk.
    pub has_data: bool,
    /// Optional custom title set by the user via the rename feature.
    pub name: Option<String>,
    /// Optional slug read from the session JSONL, used to group chained sessions.
    pub slug: Option<String>,
}

/// Metadata extracted from a session JSONL file used to populate the details bar.
#[derive(Debug, Clone, Default)]
pub struct SessionMeta {
    /// Working directory recorded in the session file.
    pub cwd: Option<String>,
    /// Git branch recorded in the session file.
    pub git_branch: Option<String>,
    /// Session ID (usually the most recent in a chain).
    pub session_id: Option<String>,
    /// Custom title set via `custom-title` entries, if any.
    pub session_name: Option<String>,
    /// All session IDs that make up the chain, sorted oldest to newest.
    pub all_session_ids: Vec<String>,
}

/// A single conversation turn shown in the preview pane.
#[derive(Debug, Clone)]
pub struct PreviewMessage {
    /// Role of the speaker: `"user"`, `"assistant"`, or `"system"`.
    pub role: String,
    /// Plain text content of the message (XML tags stripped).
    pub text: String,
}

/// Minimal deserialization target used to extract the `slug` field from a JSONL line.
#[derive(Deserialize)]
pub(crate) struct SlugLine {
    pub slug: Option<String>,
}

/// Minimal deserialization target for exit-only detection.
#[derive(Deserialize)]
pub(crate) struct MetaLine {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    #[serde(rename = "isMeta")]
    pub is_meta: Option<bool>,
    pub message: Option<MetaMessage>,
}

/// Message payload for exit-only detection.
#[derive(Deserialize)]
pub(crate) struct MetaMessage {
    pub content: Option<String>,
}

/// One line from `~/.claude/history.jsonl`, capturing session identity and timing.
#[derive(Deserialize)]
pub(crate) struct HistoryEntry {
    #[serde(default)]
    #[allow(dead_code)]
    pub display: Option<String>,
    pub timestamp: Option<i64>,
    pub project: Option<String>,
    #[serde(rename = "sessionId")]
    pub session_id: Option<String>,
}

/// One line from a session JSONL file.
#[derive(Deserialize)]
pub(crate) struct SessionEntry {
    #[serde(rename = "type")]
    pub entry_type: Option<String>,
    #[serde(rename = "isMeta")]
    pub is_meta: Option<bool>,
    pub message: Option<MessageData>,
    pub cwd: Option<String>,
    #[serde(rename = "gitBranch")]
    pub git_branch: Option<String>,
    #[serde(rename = "customTitle")]
    pub custom_title: Option<String>,
}

/// The message object nested inside a `SessionEntry`.
#[derive(Deserialize)]
pub(crate) struct MessageData {
    pub role: Option<String>,
    pub content: Option<ContentValue>,
}

/// Message content is either a plain string or an array of typed content blocks.
#[derive(Deserialize)]
#[serde(untagged)]
pub(crate) enum ContentValue {
    /// Simple text-only content.
    Text(String),
    /// Structured content blocks (e.g. text, thinking, tool_use).
    Blocks(Vec<ContentBlock>),
}

/// A single block within a structured content array.
#[derive(Deserialize)]
pub(crate) struct ContentBlock {
    #[serde(rename = "type")]
    pub block_type: Option<String>,
    pub text: Option<String>,
}
