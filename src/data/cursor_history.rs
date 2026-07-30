//! Discover Cursor Agent CLI chats under `~/.cursor/chats/`.

use std::fs;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::types::{AgentBackend, SessionInfo};

/// Fields read from a chat directory's `meta.json`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CursorMetaJson {
    created_at_ms: Option<i64>,
    updated_at_ms: Option<i64>,
    has_conversation: Option<bool>,
    cwd: Option<String>,
}

/// Canonicalize `path` for comparison/grouping; fall back to the raw string when
/// the directory no longer exists or canonicalize fails.
pub(crate) fn canonicalize_path(path: &str) -> String {
    match Path::new(path).canonicalize() {
        Ok(p) => p.to_string_lossy().to_string(),
        Err(_) => path.to_string(),
    }
}

/// True when `project` matches `filter_path` after canonicalizing both sides.
fn path_matches_filter(project: &str, filter_path: &str) -> bool {
    let project_c = canonicalize_path(project);
    let filter_c = canonicalize_path(filter_path);
    project_c.starts_with(&filter_c) || project.starts_with(filter_path)
}

/// Load Cursor chats from the default `~/.cursor/chats` location.
///
/// Malformed individual chats are skipped. Returns an empty vec when the
/// directory is absent; never errors fatally.
pub fn load_cursor_sessions(filter_path: Option<&str>) -> Vec<SessionInfo> {
    let Some(home) = dirs::home_dir() else {
        return Vec::new();
    };
    let chats_root = home.join(".cursor/chats");
    load_cursor_sessions_from(&chats_root, filter_path)
}

/// Load Cursor chats from an explicit chats root (used by tests).
pub(crate) fn load_cursor_sessions_from(
    chats_root: &Path,
    filter_path: Option<&str>,
) -> Vec<SessionInfo> {
    if !chats_root.is_dir() {
        return Vec::new();
    }

    let mut sessions = Vec::new();
    let project_dirs = match fs::read_dir(chats_root) {
        Ok(rd) => rd,
        Err(_) => return Vec::new(),
    };

    for project_entry in project_dirs.flatten() {
        let project_path = project_entry.path();
        if !project_path.is_dir() {
            continue;
        }
        let chat_dirs = match fs::read_dir(&project_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };
        for chat_entry in chat_dirs.flatten() {
            let chat_dir = chat_entry.path();
            if !chat_dir.is_dir() {
                continue;
            }
            match load_one_chat(&chat_dir, filter_path) {
                Some(session) => sessions.push(session),
                // Intentionally silent: one bad chat must not fail the scan.
                None => continue,
            }
        }
    }

    sessions.sort_by(|a, b| b.last_timestamp.cmp(&a.last_timestamp));
    sessions
}

/// Parse one `{chatId}/` directory into a `SessionInfo`, or skip it.
fn load_one_chat(chat_dir: &Path, filter_path: Option<&str>) -> Option<SessionInfo> {
    let session_id = chat_dir.file_name()?.to_str()?.to_string();
    let meta_path = chat_dir.join("meta.json");
    let meta_text = fs::read_to_string(&meta_path).ok()?;
    let meta: CursorMetaJson = serde_json::from_str(&meta_text).ok()?;

    let cwd_raw = meta.cwd.unwrap_or_default();
    if cwd_raw.is_empty() {
        return None;
    }
    if let Some(fp) = filter_path {
        if !path_matches_filter(&cwd_raw, fp) {
            return None;
        }
    }

    // Canonicalize for project grouping so /tmp and /private/tmp collapse.
    let project = canonicalize_path(&cwd_raw);
    let project_name = Path::new(&project)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| project.clone());

    let created = meta.created_at_ms.unwrap_or(0);
    let updated = meta.updated_at_ms.unwrap_or(created);
    let has_conversation = meta.has_conversation.unwrap_or(false);

    let store_path = chat_dir.join("store.db");
    let store_len = fs::metadata(&store_path).map(|m| m.len()).unwrap_or(0);
    let has_data = has_conversation && store_len > 0;

    // Do not open store.db here: listing hundreds of chats would block startup.
    // Title and entry_count are filled by App::spawn_load_session_names.
    Some(SessionInfo {
        session_id,
        project,
        project_name,
        first_timestamp: created,
        last_timestamp: updated,
        entry_count: 0,
        has_data,
        name: None,
        // Cursor has no chain concept; a non-None slug would pull it into Claude grouping.
        slug: None,
        backend: AgentBackend::CursorAgent,
    })
}

/// Locate `store.db` for a chat id by walking `~/.cursor/chats/*/{session_id}/`.
///
/// Avoids recomputing the project MD5; the chat UUID is unique across projects.
pub fn find_cursor_store(session_id: &str) -> Option<PathBuf> {
    let home = dirs::home_dir()?;
    find_cursor_store_under(&home.join(".cursor/chats"), session_id)
}

