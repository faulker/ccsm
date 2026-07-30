//! Read Cursor Agent CLI chat transcripts from `store.db`.
//!
//! Format notes live in `docs/CURSOR-CLI-STORE-FORMAT.md`. Logic ports the
//! verified prototype at `~/Dev/ccsm-cursor-fixtures/prototypes/rust_poc.rs`.

use rusqlite::{Connection, OpenFlags};
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use super::types::PreviewMessage;

/// Decode a protobuf base-128 varint, returning `(value, next_offset)`.
fn read_varint(buf: &[u8], mut i: usize) -> Option<(u64, usize)> {
    let mut val: u64 = 0;
    let mut shift = 0;
    while i < buf.len() {
        let b = buf[i];
        val |= ((b & 0x7F) as u64) << shift;
        i += 1;
        if b & 0x80 == 0 {
            return Some((val, i));
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
    None
}

/// Collect the ordered 32-byte child blob ids from repeated field 1 of an index blob.
fn blob_refs(buf: &[u8]) -> Vec<String> {
    let mut out = Vec::new();
    let mut i = 0usize;
    while i < buf.len() {
        let Some((key, ni)) = read_varint(buf, i) else {
            break;
        };
        i = ni;
        let fnum = key >> 3;
        let wtype = key & 7;
        match wtype {
            0 => {
                let Some((_, ni)) = read_varint(buf, i) else {
                    break;
                };
                i = ni;
            }
            2 => {
                let Some((len, ni)) = read_varint(buf, i) else {
                    break;
                };
                i = ni;
                let len = len as usize;
                if i + len > buf.len() {
                    break;
                }
                if fnum == 1 && len == 32 {
                    out.push(hex_encode(&buf[i..i + len]));
                }
                i += len;
            }
            5 => i += 4,
            1 => i += 8,
            _ => break,
        }
    }
    out
}

/// Hex-encode bytes as lowercase.
fn hex_encode(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Hex-decode a string into bytes, or `None` if malformed.
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// Strip Cursor's injected wrappers from a user (or assistant) text body.
///
/// Claude's `strip_xml_tags` keeps inner content of every tag, which would leave
/// the environment preamble and timestamps visible. Cursor needs: drop
/// `<user_info>` / `<timestamp>` entirely, keep only the `<user_query>` body when
/// present, then strip any remaining lowercase XML tags.
pub(crate) fn strip_cursor_wrappers(input: &str) -> String {
    let without_info = strip_tagged_sections(input, "user_info");
    let without_ts = strip_tagged_sections(&without_info, "timestamp");
    let query_body = extract_tagged_body(&without_ts, "user_query").unwrap_or(without_ts);
    super::history::strip_xml_tags(&query_body)
        .trim()
        .to_string()
}

/// Remove every `<tag>...</tag>` section (including the tags) from `input`.
fn strip_tagged_sections(input: &str, tag: &str) -> String {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let mut rest = input;
    let mut out = String::with_capacity(input.len());
    while let Some(start) = rest.find(&open) {
        out.push_str(&rest[..start]);
        let after_open = &rest[start + open.len()..];
        match after_open.find(&close) {
            Some(end) => rest = &after_open[end + close.len()..],
            None => {
                // Unclosed tag: drop the open tag and keep the rest.
                rest = after_open;
                break;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Return the inner text of the first `<tag>...</tag>`, if present.
fn extract_tagged_body(input: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = input.find(&open)? + open.len();
    let end_rel = input[start..].find(&close)?;
    Some(input[start..start + end_rel].to_string())
}

/// Flatten a Cursor message `content` value into preview text.
fn flatten_content(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(blocks) => {
            let mut text = String::new();
            for blk in blocks {
                match blk.get("type").and_then(|t| t.as_str()) {
                    Some("text") => {
                        if let Some(t) = blk.get("text").and_then(|t| t.as_str()) {
                            text.push_str(t);
                        }
                    }
                    // Skip: usually empty text plus a long opaque signature.
                    Some("reasoning") => {}
                    Some("tool-call") => {
                        let name = blk.get("toolName").cloned().unwrap_or(Value::Null);
                        text.push_str(&format!("[tool-call {name}]"));
                    }
                    Some("tool-result") => {
                        let name = blk.get("toolName").cloned().unwrap_or(Value::Null);
                        text.push_str(&format!("[tool-result {name}]"));
                    }
                    _ => {}
                }
            }
            text
        }
        _ => String::new(),
    }
}

/// Open `store.db` read-only, copying the WAL trio into a temp dir on failure.
fn open_store_readonly(store_path: &Path) -> Result<(Connection, Option<tempfile::TempDir>), String> {
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY;
    match Connection::open_with_flags(store_path, flags) {
        Ok(conn) => {
            // Probe a query: WAL shm creation can fail after open succeeds.
            match conn.query_row("select value from meta limit 1", [], |r| r.get::<_, String>(0)) {
                Ok(_) => Ok((conn, None)),
                Err(_) => open_store_via_wal_copy(store_path),
            }
        }
        Err(_) => open_store_via_wal_copy(store_path),
    }
}

/// Copy `store.db` plus `-wal`/`-shm` into a tempdir and open the copy read-only.
///
/// Reading `store.db` alone would silently drop turns that only exist in the WAL.
fn open_store_via_wal_copy(
    store_path: &Path,
) -> Result<(Connection, Option<tempfile::TempDir>), String> {
    let tmp = tempfile::tempdir().map_err(|e| format!("tempdir for WAL copy: {e}"))?;
    let dest = tmp.path().join("store.db");
    fs::copy(store_path, &dest).map_err(|e| format!("copy store.db: {e}"))?;
    for suffix in ["-wal", "-shm"] {
        let src = PathBuf::from(format!("{}{suffix}", store_path.display()));
        if src.exists() {
            let name = format!("store.db{suffix}");
            fs::copy(&src, tmp.path().join(name))
                .map_err(|e| format!("copy store.db{suffix}: {e}"))?;
        }
    }
    let conn = Connection::open_with_flags(&dest, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .map_err(|e| format!("open WAL copy: {e}"))?;
    Ok((conn, Some(tmp)))
}

/// Chat title and message stats pulled from an already-open `store.db`.
#[derive(Debug, Default)]
pub(crate) struct CursorStoreMeta {
    /// Custom title from the meta row, or `None` when untitled / default "New Agent".
    pub name: Option<String>,
    /// Count of root-ref messages whose role is `user` or `assistant`.
    pub entry_count: usize,
}

/// Read the `meta` row title and cheaply count user/assistant root refs.
///
/// Returns `None` when the database cannot be opened or has no meta row.
pub(crate) fn read_store_meta(store_path: &Path) -> Option<CursorStoreMeta> {
    // Unused chats are 0-byte files; never open them.
    let len = fs::metadata(store_path).ok()?.len();
    if len == 0 {
        return Some(CursorStoreMeta::default());
    }
    let (conn, _tmp) = open_store_readonly(store_path).ok()?;
    read_store_meta_from_conn(&conn).ok()
}

/// Parse meta + root refs from an open connection.
fn read_store_meta_from_conn(conn: &Connection) -> Result<CursorStoreMeta, String> {
    let raw: String = conn
        .query_row("select value from meta limit 1", [], |r| r.get(0))
        .map_err(|e| format!("meta row: {e}"))?;
    let bytes = hex_decode(&raw).ok_or_else(|| "meta value is not valid hex".to_string())?;
    let meta: Value =
        serde_json::from_slice(&bytes).map_err(|e| format!("meta JSON: {e}"))?;

    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|n| !n.is_empty() && *n != "New Agent")
        .map(|s| s.to_string());

    let latest_root = meta.get("latestRootBlobId").and_then(|v| v.as_str());

    let mut entry_count = 0usize;
    let mut saw_user = false;
    if let Some(root_id) = latest_root {
        if let Ok(root_data) = load_blob(conn, root_id) {
            let refs = blob_refs(&root_data);
            for r in refs {
                if let Ok(data) = load_blob(conn, &r) {
                    if data.first() != Some(&b'{') {
                        continue;
                    }
                    let Ok(msg) = serde_json::from_slice::<Value>(&data) else {
                        continue;
                    };
                    match msg.get("role").and_then(|v| v.as_str()) {
                        // System boilerplate and the injected env preamble are
                        // not real turns; a one-exchange chat should read "2 msg".
                        Some("system") => {}
                        Some("user") => {
                            let raw = flatten_content(msg.get("content").unwrap_or(&Value::Null));
                            if !saw_user {
                                saw_user = true;
                                if raw.contains("<user_info>") {
                                    continue;
                                }
                            }
                            entry_count += 1;
                        }
                        Some("assistant") => entry_count += 1,
                        _ => {}
                    }
                }
            }
        }
    }

    Ok(CursorStoreMeta { name, entry_count })
}

/// Load one blob by id from the open connection.
fn load_blob(conn: &Connection, id: &str) -> Result<Vec<u8>, String> {
    conn.query_row("select data from blobs where id = ?1", [id], |r| r.get(0))
        .map_err(|e| format!("blob {id}: {e}"))
}

/// Load all blobs into a map (used when reconstructing a full transcript).
fn load_all_blobs(conn: &Connection) -> Result<HashMap<String, Vec<u8>>, String> {
    let mut blobs = HashMap::new();
    let mut stmt = conn
        .prepare("select id, data from blobs")
        .map_err(|e| format!("prepare blobs: {e}"))?;
    let rows = stmt
        .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Vec<u8>>(1)?)))
        .map_err(|e| format!("query blobs: {e}"))?;
    for row in rows {
        let (id, data) = row.map_err(|e| format!("blob row: {e}"))?;
        blobs.insert(id, data);
    }
    Ok(blobs)
}

/// One clear preview line used when the store cannot be decoded.
fn error_preview(text: impl Into<String>) -> Vec<PreviewMessage> {
    vec![PreviewMessage {
        role: "system".to_string(),
        text: text.into(),
    }]
}

/// Reconstruct ordered preview messages (and optional title) from a Cursor `store.db`.
///
/// Degrades to a single system line on failure; never panics. A 0-byte store
/// is treated as empty without opening SQLite. The name is read from the same
/// connection as the transcript so callers that need both (preview pane) do
/// not open SQLite twice.
fn load_cursor_transcript(store_path: &Path) -> (Option<String>, Vec<PreviewMessage>) {
    let len = match fs::metadata(store_path) {
        Ok(m) => m.len(),
        Err(_) => return (None, error_preview("Cursor store.db not found")),
    };
    if len == 0 {
        return (None, Vec::new());
    }

    let (conn, _tmp) = match open_store_readonly(store_path) {
        Ok(c) => c,
        Err(e) => return (None, error_preview(format!("Failed to open Cursor store: {e}"))),
    };

    let raw: String = match conn.query_row("select value from meta limit 1", [], |r| r.get(0)) {
        Ok(v) => v,
        Err(e) => return (None, error_preview(format!("Cursor store has no meta row: {e}"))),
    };
    let bytes = match hex_decode(&raw) {
        Some(b) => b,
        None => return (None, error_preview("Cursor meta value is not valid hex")),
    };
    let meta: Value = match serde_json::from_slice(&bytes) {
        Ok(v) => v,
        Err(e) => return (None, error_preview(format!("Cursor meta JSON corrupt: {e}"))),
    };
    let name = meta
        .get("name")
        .and_then(|v| v.as_str())
        .filter(|n| !n.is_empty() && *n != "New Agent")
        .map(|s| s.to_string());
    let root_id = match meta.get("latestRootBlobId").and_then(|v| v.as_str()) {
        Some(id) => id,
        None => return (name, error_preview("Cursor store missing latestRootBlobId")),
    };

    let blobs = match load_all_blobs(&conn) {
        Ok(b) => b,
        Err(e) => return (name, error_preview(format!("Failed to read Cursor blobs: {e}"))),
    };

    let root = match blobs.get(root_id) {
        Some(b) => b,
        // Missing root is a visible error, not a panic or empty list.
        None => return (name, error_preview("Cursor root blob missing or dangling")),
    };

    let refs = blob_refs(root);
    let mut messages = Vec::new();
    let mut saw_user = false;

    for r in refs {
        let Some(data) = blobs.get(&r) else {
            continue;
        };
        if data.first() != Some(&b'{') {
            continue;
        }
        let msg: Value = match serde_json::from_slice(data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let role = msg
            .get("role")
            .and_then(|v| v.as_str())
            .unwrap_or("?")
            .to_string();

        // Cursor's system prompt is ~2k words of static boilerplate; Claude
        // previews never show system turns, so skip these entirely.
        if role == "system" {
            continue;
        }

        // Suppress the injected environment preamble (first user message).
        if role == "user" && !saw_user {
            saw_user = true;
            let raw_text = flatten_content(msg.get("content").unwrap_or(&Value::Null));
            if raw_text.contains("<user_info>") {
                continue;
            }
            // First user without user_info still counts as "first"; show it.
            let text = strip_cursor_wrappers(&raw_text);
            if !text.is_empty() {
                messages.push(PreviewMessage { role, text });
            }
            continue;
        }
        if role == "user" {
            saw_user = true;
        }

        let raw_text = flatten_content(msg.get("content").unwrap_or(&Value::Null));
        let text = if role == "user" {
            strip_cursor_wrappers(&raw_text)
        } else {
            // tool-call / tool-result markers must stay intact; only strip XML
            // from prose portions via the Claude stripper on the whole string.
            let stripped = super::history::strip_xml_tags(&raw_text);
            stripped.trim().to_string()
        };

        if text.is_empty() {
            continue;
        }
        messages.push(PreviewMessage { role, text });
    }

    (name, messages)
}

/// Load a Cursor chat preview: last 20 turns, with cwd filled from `project`.
pub fn load_cursor_preview(project: &str, store_path: &Path) -> (super::types::SessionMeta, Vec<PreviewMessage>) {
    let (name, messages) = load_cursor_transcript(store_path);
    let meta = super::types::SessionMeta {
        cwd: Some(project.to_string()),
        session_name: name,
        ..Default::default()
    };
    let start = messages.len().saturating_sub(20);
    (meta, messages[start..].to_vec())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    /// SHA-256 hex id for content-addressed blob storage.
    fn blob_id(data: &[u8]) -> String {
        hex_encode(&Sha256::digest(data))
    }

    /// Build a minimal protobuf index blob with repeated field-1 32-byte refs.
    fn make_root_blob(child_ids_hex: &[String]) -> Vec<u8> {
        let mut out = Vec::new();
        for id in child_ids_hex {
            let raw = hex_decode(id).expect("hex id");
            assert_eq!(raw.len(), 32);
            // field 1, wire type 2 → key = (1 << 3) | 2 = 0x0a
            out.push(0x0a);
            // length varint 32
            out.push(32);
            out.extend_from_slice(&raw);
        }
        out
    }

    /// Create a synthetic store.db with the given JSON message blobs in order.
    fn write_store(dir: &Path, messages: &[Value]) -> PathBuf {
        let path = dir.join("store.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE meta  (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();

        let mut child_ids = Vec::new();
        for msg in messages {
            let data = serde_json::to_vec(msg).unwrap();
            let id = blob_id(&data);
            conn.execute("insert into blobs (id, data) values (?1, ?2)", (&id, &data))
                .unwrap();
            child_ids.push(id);
        }
        let root = make_root_blob(&child_ids);
        let root_id = blob_id(&root);
        conn.execute("insert into blobs (id, data) values (?1, ?2)", (&root_id, &root))
            .unwrap();

        let meta = serde_json::json!({
            "agentId": "test-agent",
            "latestRootBlobId": root_id,
            "name": "New Agent",
        });
        let hex = hex_encode(serde_json::to_string(&meta).unwrap().as_bytes());
        conn.execute("insert into meta (key, value) values ('0', ?1)", [&hex])
            .unwrap();
        path
    }

    #[test]
    fn ordered_role_reconstruction() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = vec![
            serde_json::json!({"role": "system", "content": "sys"}),
            serde_json::json!({"role": "user", "content": "<user_info>env</user_info>"}),
            serde_json::json!({"role": "user", "content": "<user_query>hello</user_query>"}),
            serde_json::json!({"role": "assistant", "content": [{"type": "text", "text": "hi"}]}),
        ];
        let path = write_store(dir.path(), &msgs);
        let (_, preview) = load_cursor_transcript(&path);
        // system skipped, first user (user_info) suppressed, second user + assistant kept
        assert_eq!(preview.len(), 2);
        assert_eq!(preview[0].role, "user");
        assert_eq!(preview[0].text, "hello");
        assert_eq!(preview[1].role, "assistant");
        assert_eq!(preview[1].text, "hi");
        let meta = read_store_meta(&path).unwrap();
        // One genuine user + one assistant (not system, not user_info).
        assert_eq!(meta.entry_count, 2);
    }

    #[test]
    fn entry_count_excludes_system_and_user_info_preamble() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = vec![
            serde_json::json!({"role": "system", "content": "long boilerplate"}),
            serde_json::json!({"role": "user", "content": "<user_info>env</user_info>"}),
            serde_json::json!({"role": "user", "content": "<user_query>hi</user_query>"}),
            serde_json::json!({"role": "assistant", "content": "hello"}),
        ];
        let path = write_store(dir.path(), &msgs);
        let meta = read_store_meta(&path).unwrap();
        assert_eq!(meta.entry_count, 2, "one-exchange chat should read 2 msg");
    }

    #[test]
    fn wrapper_stripping_keeps_user_query_body() {
        let text = strip_cursor_wrappers(
            "<timestamp>Wed</timestamp>\n<user_query>Use the shell</user_query>",
        );
        assert_eq!(text, "Use the shell");
    }

    #[test]
    fn user_info_preamble_is_suppressed() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = vec![
            serde_json::json!({"role": "user", "content": "<user_info>OS Version: darwin</user_info>"}),
            serde_json::json!({"role": "user", "content": "<user_query>real question</user_query>"}),
            serde_json::json!({"role": "assistant", "content": "ok"}),
        ];
        let path = write_store(dir.path(), &msgs);
        let (_, preview) = load_cursor_transcript(&path);
        assert!(preview.iter().all(|m| !m.text.contains("OS Version")));
        assert_eq!(preview[0].text, "real question");
    }

    #[test]
    fn zero_byte_store_is_empty_and_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        fs::File::create(&path).unwrap();
        assert_eq!(fs::metadata(&path).unwrap().len(), 0);
        let (_, preview) = load_cursor_transcript(&path);
        assert!(preview.is_empty());
        let meta = read_store_meta(&path).unwrap();
        assert_eq!(meta.entry_count, 0);
        assert!(meta.name.is_none());
    }

    #[test]
    fn missing_root_blob_degrades_visibly() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE meta  (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        let meta = serde_json::json!({
            "latestRootBlobId": "aa".repeat(32),
            "name": "New Agent",
        });
        let hex = hex_encode(serde_json::to_string(&meta).unwrap().as_bytes());
        conn.execute("insert into meta (key, value) values ('0', ?1)", [&hex])
            .unwrap();
        drop(conn);

        let (_, preview) = load_cursor_transcript(&path);
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].role, "system");
        assert!(preview[0].text.contains("root blob missing"));
    }

    #[test]
    fn tool_result_renders_as_marker() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = vec![
            serde_json::json!({"role": "user", "content": "<user_info>x</user_info>"}),
            serde_json::json!({"role": "assistant", "content": [
                {"type": "text", "text": "running"},
                {"type": "tool-call", "toolName": "Shell"}
            ]}),
            serde_json::json!({"role": "tool", "content": [
                {"type": "tool-result", "toolName": "Shell", "result": "hello"}
            ]}),
        ];
        let path = write_store(dir.path(), &msgs);
        let (_, preview) = load_cursor_transcript(&path);
        let tool = preview.iter().find(|m| m.role == "tool").unwrap();
        assert!(tool.text.contains("[tool-result \"Shell\"]") || tool.text.contains("[tool-result Shell]"));
        let assistant = preview.iter().find(|m| m.role == "assistant").unwrap();
        assert!(assistant.text.contains("tool-call"));
    }

    #[test]
    fn reasoning_blocks_are_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let msgs = vec![
            serde_json::json!({"role": "user", "content": "<user_info>x</user_info>"}),
            serde_json::json!({"role": "assistant", "content": [
                {"type": "reasoning", "text": "", "signature": "opaque-sig"},
                {"type": "text", "text": "visible"}
            ]}),
        ];
        let path = write_store(dir.path(), &msgs);
        let (_, preview) = load_cursor_transcript(&path);
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].text, "visible");
        assert!(!preview[0].text.contains("opaque-sig"));
    }

    #[test]
    fn new_agent_title_is_treated_as_untitled() {
        let dir = tempfile::tempdir().unwrap();
        let path = write_store(dir.path(), &[serde_json::json!({"role": "system", "content": "x"})]);
        let meta = read_store_meta(&path).unwrap();
        assert!(meta.name.is_none());
    }

    /// Manual check against real fixture DBs outside the repo. Ignored by default.
    #[test]
    #[ignore = "needs local fixtures at ~/Dev/ccsm-cursor-fixtures"]
    fn fixture_tool_chat_matches_reference_roles() {
        let home = dirs::home_dir().expect("home");
        let base = home.join(
            "Dev/ccsm-cursor-fixtures/cursor/chats/ca31439697121595d577b6047eae1794",
        );
        let tool_store = base.join("334ca342-33d8-4963-94b1-601ba1bf0e2d/store.db");
        let simple_store = base.join("f42dd464-c77f-428f-8863-5c42b3b0ae81/store.db");
        assert!(tool_store.exists(), "fixture missing: {}", tool_store.display());
        assert!(simple_store.exists(), "fixture missing: {}", simple_store.display());

        let (_, msgs) = load_cursor_transcript(&tool_store);
        // 8 root refs minus system and user_info preamble → 6 preview lines.
        let roles: Vec<&str> = msgs.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(
            roles,
            vec!["user", "assistant", "tool", "assistant", "tool", "assistant"]
        );
        assert!(msgs.iter().all(|m| m.role != "system"));
        assert!(msgs[0].text.contains("cat note.txt") || msgs[0].text.contains("note.txt"));
        assert!(!msgs[0].text.contains("<user_query>"));
        assert!(msgs[1].text.contains("[tool-call \"Shell\"]"));
        assert_eq!(msgs[2].text, "[tool-result \"Shell\"]");
        assert!(msgs[3].text.contains("hello fixture"));
        assert!(msgs[3].text.contains("[tool-call \"Shell\"]"));
        assert_eq!(msgs[4].text, "[tool-result \"Shell\"]");

        let (_, simple) = load_cursor_transcript(&simple_store);
        // 6 root refs minus system and user_info → 4 lines: user, assistant, user, assistant
        assert_eq!(simple.len(), 4);
        assert_eq!(simple[0].role, "user");
        assert_eq!(simple[1].role, "assistant");
        assert!(simple.iter().all(|m| m.role != "system"));
    }
}
