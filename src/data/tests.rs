use super::history::{read_session_meta, strip_xml_tags};
use super::io::project_to_dir_name;
use super::preview::{load_chain_preview, load_preview};
use super::types::*;

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
fn test_read_session_meta_finds_slug() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_slug");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("test_slug.jsonl");

    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"system","slug":"happy-flying-penguin","content":"init"}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#).unwrap();

    let (slug, exit_only) = read_session_meta(&path);
    assert_eq!(slug, Some("happy-flying-penguin".to_string()));
    assert!(!exit_only);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_session_meta_missing_slug_returns_none() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_slug2");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("no_slug.jsonl");

    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello"}}}}"#).unwrap();

    let (slug, exit_only) = read_session_meta(&path);
    assert_eq!(slug, None);
    assert!(!exit_only);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_session_meta_only_checks_first_20_lines() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_slug3");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("late_slug.jsonl");

    let mut f = std::fs::File::create(&path).unwrap();
    // 20 lines without slug
    for _ in 0..20 {
        writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"x"}}}}"#).unwrap();
    }
    // slug on line 21 (index 20) — should not be found
    writeln!(f, r#"{{"type":"system","slug":"late-slug","content":"init"}}"#).unwrap();

    let (slug, _) = read_session_meta(&path);
    assert_eq!(slug, None);

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_session_meta_exit_only_session() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_exit_only");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("exit_only.jsonl");

    // Matches real exit-only session structure: system line, meta user messages,
    // file-history-snapshot, /exit command, and local-command-stdout "Bye!"
    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"system","subtype":"bridge_status","content":"init","isMeta":false}}"#).unwrap();
    writeln!(f, r#"{{"type":"file-history-snapshot","data":{{}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"<local-command-caveat>Caveat</local-command-caveat>"}},"isMeta":true}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"<command-name>/exit</command-name>\n<command-message>exit</command-message>"}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"<local-command-stdout>Bye!</local-command-stdout>"}}}}"#).unwrap();

    let (_, exit_only) = read_session_meta(&path);
    assert!(exit_only, "Session with only /exit user message should be exit_only");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_session_meta_real_conversation_not_exit_only() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_not_exit_only");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("real_convo.jsonl");

    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"file-history-snapshot","data":{{}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"user","message":{{"role":"user","content":"hello world"}}}}"#).unwrap();
    writeln!(f, r#"{{"type":"assistant","message":{{"role":"assistant","content":"Hi! How can I help?"}}}}"#).unwrap();

    let (_, exit_only) = read_session_meta(&path);
    assert!(!exit_only, "Session with real conversation should not be exit_only");

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_read_session_meta_no_user_messages_not_exit_only() {
    use std::io::Write;
    let dir = std::env::temp_dir().join("ccsm_test_no_user");
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("no_user.jsonl");

    let mut f = std::fs::File::create(&path).unwrap();
    writeln!(f, r#"{{"type":"file-history-snapshot","data":{{}}}}"#).unwrap();

    let (_, exit_only) = read_session_meta(&path);
    assert!(!exit_only, "Session with no user messages should not be exit_only");

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