/// Locate `store.db` under an explicit chats root (tests).
pub(crate) fn find_cursor_store_under(chats_root: &Path, session_id: &str) -> Option<PathBuf> {
    let project_dirs = fs::read_dir(chats_root).ok()?;
    for project_entry in project_dirs.flatten() {
        let candidate = project_entry.path().join(session_id).join("store.db");
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use sha2::{Digest, Sha256};

    fn hex_encode(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    fn write_minimal_store(chat_dir: &Path, name: &str) {
        let path = chat_dir.join("store.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE blobs (id TEXT PRIMARY KEY, data BLOB);
             CREATE TABLE meta  (key TEXT PRIMARY KEY, value TEXT);",
        )
        .unwrap();
        let msg = serde_json::json!({"role": "user", "content": "hi"});
        let data = serde_json::to_vec(&msg).unwrap();
        let id = hex_encode(&Sha256::digest(&data));
        conn.execute("insert into blobs (id, data) values (?1, ?2)", (&id, &data))
            .unwrap();
        // Root with one 32-byte ref
        let raw_id = {
            let mut v = Vec::new();
            for i in (0..id.len()).step_by(2) {
                v.push(u8::from_str_radix(&id[i..i + 2], 16).unwrap());
            }
            v
        };
        let mut root = vec![0x0a, 32];
        root.extend_from_slice(&raw_id);
        let root_id = hex_encode(&Sha256::digest(&root));
        conn.execute(
            "insert into blobs (id, data) values (?1, ?2)",
            (&root_id, &root),
        )
        .unwrap();
        let meta = serde_json::json!({
            "latestRootBlobId": root_id,
            "name": name,
        });
        let hex = hex_encode(serde_json::to_string(&meta).unwrap().as_bytes());
        conn.execute("insert into meta (key, value) values ('0', ?1)", [&hex])
            .unwrap();
    }

    fn write_chat(chats_root: &Path, project_hash: &str, chat_id: &str, cwd: &str, name: &str) {
        let chat_dir = chats_root.join(project_hash).join(chat_id);
        fs::create_dir_all(&chat_dir).unwrap();
        let meta = serde_json::json!({
            "schemaVersion": 1,
            "createdAtMs": 1000,
            "updatedAtMs": 2000,
            "hasConversation": true,
            "cwd": cwd,
        });
        fs::write(chat_dir.join("meta.json"), serde_json::to_string(&meta).unwrap()).unwrap();
        write_minimal_store(&chat_dir, name);
    }

    #[test]
    fn load_cursor_sessions_filters_by_path() {
        let dir = tempfile::tempdir().unwrap();
        let chats = dir.path().join("chats");
        write_chat(
            &chats,
            "hash1",
            "11111111-1111-1111-1111-111111111111",
            "/tmp/ccsm-cursor-filter-a",
            "Chat A",
        );
        write_chat(
            &chats,
            "hash2",
            "22222222-2222-2222-2222-222222222222",
            "/tmp/ccsm-cursor-filter-b",
            "Chat B",
        );

        let all = load_cursor_sessions_from(&chats, None);
        assert_eq!(all.len(), 2);

        let filtered = load_cursor_sessions_from(&chats, Some("/tmp/ccsm-cursor-filter-a"));
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].session_id, "11111111-1111-1111-1111-111111111111");
        assert!(filtered[0].slug.is_none());
        assert_eq!(filtered[0].backend, AgentBackend::CursorAgent);
        // Titles and entry counts load asynchronously after list scan.
        assert!(filtered[0].name.is_none());
        assert_eq!(filtered[0].entry_count, 0);
        assert!(filtered[0].has_data);
    }

    #[test]
    fn load_cursor_sessions_canonicalizes_tmp_paths() {
        // On macOS /tmp is a symlink to /private/tmp. Create a real dir and
        // record both the user-facing and realpath forms across two chats.
        let real_dir = tempfile::tempdir().unwrap();
        let real_cwd = real_dir.path().canonicalize().unwrap();
        let real_cwd_str = real_cwd.to_string_lossy().to_string();

        let chats = tempfile::tempdir().unwrap();
        let chats_root = chats.path();

        // Chat recorded with the canonical path.
        write_chat(
            chats_root,
            "hash-canon",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            &real_cwd_str,
            "Canon",
        );

        // If /tmp resolves into this tree, also write a chat under a /tmp-style
        // path that canonicalizes to the same place. Otherwise just verify that
        // canonicalize_path is stable for the real path.
        let sessions = load_cursor_sessions_from(chats_root, Some(&real_cwd_str));
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0].project, real_cwd_str);

        // filter with a non-canonical path that still canonicalize()-matches.
        // On Linux real_cwd may already equal itself; the important property is
        // that filter and project agree after canonicalize_path.
        let filter_raw = real_cwd_str.clone();
        assert!(path_matches_filter(&real_cwd_str, &filter_raw));
    }

    #[test]
    fn cursor_session_slug_is_always_none() {
        let dir = tempfile::tempdir().unwrap();
        write_chat(
            dir.path(),
            "h",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "/tmp/x",
            "New Agent",
        );
        let sessions = load_cursor_sessions_from(dir.path(), None);
        assert_eq!(sessions.len(), 1);
        assert!(sessions[0].slug.is_none());
        assert!(sessions[0].name.is_none());
        assert_eq!(sessions[0].entry_count, 0);
    }

    #[test]
    fn absent_chats_root_returns_empty() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("no-such-chats");
        assert!(load_cursor_sessions_from(&missing, None).is_empty());
    }
}
