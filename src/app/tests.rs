use super::*;
use crate::config::Config;
use crate::data::AgentBackend;
use std::path::PathBuf;
use tui_input::Input;

/// Creates an App with live sessions cleared so tests are not affected by
/// any tmux sessions running on the host machine.
fn make_app(sessions: Vec<SessionInfo>, filter_path: Option<String>, config: Config) -> App {
    let mut app = App::new(sessions, filter_path, config);
    app.live_sessions = vec![];
    app.recompute_flat_rows();
    app.recompute_tree();
    app
}

fn make_sessions() -> Vec<SessionInfo> {
    vec![
        SessionInfo {
            session_id: "s1".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1000,
            last_timestamp: 2000,
            entry_count: 5,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s2".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 1500,
            last_timestamp: 3000,
            entry_count: 3,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s3".into(),
            project: "/Users/sane/Dev/gamma".into(),
            project_name: "gamma".into(),
            first_timestamp: 500,
            last_timestamp: 4000,
            entry_count: 10,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
    ]
}

#[test]
fn test_new_app_initializes_all_indices() {
    let app = make_app(make_sessions(), None, Config::default());
    // Sorted by last_timestamp desc: s3(4000), s2(3000), s1(2000) → [2, 1, 0]
    assert_eq!(app.filtered_indices, vec![2, 1, 0]);
    assert_eq!(app.selected, 0);
    assert!(!app.filter_active);
    assert!(app.filter_input.value().is_empty());
    assert!(app.tree_view);
    assert!(!app.shift_active);
}

#[test]
fn test_new_app_starts_all_collapsed() {
    let app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    // All groups collapsed: only headers visible
    assert!(app.tree_rows.iter().all(|r| matches!(r, TreeRow::Header { .. })));
    assert_eq!(app.tree_rows.len(), 2); // beta header + alpha header
}

#[test]
fn test_right_arrow_expands_collapsed_header() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    // All collapsed, selected=0 is first header (beta)
    app.selected = 0;
    let project = match &app.tree_rows[0] {
        TreeRow::Header { project, .. } => project.clone(),
        _ => panic!("expected header"),
    };
    assert!(app.collapsed.contains(&project));

    // Simulate expand (project + its history sub-section)
    app.collapsed.remove(&project);
    app.collapsed.remove(&format!("history:{}", project));
    app.recompute_tree();

    // beta now expanded: header + history-header + 2 sessions
    assert!(!app.collapsed.contains(&project));
    assert!(matches!(&app.tree_rows[1], TreeRow::HistoryHeader { .. }));
    assert!(matches!(&app.tree_rows[2], TreeRow::Session { .. }));
}

#[test]
fn test_left_arrow_collapses_expanded_header() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    // Expand beta first
    let project = match &app.tree_rows[0] {
        TreeRow::Header { project, .. } => project.clone(),
        _ => panic!("expected header"),
    };
    app.collapsed.remove(&project);
    app.recompute_tree();
    let expanded_len = app.tree_rows.len();

    // Now collapse
    app.collapsed.insert(project.clone());
    app.recompute_tree();
    assert!(app.tree_rows.len() < expanded_len);
    assert!(app.collapsed.contains(&project));
}

#[test]
fn test_filter_narrows_indices() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = Input::from("beta");
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![1]);
}

#[test]
fn test_filter_case_insensitive() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = Input::from("ALPHA");
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![0]);
}

#[test]
fn test_filter_matches_path() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = Input::from("/Dev/gamma");
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![2]);
}

#[test]
fn test_filter_no_match() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = Input::from("nonexistent");
    app.recompute_filter();
    assert!(app.filtered_indices.is_empty());
    assert_eq!(app.selected_session_index(), None);
}

#[test]
fn test_clear_filter_restores_all() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = Input::from("beta");
    app.recompute_filter();
    assert_eq!(app.filtered_indices.len(), 1);

    app.filter_input = Input::default();
    app.recompute_filter();
    // Sorted by last_timestamp desc: s3(4000), s2(3000), s1(2000) → [2, 1, 0]
    assert_eq!(app.filtered_indices, vec![2, 1, 0]);
}

#[test]
fn test_selected_clamps_on_filter() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.tree_view = false;
    app.selected = 2;
    app.filter_input = Input::from("alpha");
    app.recompute_filter();
    // selected was 2 but only 1 match, should clamp to 0
    assert_eq!(app.selected, 0);
    assert_eq!(app.selected_session_index(), Some(0));
}

#[test]
fn test_selected_session_index() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.tree_view = false;
    app.filter_input = Input::from("amma"); // matches only gamma
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![2]);
    app.selected = 0;
    assert_eq!(app.selected_session_index(), Some(2));
}

#[test]
fn test_filter_path_stored() {
    let app = make_app(make_sessions(), Some("/Users/sane/Dev".into()), Config::default());
    assert_eq!(app.filter_path.as_deref(), Some("/Users/sane/Dev"));
}

fn make_sessions_with_shared_projects() -> Vec<SessionInfo> {
    vec![
        SessionInfo {
            session_id: "s1".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1000,
            last_timestamp: 5000,
            entry_count: 5,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s2".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 1500,
            last_timestamp: 3000,
            entry_count: 3,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s3".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 500,
            last_timestamp: 4000,
            entry_count: 10,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s4".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 2000,
            last_timestamp: 6000,
            entry_count: 2,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
    ]
}

#[test]
fn test_tree_grouping() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    app.display_mode = DisplayMode::Name;
    app.recompute_tree();
    // Expand all groups to test full tree structure
    app.collapsed.clear();
    app.recompute_tree();

    // beta group first (s4 has last_timestamp=6000), then alpha (s1 has 5000)
    // filtered_indices sorted desc: [3(6000), 0(5000), 2(4000), 1(3000)]
    // tree: beta header → HistoryHeader → s4(idx=3), s2(idx=1) ; alpha header → HistoryHeader → s1(idx=0), s3(idx=2)
    assert_eq!(app.tree_rows.len(), 8); // 2 headers + 2 history-headers + 4 sessions
    assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, session_count, .. } if project_name == "beta" && *session_count == 2));
    assert!(matches!(&app.tree_rows[1], TreeRow::HistoryHeader { count: 2, .. }));
    assert!(matches!(&app.tree_rows[2], TreeRow::Session { session_index: 3 }));
    assert!(matches!(&app.tree_rows[3], TreeRow::Session { session_index: 1 }));
    assert!(matches!(&app.tree_rows[4], TreeRow::Header { project_name, session_count, .. } if project_name == "alpha" && *session_count == 2));
    assert!(matches!(&app.tree_rows[5], TreeRow::HistoryHeader { count: 2, .. }));
    assert!(matches!(&app.tree_rows[6], TreeRow::Session { session_index: 0 }));
    assert!(matches!(&app.tree_rows[7], TreeRow::Session { session_index: 2 }));
}

#[test]
fn test_tree_collapse_expand() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    app.display_mode = DisplayMode::Name;
    app.recompute_tree();
    // Start: all collapsed, only headers
    assert_eq!(app.tree_rows.len(), 2);

    // Expand all
    app.collapsed.clear();
    app.recompute_tree();
    assert_eq!(app.tree_rows.len(), 8); // 2 headers + 2 history-headers + 4 sessions

    // Collapse beta
    app.collapsed.insert("/Users/sane/Dev/beta".into());
    app.recompute_tree();
    assert_eq!(app.tree_rows.len(), 5); // beta header + alpha header + alpha history-header + 2 alpha sessions
    assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, .. } if project_name == "beta"));
    assert!(matches!(&app.tree_rows[1], TreeRow::Header { project_name, .. } if project_name == "alpha"));

    // Expand beta
    app.collapsed.remove("/Users/sane/Dev/beta");
    app.recompute_tree();
    assert_eq!(app.tree_rows.len(), 8);
}

#[test]
fn test_selected_session_index_returns_none_for_header() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    app.selected = 0; // header row (all collapsed)
    assert_eq!(app.selected_session_index(), None);
}

#[test]
fn test_selected_session_index_returns_some_for_session_in_tree() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    app.collapsed.clear();
    app.recompute_tree();
    app.selected = 2; // first session row under first header (beta → HistoryHeader → s4, session_index=3)
    assert_eq!(app.selected_session_index(), Some(3));
}

#[test]
fn test_visible_item_count_flat_vs_tree() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    // Default is tree view, all collapsed: 2 headers
    assert_eq!(app.visible_item_count(), 2);

    // Expand all
    app.collapsed.clear();
    app.recompute_tree();
    assert_eq!(app.visible_item_count(), 8); // 2 headers + 2 history-headers + 4 sessions

    // Switch to flat
    app.tree_view = false;
    assert_eq!(app.visible_item_count(), 4); // 4 sessions
}

#[test]
fn test_tree_with_filter() {
    let mut app = make_app(make_sessions_with_shared_projects(), None, Config::default());
    app.display_mode = DisplayMode::Name;
    app.filter_input = Input::from("alpha");
    app.recompute_filter();
    // Only alpha sessions should appear, but collapsed
    assert_eq!(app.tree_rows.len(), 1); // 1 header (collapsed)
    assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, .. } if project_name == "alpha"));

    // Expand to see sessions
    app.collapsed.remove("/Users/sane/Dev/alpha");
    app.collapsed.remove("history:/Users/sane/Dev/alpha");
    app.recompute_filter();
    assert_eq!(app.tree_rows.len(), 4); // 1 header + 1 history-header + 2 sessions
}

fn make_sessions_with_projects() -> Vec<SessionInfo> {
    vec![
        SessionInfo {
            session_id: "s1".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1000,
            last_timestamp: 5000,
            entry_count: 5,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s2".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1500,
            last_timestamp: 3000,
            entry_count: 3,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s3".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 500,
            last_timestamp: 4000,
            entry_count: 10,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s4".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 2000,
            last_timestamp: 6000,
            entry_count: 2,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
    ]
}

#[test]
fn test_short_dir_groups_by_project() {
    let mut app = make_app(make_sessions_with_projects(), None, Config::default());
    app.display_mode = DisplayMode::ShortDir;
    app.collapsed.clear();
    app.recompute_tree();

    // 2 groups: beta (ts=6000) and alpha (ts=5000)
    let headers: Vec<_> = app.tree_rows.iter().filter(|r| matches!(r, TreeRow::Header { .. })).collect();
    assert_eq!(headers.len(), 2);

    // First group: beta (truncated)
    assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, session_count, .. }
        if project_name == "Dev/beta" && *session_count == 1));

    // Second group: alpha (3 sessions, truncated) — after beta's HistoryHeader + 1 Session
    assert!(matches!(&app.tree_rows[3], TreeRow::Header { project_name, session_count, .. }
        if project_name == "Dev/alpha" && *session_count == 3));
}

#[test]
fn test_display_mode_toggle_changes_display_name() {
    let mut app = make_app(make_sessions_with_projects(), None, Config::default());
    app.display_mode = DisplayMode::ShortDir;
    app.recompute_tree();
    let headers: Vec<_> = app.tree_rows.iter().filter(|r| matches!(r, TreeRow::Header { .. })).collect();
    assert_eq!(headers.len(), 2);

    app.display_mode = DisplayMode::Name;
    app.recompute_tree();
    let headers: Vec<_> = app.tree_rows.iter().filter(|r| matches!(r, TreeRow::Header { .. })).collect();
    assert_eq!(headers.len(), 2);
}

#[test]
fn test_display_name_short_dir() {
    let app = make_app(make_sessions_with_projects(), None, Config {
        display_mode: DisplayMode::ShortDir,
        ..Config::default()
    });
    assert_eq!(app.display_name(&app.sessions[0]), "Dev/alpha");
    assert_eq!(app.display_name(&app.sessions[3]), "Dev/beta");
}

#[test]
fn test_display_name_project_name() {
    let app = make_app(make_sessions_with_projects(), None, Config::default());
    assert_eq!(app.display_name(&app.sessions[0]), "alpha");
    assert_eq!(app.display_name(&app.sessions[3]), "beta");
}

#[test]
fn test_display_name_full_dir() {
    let app = make_app(make_sessions_with_projects(), None, Config {
        display_mode: DisplayMode::FullDir,
        ..Config::default()
    });
    assert_eq!(app.display_name(&app.sessions[0]), "/Users/sane/Dev/alpha");
    assert_eq!(app.display_name(&app.sessions[3]), "/Users/sane/Dev/beta");
}

#[test]
fn test_app_default_mode_is_normal() {
    let app = make_app(make_sessions(), None, Config::default());
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn test_selected_cwd_from_session() {
    let mut app = make_app(make_sessions_with_projects(), None, Config::default());
    app.collapsed.clear();
    app.recompute_tree();
    // Select first session (under first header)
    app.selected = 1;
    let cwd = app.selected_cwd();
    assert!(cwd.is_some());
    let cwd_str = cwd.unwrap();
    assert!(cwd_str.contains("beta"));
}

#[test]
fn test_selected_cwd_from_header() {
    let app = make_app(make_sessions_with_projects(), None, Config::default());
    // selected=0 is a header
    let cwd = app.selected_cwd();
    assert!(cwd.is_some());
}

#[test]
fn test_launch_request_resume_variant() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.collapsed.clear();
    app.recompute_tree();
    // Find a session row
    let session_idx = app.tree_rows.iter().position(|r| matches!(r, TreeRow::Session { .. }));
    if let Some(idx) = session_idx {
        app.selected = idx;
        if let Some(TreeRow::Session { session_index }) = app.tree_rows.get(idx) {
            let session = &app.sessions[*session_index];
            app.launch_session = Some(LaunchRequest::Resume {
                session_id: session.session_id.clone(),
                cwd: session.project.clone(),
                backend: AgentBackend::ClaudeCode,
            });
        }
    }
    if let Some(LaunchRequest::Resume { session_id, .. }) = &app.launch_session {
        assert!(!session_id.is_empty());
    }
}

#[test]
fn test_reload_sessions_updates_list() {
    let mut app = make_app(make_sessions(), None, Config::default());
    let original_count = app.sessions.len();

    // Simulate a new session appearing after a Claude session ends
    let mut updated = make_sessions();
    updated.push(SessionInfo {
        session_id: "new-session".into(),
        project: "/Users/sane/Dev/new-project".into(),
        project_name: "new-project".into(),
        first_timestamp: 9000,
        last_timestamp: 9500,
        entry_count: 3,
        has_data: true,
        name: None,
        slug: None,
            ..Default::default()
        });

    app.reload_sessions(updated);
    assert_eq!(app.sessions.len(), original_count + 1);
    assert!(app.sessions.iter().any(|s| s.session_id == "new-session"));
    // Preview cache should be cleared
    assert!(app.preview_cache.is_empty());
    // Filtered indices should be recomputed
    assert_eq!(app.filtered_indices.len(), app.sessions.len());
}

fn make_sessions_mixed_data() -> Vec<SessionInfo> {
    vec![
        SessionInfo {
            session_id: "s1".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1000,
            last_timestamp: 2000,
            entry_count: 5,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s2".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 1500,
            last_timestamp: 3000,
            entry_count: 3,
            has_data: false,
            name: None,
            slug: None,
            ..Default::default()
        },
        SessionInfo {
            session_id: "s3".into(),
            project: "/Users/sane/Dev/gamma".into(),
            project_name: "gamma".into(),
            first_timestamp: 500,
            last_timestamp: 4000,
            entry_count: 10,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
    ]
}

#[test]
fn test_hide_empty_filters_sessions() {
    // Default config has hide_empty=true, so empty sessions are filtered at construction
    let mut app = make_app(make_sessions_mixed_data(), None, Config::default());
    app.tree_view = false;
    app.recompute_filter();
    // s2 (index 1) has_data=false, should be excluded; sorted desc: s3(4000), s1(2000) → [2, 0]
    assert_eq!(app.filtered_indices, vec![2, 0]);

    // Disabling hide_empty shows all sessions; sorted desc: s3(4000), s2(3000), s1(2000) → [2, 1, 0]
    app.hide_empty = false;
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![2, 1, 0]);
}

#[test]
fn test_hide_empty_with_text_filter() {
    let mut app = make_app(make_sessions_mixed_data(), None, Config::default());
    app.tree_view = false;
    app.hide_empty = true;
    app.filter_input = Input::from("a"); // matches alpha and gamma; sorted desc: s3(4000), s1(2000) → [2, 0]
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![2, 0]);

    // beta matches text but has_data=false
    app.filter_input = Input::from("beta");
    app.recompute_filter();
    assert!(app.filtered_indices.is_empty());
}

#[test]
fn test_tab_cycles_through_view_modes() {
    let mut app = make_app(make_sessions(), None, Config::default());
    // Default: tree_view=true, display_mode=Name
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);

    // Tab 1: tree+Name → tree+ShortDir
    app.tree_view = true;
    app.display_mode = DisplayMode::Name;
    // Simulate Tab cycle logic
    app.display_mode = DisplayMode::ShortDir;
    app.recompute_tree();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::ShortDir);

    // Tab 2: tree+ShortDir → tree+FullDir
    app.display_mode = DisplayMode::FullDir;
    app.recompute_tree();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::FullDir);

    // Tab 3: tree+FullDir → flat
    app.tree_view = false;
    assert!(!app.tree_view);

    // Tab 4: flat → tree+Name
    app.tree_view = true;
    app.display_mode = DisplayMode::Name;
    app.recompute_tree();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);
}

#[test]
fn test_shift_active_default_false() {
    let app = make_app(make_sessions(), None, Config::default());
    assert!(!app.shift_active);
}

#[test]
fn test_tab_cycles_all_six_modes() {
    let mut app = make_app(make_sessions(), None, Config::default());

    // Start: tree + Name
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);

    app.cycle_view_forward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::ShortDir);

    app.cycle_view_forward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::FullDir);

    app.cycle_view_forward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);

    app.cycle_view_forward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::ShortDir);

    app.cycle_view_forward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::FullDir);

    // Full cycle back to tree + Name
    app.cycle_view_forward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);
}

#[test]
fn test_backtab_cycles_reverse() {
    let mut app = make_app(make_sessions(), None, Config::default());

    // Start: tree + Name
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);

    // Reverse: tree+Name → flat+FullDir
    app.cycle_view_backward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::FullDir);

    app.cycle_view_backward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::ShortDir);

    app.cycle_view_backward();
    assert!(!app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);

    app.cycle_view_backward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::FullDir);

    app.cycle_view_backward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::ShortDir);

    app.cycle_view_backward();
    assert!(app.tree_view);
    assert_eq!(app.display_mode, DisplayMode::Name);
}

#[test]
fn test_config_selected_bounds() {
    use crate::ui::config_tab::CONFIG_MAX_ROW;
    let mut app = make_app(make_sessions(), None, Config::default());
    app.main_tab = MainTab::Config;
    assert_eq!(app.config_selected, 0);

    // Can't go below 0
    app.config_selected = 0;
    if app.config_selected > 0 {
        app.config_selected -= 1;
    }
    assert_eq!(app.config_selected, 0);

    // Navigate to the last settings row (About URL)
    app.config_selected = CONFIG_MAX_ROW;
    assert_eq!(app.config_selected, CONFIG_MAX_ROW);

    // Can't go above CONFIG_MAX_ROW
    if app.config_selected < CONFIG_MAX_ROW {
        app.config_selected += 1;
    }
    assert_eq!(app.config_selected, CONFIG_MAX_ROW);
}

#[test]
fn test_config_toggle_hide_empty() {
    let mut app = make_app(make_sessions_mixed_data(), None, Config::default());
    app.tree_view = false;
    app.main_tab = MainTab::Config;
    app.config_selected = 0;

    // Default: hide_empty = true
    assert!(app.hide_empty);
    app.recompute_filter();
    assert_eq!(app.filtered_indices.len(), 2); // s1, s3 (s2 has no data)

    // Toggle hide_empty off
    app.hide_empty = !app.hide_empty;
    app.recompute_filter();
    assert!(!app.hide_empty);
    assert_eq!(app.filtered_indices.len(), 3); // all sessions visible
}

#[test]
fn test_config_toggle_group_chains() {
    let mut app = make_app(make_chained_sessions(), None, Config::default());
    app.tree_view = false;
    app.main_tab = MainTab::Config;
    app.config_selected = 1;

    // Default: group_chains = true
    assert!(app.group_chains);
    app.recompute_filter();
    assert_eq!(app.filtered_indices.len(), 2); // chain collapsed

    // Toggle group_chains off
    app.group_chains = !app.group_chains;
    app.preview_cache.clear();
    app.recompute_filter();
    assert!(!app.group_chains);
    assert_eq!(app.filtered_indices.len(), 3); // all sessions visible
}

#[test]
fn test_session_name_set_directly() {
    let mut app = make_app(make_sessions(), None, Config::default());
    // Initially no names
    assert!(app.sessions[0].name.is_none());

    // Directly set a name (simulates what rename does)
    app.sessions[0].name = Some("My Session".to_string());
    assert_eq!(app.sessions[0].name, Some("My Session".to_string()));
}

#[test]
fn test_rename_mode_transitions() {
    let mut app = make_app(make_sessions(), None, Config::default());
    // Select a session (expand first header, then move to session)
    app.tree_view = false;
    app.recompute_filter();
    app.selected = 0;

    // Start renaming
    let idx = app.selected_session_index().unwrap();
    let session_id = app.sessions[idx].session_id.clone();
    app.rename_session_id = Some(session_id.clone());
    app.rename_input = Input::default();
    app.mode = AppMode::Renaming;

    assert_eq!(app.mode, AppMode::Renaming);
    assert_eq!(app.rename_session_id, Some(session_id));
}

#[test]
fn test_hide_empty_toggle_restores() {
    let mut app = make_app(make_sessions_mixed_data(), None, Config::default());
    app.tree_view = false;

    app.hide_empty = true;
    app.recompute_filter();
    // sorted desc: s3(4000), s1(2000) → [2, 0]
    assert_eq!(app.filtered_indices, vec![2, 0]);

    app.hide_empty = false;
    app.recompute_filter();
    // sorted desc: s3(4000), s2(3000), s1(2000) → [2, 1, 0]
    assert_eq!(app.filtered_indices, vec![2, 1, 0]);
}

fn make_chained_sessions() -> Vec<SessionInfo> {
    vec![
        // Two sessions sharing slug "cool-flying-cat" — form a chain
        SessionInfo {
            session_id: "chain-a".into(),
            project: "/test/proj".into(),
            project_name: "proj".into(),
            first_timestamp: 1000,
            last_timestamp: 2000,
            entry_count: 4,
            has_data: true,
            name: None,
            slug: Some("cool-flying-cat".into()),
            ..Default::default()
        },
        SessionInfo {
            session_id: "chain-b".into(),
            project: "/test/proj".into(),
            project_name: "proj".into(),
            first_timestamp: 2500,
            last_timestamp: 4000,
            entry_count: 6,
            has_data: true,
            name: None,
            slug: Some("cool-flying-cat".into()),
            ..Default::default()
        },
        // Standalone session without a slug
        SessionInfo {
            session_id: "standalone".into(),
            project: "/test/other".into(),
            project_name: "other".into(),
            first_timestamp: 500,
            last_timestamp: 5000,
            entry_count: 2,
            has_data: true,
            name: None,
            slug: None,
            ..Default::default()
        },
    ]
}

#[test]
fn test_recompute_filter_groups_chains() {
    let mut app = make_app(make_chained_sessions(), None, Config::default());
    app.tree_view = false;
    app.group_chains = true;
    app.recompute_filter();

    // Two entries: standalone (last_ts=5000) and canonical for chain (last_ts=4000)
    assert_eq!(app.filtered_indices.len(), 2);
    // Standalone session (index 2, last_ts=5000) should come first
    assert_eq!(app.filtered_indices[0], 2);
    // Canonical chain entry = chain-b (index 1, last_ts=4000)
    assert_eq!(app.filtered_indices[1], 1);
    // chain_map should have canonical (1) → [0, 1] ordered oldest first
    let chain = app.chain_map.get(&1).expect("chain_map should have entry for canonical");
    assert_eq!(chain, &vec![0usize, 1usize]);
}

#[test]
fn test_recompute_filter_ungrouped_mode() {
    let mut app = make_app(make_chained_sessions(), None, Config::default());
    app.tree_view = false;
    app.group_chains = false;
    app.recompute_filter();

    // All 3 sessions appear independently
    assert_eq!(app.filtered_indices.len(), 3);
    assert!(app.chain_map.is_empty());
}

#[test]
fn test_chain_entry_count_sums_chain() {
    let mut app = make_app(make_chained_sessions(), None, Config::default());
    app.tree_view = false;
    app.group_chains = true;
    app.recompute_filter();

    // canonical_idx = 1 (chain-b); chain = [0, 1] with counts 4+6=10
    assert_eq!(app.chain_entry_count(1), 10);
    // standalone (idx=2) has no chain entry, returns its own count
    assert_eq!(app.chain_entry_count(2), 2);
}

#[test]
fn test_single_slug_session_not_chained() {
    // A single session with a slug but no partner should appear standalone
    let sessions = vec![SessionInfo {
        session_id: "solo".into(),
        project: "/test/solo".into(),
        project_name: "solo".into(),
        first_timestamp: 1000,
        last_timestamp: 2000,
        entry_count: 3,
        has_data: true,
        name: None,
        slug: Some("lone-slug".into()),
            ..Default::default()
        }];
    let mut app = make_app(sessions, None, Config::default());
    app.tree_view = false;
    app.group_chains = true;
    app.recompute_filter();

    assert_eq!(app.filtered_indices, vec![0]);
    assert!(app.chain_map.is_empty());
}

#[test]
fn truncate_path_trailing_slash() {
    assert_eq!(truncate_path("/Users/sane/Dev/"), "sane/Dev");
}

#[test]
fn truncate_path_normal() {
    assert_eq!(truncate_path("/Users/sane/Dev/ccsm"), "Dev/ccsm");
}

#[test]
fn truncate_path_single_component() {
    assert_eq!(truncate_path("foo"), "foo");
}

#[test]
fn truncate_path_multiple_trailing_slashes() {
    assert_eq!(truncate_path("/a/b/c//"), "b/c");
}

#[test]
fn preview_auto_scroll_defaults_to_true() {
    let app = make_app(make_sessions(), None, Config::default());
    assert!(app.preview_auto_scroll);
    assert_eq!(app.preview_scroll, u16::MAX);
}

#[test]
fn reload_sessions_resets_auto_scroll() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.preview_auto_scroll = false;
    app.preview_scroll = 42;
    app.reload_sessions(make_sessions());
    assert!(app.preview_auto_scroll);
    assert_eq!(app.preview_scroll, u16::MAX);
}

#[test]
fn mouse_scroll_down_increments_preview_scroll() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.preview_scroll = 0;
    // Simulate scroll down
    app.preview_scroll = app.preview_scroll.saturating_add(3);
    assert_eq!(app.preview_scroll, 3);
    app.preview_scroll = app.preview_scroll.saturating_add(3);
    assert_eq!(app.preview_scroll, 6);
}

#[test]
fn mouse_scroll_up_decrements_preview_scroll_and_disables_auto_scroll() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.preview_scroll = 10;
    app.preview_auto_scroll = true;
    // Simulate scroll up
    app.preview_auto_scroll = false;
    app.preview_scroll = app.preview_scroll.saturating_sub(3);
    assert_eq!(app.preview_scroll, 7);
    assert!(!app.preview_auto_scroll);
}

#[test]
fn mouse_scroll_up_saturates_at_zero() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.preview_scroll = 1;
    app.preview_auto_scroll = false;
    app.preview_scroll = app.preview_scroll.saturating_sub(3);
    assert_eq!(app.preview_scroll, 0);
}

#[test]
fn naming_mode_defaults_to_plain() {
    let app = make_app(make_sessions(), None, Config::default());
    assert_eq!(app.naming_mode, NewSessionMode::Plain);
}

/// Drive the real naming popup: set up the launch mode, then press Enter.
fn confirm_naming(app: &mut App, mode: NewSessionMode) {
    app.naming_mode = mode;
    app.mode = AppMode::NamingSession;
    app.naming_cwd = Some("/tmp".into());
    app.naming_placeholder = "test-session".into();
    app.naming_input = tui_input::Input::default();
    app.handle_naming_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
}

#[test]
fn naming_carries_the_dangerous_flag_into_the_launch_request() {
    let mut app = make_app(make_sessions(), None, Config::default());
    confirm_naming(&mut app, NewSessionMode::Dangerous);

    assert_eq!(
        app.naming_mode,
        NewSessionMode::Plain,
        "naming_mode should reset after confirm"
    );
    match &app.launch_session {
        Some(LaunchRequest::NewLive { name, dangerous, worktree, .. }) => {
            assert_eq!(name, "test-session");
            assert!(dangerous);
            assert!(!worktree);
        }
        other => panic!("Expected NewLive, got {:?}", other),
    }
}

#[test]
fn naming_carries_the_worktree_flag_into_the_launch_request() {
    let mut app = make_app(make_sessions(), None, Config::default());
    confirm_naming(&mut app, NewSessionMode::Worktree);

    assert_eq!(
        app.naming_mode,
        NewSessionMode::Plain,
        "naming_mode should reset after confirm"
    );
    match &app.launch_session {
        Some(LaunchRequest::NewLive { worktree, dangerous, .. }) => {
            assert!(worktree);
            assert!(!dangerous);
        }
        other => panic!("Expected NewLive, got {:?}", other),
    }
}

#[test]
fn naming_plain_session_sets_neither_flag() {
    let mut app = make_app(make_sessions(), None, Config::default());
    confirm_naming(&mut app, NewSessionMode::Plain);

    match &app.launch_session {
        Some(LaunchRequest::NewLive { name, dangerous, worktree, .. }) => {
            assert_eq!(name, "test-session");
            assert!(!dangerous);
            assert!(!worktree);
        }
        other => panic!("Expected NewLive, got {:?}", other),
    }
}

#[test]
fn open_naming_popup_refuses_a_worktree_outside_a_git_repo() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().to_string_lossy().to_string();
    // Point every session at the (non-repo) temp dir so whichever row the
    // selection lands on resolves to it.
    let mut sessions = make_sessions();
    for session in &mut sessions {
        session.project = path.clone();
        session.project_name = "tmp".into();
    }
    let mut app = make_app(sessions, None, Config::default());

    assert!(!app.open_naming_popup(NewSessionMode::Worktree));
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.status_error.is_some());
}

// --- Job form model picker ---

#[test]
fn cycling_the_model_walks_the_discovered_list_and_wraps() {
    let mut app = make_app(make_sessions(), None, Config::default());
    let first = app.model_options[0].value.clone();
    let second = app.model_options[1].value.clone();
    let last = app.model_options[app.model_options.len() - 1].value.clone();

    app.job_form_model = first.clone();
    app.cycle_job_form_model(true);
    assert_eq!(app.job_form_model, second);

    app.job_form_model = first.clone();
    app.cycle_job_form_model(false);
    assert_eq!(app.job_form_model, last);
}

#[test]
fn cycling_from_a_hand_typed_model_rejoins_the_list() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.job_form_model = "some-unlisted-model".to_string();
    app.cycle_job_form_model(true);
    assert_eq!(app.job_form_model, app.model_options[1].value);
}

#[test]
fn cycling_is_a_no_op_when_no_models_were_discovered() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.model_options.clear();
    app.job_form_model = "opus".to_string();
    app.cycle_job_form_model(true);
    assert_eq!(app.job_form_model, "opus");
}

// --- Directory picker (DirBrowser) ---

#[test]
fn test_refresh_lists_dirs_only_hidden_included_sorted_case_insensitive() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join(".git")).unwrap();
    std::fs::create_dir(dir.path().join("Src")).unwrap();
    std::fs::create_dir(dir.path().join("apple")).unwrap();
    std::fs::write(dir.path().join("readme.txt"), b"hi").unwrap();

    let browser = DirBrowser::new(dir.path().to_path_buf());
    let names: Vec<&str> = browser.entries.iter().map(|e| e.name.as_str()).collect();

    // ".." first (a tempdir always has a parent), then case-insensitive alpha order.
    // The file "readme.txt" must be excluded entirely.
    assert_eq!(names, vec!["..", ".git", "apple", "Src"]);
    assert!(browser.entries.iter().all(|e| e.is_dir));
}

#[test]
fn test_refresh_at_filesystem_root_has_no_parent_entry() {
    let browser = DirBrowser::new(PathBuf::from("/"));
    assert!(!browser.entries.iter().any(|e| e.name == ".."));
}

#[test]
fn test_enter_selected_descends_and_go_up_ascends() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("child");
    std::fs::create_dir(&sub).unwrap();

    let mut browser = DirBrowser::new(dir.path().to_path_buf());
    let idx = browser.entries.iter().position(|e| e.name == "child").unwrap();
    browser.selected = idx;
    browser.enter_selected();
    assert_eq!(browser.current_dir, sub);
    // refresh() ran on the new directory: it has a parent, so ".." is present.
    assert!(browser.entries.iter().any(|e| e.name == ".."));

    browser.go_up();
    assert_eq!(browser.current_dir, dir.path());
    // refresh() ran again: "child" is visible from the parent directory.
    assert!(browser.entries.iter().any(|e| e.name == "child"));
}

#[test]
fn test_refresh_nonexistent_directory_sets_error_keeps_previous_entries() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("real")).unwrap();

    let mut browser = DirBrowser::new(dir.path().to_path_buf());
    assert!(browser.error.is_none());
    let prev_len = browser.entries.len();
    assert!(prev_len > 0);

    browser.current_dir = dir.path().join("does-not-exist");
    browser.refresh();

    assert!(browser.error.is_some());
    assert_eq!(browser.entries.len(), prev_len, "entries should be left intact on error");
}

#[test]
fn test_apply_typed_path_accepts_existing_dir() {
    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("target");
    std::fs::create_dir(&sub).unwrap();

    let mut browser = DirBrowser::new(dir.path().to_path_buf());
    browser.path_input = Input::from(sub.to_string_lossy().to_string());
    browser.input_active = true;
    browser.apply_typed_path();

    assert_eq!(browser.current_dir, sub);
    assert!(!browser.input_active);
    assert!(browser.error.is_none());
}

#[test]
fn test_apply_typed_path_rejects_file() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("file.txt");
    std::fs::write(&file, b"hi").unwrap();

    let mut browser = DirBrowser::new(dir.path().to_path_buf());
    browser.path_input = Input::from(file.to_string_lossy().to_string());
    browser.input_active = true;
    browser.apply_typed_path();

    assert!(browser.error.is_some());
    assert_eq!(browser.current_dir, dir.path());
    assert!(browser.input_active, "input should stay open on error");
}

#[test]
fn test_apply_typed_path_rejects_nonexistent() {
    let dir = tempfile::tempdir().unwrap();
    let mut browser = DirBrowser::new(dir.path().to_path_buf());
    browser.path_input = Input::from(dir.path().join("nope").to_string_lossy().to_string());
    browser.input_active = true;
    browser.apply_typed_path();

    assert!(browser.error.is_some());
    assert!(browser.input_active);
}

#[test]
fn test_apply_typed_path_expands_tilde() {
    let mut browser = DirBrowser::new(std::env::current_dir().unwrap());
    browser.path_input = Input::from("~".to_string());
    browser.input_active = true;
    browser.apply_typed_path();

    assert_eq!(browser.current_dir, dirs::home_dir().unwrap());
    assert!(!browser.input_active);
}

#[test]
fn test_open_dir_picker_sets_mode() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_dir_picker();
    assert_eq!(app.mode, AppMode::DirPicker);
    assert!(app.dir_browser.is_some());
}

#[test]
fn test_dir_picker_escape_closes_input_before_picker() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_dir_picker();
    app.dir_picker_activate_input();
    assert!(app.dir_browser.as_ref().unwrap().input_active);

    // First Esc closes just the input, staying in DirPicker mode.
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::DirPicker);
    assert!(!app.dir_browser.as_ref().unwrap().input_active);

    // Second Esc closes the whole picker.
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::Normal);
    assert!(app.dir_browser.is_none());
}

#[test]
fn test_dir_picker_select_moves_to_naming_session() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_dir_picker();
    app.dir_picker_select();
    assert_eq!(app.mode, AppMode::NamingSession);
    assert!(app.naming_cwd.is_some());
    assert!(app.dir_browser.is_none());
}

#[test]
fn test_dir_picker_move_clamps_at_bounds() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("a")).unwrap();
    std::fs::create_dir(dir.path().join("b")).unwrap();

    let mut app = make_app(make_sessions(), None, Config::default());
    app.dir_browser = Some(DirBrowser::new(dir.path().to_path_buf()));
    let count = app.dir_browser.as_ref().unwrap().entries.len(); // "..", "a", "b"
    assert_eq!(count, 3);

    // Already at the top: moving up stays at 0.
    app.dir_picker_move_up();
    assert_eq!(app.dir_browser.as_ref().unwrap().selected, 0);

    // Moving down past the last entry stops at the last index.
    for _ in 0..10 {
        app.dir_picker_move_down();
    }
    assert_eq!(app.dir_browser.as_ref().unwrap().selected, count - 1);

    // Moving up from the bottom decreases by exactly one.
    app.dir_picker_move_up();
    assert_eq!(app.dir_browser.as_ref().unwrap().selected, count - 2);
}

// ---------------------------------------------------------------------
// Jobs manager modals
// ---------------------------------------------------------------------

/// A live session fixture, optionally carrying a scheduler job tag.
fn make_live(tmux_name: &str, cwd: &str, job_id: Option<&str>) -> crate::live::LiveSession {
    crate::live::LiveSession {
        tmux_name: tmux_name.to_string(),
        display_name: tmux_name.to_string(),
        cwd: cwd.to_string(),
        project_name: "proj".to_string(),
        job_id: job_id.map(|s| s.to_string()),
        backend: None,
    }
}

#[test]
fn w_switches_to_the_jobs_tab_and_esc_returns_to_sessions() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_jobs_tab();
    assert_eq!(app.main_tab, MainTab::Jobs);
    assert_eq!(app.mode, AppMode::Normal, "the Jobs tab is not a modal");
    app.open_sessions_tab();
    assert_eq!(app.main_tab, MainTab::Sessions);
}

#[test]
fn tab_cycles_through_the_three_main_tabs() {
    let mut app = make_app(make_sessions(), None, Config::default());
    assert_eq!(app.main_tab, MainTab::Sessions);
    app.cycle_main_tab(true);
    assert_eq!(app.main_tab, MainTab::Jobs);
    app.cycle_main_tab(true);
    assert_eq!(app.main_tab, MainTab::Config);
    app.cycle_main_tab(true);
    assert_eq!(app.main_tab, MainTab::Sessions, "and wraps around");

    app.cycle_main_tab(false);
    assert_eq!(app.main_tab, MainTab::Config, "Shift+Tab wraps the other way");
    app.cycle_main_tab(false);
    assert_eq!(app.main_tab, MainTab::Jobs);
    app.cycle_main_tab(false);
    assert_eq!(app.main_tab, MainTab::Sessions);
}

#[test]
fn help_opens_on_the_page_matching_the_current_tab() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_help();
    assert_eq!(app.mode, AppMode::Help);
    assert_eq!(app.help_tab, HelpTab::Sessions);

    app.mode = AppMode::Normal;
    app.main_tab = MainTab::Jobs;
    app.open_help();
    assert_eq!(app.help_tab, HelpTab::Jobs, "jobs help is default on the Jobs tab");

    // Tab order wraps in both directions.
    assert_eq!(HelpTab::Jobs.next(), HelpTab::General);
    assert_eq!(HelpTab::Sessions.prev(), HelpTab::General);
}

#[test]
fn new_job_form_starts_without_an_edit_id() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_jobs_tab();
    app.open_job_form_new();
    assert_eq!(app.mode, AppMode::JobForm);
    assert!(app.job_form_edit_id.is_none());
}

#[test]
fn job_form_from_a_historical_row_binds_the_chain_latest_session_id() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.tree_view = false;
    app.recompute_flat_rows();
    let Some(idx) = app.selected_session_index() else {
        return; // no historical row selected in this layout
    };
    let expected_id = app.resume_session_id_for(idx).to_string();
    let expected_cwd = app.sessions[idx].project.clone();
    app.job_form_from_selection();
    assert_eq!(app.mode, AppMode::JobForm);
    assert_eq!(app.job_form_bind, JobBind::Resume(expected_id));
    assert_eq!(app.job_form_cwd, expected_cwd);
}

#[test]
fn job_form_from_a_live_row_binds_the_tmux_name() {
    let mut app = make_app(vec![], None, Config::default());
    app.live_sessions = vec![make_live("alpha-A", "/tmp", None)];
    app.live_filter = true;
    app.tree_view = false;
    app.recompute_flat_rows();
    if app.selected_live_index().is_none() {
        return; // layout did not select a live row
    }
    app.job_form_from_selection();
    assert_eq!(app.job_form_bind, JobBind::Live("alpha-A".to_string()));
    assert_eq!(app.job_form_cwd, "/tmp");
}

#[test]
fn submitting_an_empty_name_keeps_the_form_open_with_an_error() {
    let mut app = make_app(vec![], None, Config::default());
    app.mode = AppMode::JobForm;
    app.job_form_name = "   ".to_string();
    app.job_form_cwd = "/tmp".to_string();
    app.submit_job_form();
    assert_eq!(app.mode, AppMode::JobForm, "must stay in the form");
    assert!(app.status_error.is_some());
}

#[test]
fn submitting_a_nonexistent_directory_keeps_the_form_open_with_an_error() {
    let mut app = make_app(vec![], None, Config::default());
    app.mode = AppMode::JobForm;
    app.job_form_name = "job".to_string();
    app.job_form_cwd = "/definitely/not/a/real/directory".to_string();
    app.submit_job_form();
    assert_eq!(app.mode, AppMode::JobForm);
    assert!(app.status_error.is_some());
}

#[test]
fn submitting_a_valid_form_enqueues_exactly_one_create_job() {
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let work = tempfile::tempdir().unwrap();
    let mut app = make_app(vec![], None, Config::default());
    app.mode = AppMode::JobForm;
    app.job_form_name = "my-job".to_string();
    app.job_form_cwd = work.path().to_string_lossy().to_string();
    app.job_form_prompt = "do the thing".to_string();
    app.submit_job_form();

    assert_eq!(app.mode, AppMode::Normal, "a valid submit closes the form");
    assert_eq!(app.main_tab, MainTab::Jobs, "and lands back on the Jobs tab");
    assert!(app.status_error.is_none());

    let (pending, warnings) = crate::schedule::command::read_pending();
    assert!(warnings.is_empty());
    assert_eq!(pending.len(), 1);
    match &pending[0].1 {
        crate::schedule::command::Command::CreateJob { job } => {
            assert_eq!(job.name, "my-job");
            assert_eq!(job.prompt, "do the thing");
            assert!(!job.id.is_empty());
        }
        other => panic!("expected CreateJob, got {other:?}"),
    }

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn submitting_a_live_bound_form_also_enqueues_adopt_live() {
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let work = tempfile::tempdir().unwrap();
    let mut app = make_app(vec![], None, Config::default());
    app.mode = AppMode::JobForm;
    app.job_form_name = "adopted".to_string();
    app.job_form_cwd = work.path().to_string_lossy().to_string();
    app.job_form_bind = JobBind::Live("live-A".to_string());
    app.submit_job_form();

    let (pending, _) = crate::schedule::command::read_pending();
    assert_eq!(pending.len(), 2, "CreateJob plus AdoptLive");
    assert!(matches!(
        pending[1].1,
        crate::schedule::command::Command::AdoptLive { .. }
    ));

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn x_on_a_tagged_live_session_enqueues_stop_job_instead_of_killing_tmux() {
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut app = make_app(vec![], None, Config::default());
    app.live_sessions = vec![make_live("managed-A", "/tmp", Some("job-abc"))];
    app.live_filter = true;
    app.tree_view = false;
    app.recompute_flat_rows();
    if app.selected_live_index().is_none() {
        std::env::remove_var("CCSM_CONFIG_DIR");
        return;
    }
    app.stop_selected_live_session();

    let (pending, _) = crate::schedule::command::read_pending();
    assert_eq!(pending.len(), 1, "managed sessions route through the daemon");
    match &pending[0].1 {
        crate::schedule::command::Command::StopJob { id } => assert_eq!(id, "job-abc"),
        other => panic!("expected StopJob, got {other:?}"),
    }

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn jobs_navigation_clamps_at_both_ends() {
    let mut app = make_app(vec![], None, Config::default());
    app.jobs = vec![];
    app.jobs_selected = 0;
    app.jobs_move_up();
    assert_eq!(app.jobs_selected, 0, "must not underflow on an empty list");
    app.jobs_move_down();
    assert_eq!(app.jobs_selected, 0, "must not run past the end");
}

// ---------------------------------------------------------------------
// Attaching to sessions that are no longer running
// ---------------------------------------------------------------------

#[test]
fn attaching_to_a_missing_session_reports_instead_of_launching() {
    let mut app = make_app(vec![], None, Config::default());
    app.request_attach("ccsm-test-definitely-not-running".to_string());

    assert!(
        app.launch_session.is_none(),
        "a dead session must never reach the launch path, which aborts the app on failure"
    );
    let err = app.status_error.expect("the user must be told why nothing happened");
    assert!(err.contains("ccsm-test-definitely-not-running"), "got: {err}");
}

#[test]
fn attaching_to_a_missing_session_prunes_it_from_the_live_list() {
    let mut app = make_app(vec![], None, Config::default());
    app.live_sessions = vec![make_live("ccsm-test-ghost", "/tmp", None)];
    app.request_attach("ccsm-test-ghost".to_string());

    assert!(
        !app.live_sessions.iter().any(|s| s.tmux_name == "ccsm-test-ghost"),
        "the stale row must not survive a failed attach"
    );
}

#[test]
fn enter_on_a_job_with_no_session_reports_instead_of_launching() {
    // Isolate schedule.json: handle_jobs_tab_event polls it, and a real one on
    // disk would replace the job seeded below.
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut app = make_app(vec![], None, Config::default());
    // Every Job field has a serde default, so this is a queued job with no tmux_name.
    let job: crate::schedule::Job =
        serde_json::from_str(r#"{"id":"job-1","name":"queued-job","cwd":"/tmp"}"#).unwrap();
    app.jobs = vec![job];
    app.jobs_selected = 0;
    app.main_tab = MainTab::Jobs;
    app.handle_jobs_tab_event(key(crossterm::event::KeyCode::Enter));

    assert!(app.launch_session.is_none());
    assert!(app.status_error.is_some(), "a job with no tmux session must say so");

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn polling_live_sessions_drops_ones_that_are_gone() {
    let mut app = make_app(vec![], None, Config::default());
    app.live_sessions = vec![make_live("ccsm-test-vanished", "/tmp", Some("job-1"))];
    app.live_preview_cache
        .insert("ccsm-test-vanished".to_string(), (String::new(), std::time::Instant::now()));
    app.activity_states
        .insert("ccsm-test-vanished".to_string(), crate::live::ActivityState::Idle);
    app.recompute_flat_rows();

    assert!(app.poll_live_sessions(), "a session that no longer exists is a change");
    assert!(
        !app.live_sessions.iter().any(|s| s.tmux_name == "ccsm-test-vanished"),
        "a stopped session must stop being listed as running"
    );
    assert!(!app.live_preview_cache.contains_key("ccsm-test-vanished"));
    assert!(!app.activity_states.contains_key("ccsm-test-vanished"));
}

#[test]
fn polling_live_sessions_reports_no_change_when_nothing_moved() {
    let mut app = make_app(vec![], None, Config::default());
    // Sync to whatever tmux actually reports, then confirm a second poll is a no-op.
    app.poll_live_sessions();
    assert!(!app.poll_live_sessions(), "an unchanged set must not force a redraw");
}

// ---------------------------------------------------------------------
// Path pickers: browsing and manual entry for every path field
// ---------------------------------------------------------------------

/// A `KeyEvent` with no modifiers, for driving the modal key handlers directly.
fn key(code: crossterm::event::KeyCode) -> crossterm::event::KeyEvent {
    crossterm::event::KeyEvent::new(code, crossterm::event::KeyModifiers::NONE)
}

#[test]
fn file_pickers_list_files_while_directory_pickers_do_not() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("claude"), b"#!/bin/sh\n").unwrap();

    let dirs_only = DirBrowser::new(dir.path().to_path_buf());
    assert!(dirs_only.entries.iter().all(|e| e.is_dir));
    assert!(!dirs_only.entries.iter().any(|e| e.name == "claude"));

    let with_files = DirBrowser::with_kind(dir.path().to_path_buf(), PickerKind::File);
    assert!(with_files.entries.iter().any(|e| e.name == "claude" && !e.is_dir));
    // Directories still sort ahead of files so navigation stays at the top.
    let first_file = with_files.entries.iter().position(|e| !e.is_dir).unwrap();
    assert!(with_files.entries[..first_file].iter().all(|e| e.is_dir));
}

#[test]
fn file_picker_selection_only_commits_files() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("tmux"), b"x").unwrap();

    let mut browser = DirBrowser::with_kind(dir.path().to_path_buf(), PickerKind::File);
    let dir_idx = browser.entries.iter().position(|e| e.name == "sub").unwrap();
    browser.selected = dir_idx;
    assert!(browser.selected_path().is_none(), "a directory is not a valid file pick");

    let file_idx = browser.entries.iter().position(|e| e.name == "tmux").unwrap();
    browser.selected = file_idx;
    assert_eq!(browser.selected_path(), Some(dir.path().join("tmux")));
}

#[test]
fn picking_a_binary_writes_the_config_field_and_returns_to_the_config_tab() {
    let _guard = crate::config::test_lock();
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", cfg_dir.path());

    let dir = tempfile::tempdir().unwrap();
    let bin = dir.path().join("claude");
    std::fs::write(&bin, b"x").unwrap();

    let mut app = make_app(vec![], None, Config::default());
    app.open_config_tab();
    app.open_path_picker(PickerTarget::ConfigClaude, &bin.to_string_lossy());
    assert_eq!(app.mode, AppMode::DirPicker);
    assert_eq!(
        app.dir_browser.as_ref().unwrap().current_dir,
        dir.path(),
        "a file value starts the picker in its parent directory"
    );

    app.dir_picker_select();
    // The Config tab is a MainTab, not a mode, and the picker never left it:
    // closing the picker just drops back to Normal on top of it.
    assert_eq!(app.mode, AppMode::Normal, "commit returns to the Config tab");
    assert_eq!(app.main_tab, MainTab::Config);
    assert_eq!(app.config.claude_path, Some(bin.to_string_lossy().to_string()));
    assert!(app.dir_browser.is_none());

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn cancelling_a_picker_returns_to_whichever_mode_opened_it() {
    let mut app = make_app(vec![], None, Config::default());

    app.open_path_picker(PickerTarget::JobCwd, "");
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::JobForm);

    app.main_tab = MainTab::Config;
    app.open_path_picker(PickerTarget::ConfigTmux, "");
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::Normal);
    assert_eq!(app.main_tab, MainTab::Config, "and back onto the Config tab");
    app.main_tab = MainTab::Sessions;

    app.open_dir_picker();
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::Normal);
}

#[test]
fn typing_a_path_commits_it_when_it_matches_what_the_picker_wants() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("work");
    std::fs::create_dir(&target).unwrap();

    let mut app = make_app(vec![], None, Config::default());
    app.open_path_picker(PickerTarget::JobCwd, dir.path().to_str().unwrap());
    app.dir_picker_activate_input();
    app.dir_browser.as_mut().unwrap().path_input =
        Input::from(target.to_string_lossy().to_string());
    app.dir_picker_commit_input();

    assert_eq!(app.mode, AppMode::JobForm);
    assert_eq!(app.job_form_cwd, target.to_string_lossy().to_string());
}

#[test]
fn typing_a_directory_into_a_file_picker_navigates_instead_of_committing() {
    let _guard = crate::config::test_lock();
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", cfg_dir.path());

    let dir = tempfile::tempdir().unwrap();
    let sub = dir.path().join("bin");
    std::fs::create_dir(&sub).unwrap();

    let mut app = make_app(vec![], None, Config::default());
    app.open_path_picker(PickerTarget::ConfigUsage, "");
    app.dir_picker_activate_input();
    app.dir_browser.as_mut().unwrap().path_input = Input::from(sub.to_string_lossy().to_string());
    app.dir_picker_commit_input();

    assert_eq!(app.mode, AppMode::DirPicker, "still browsing");
    assert_eq!(app.dir_browser.as_ref().unwrap().current_dir, sub);
    assert!(app.config.usage_history_path.is_none(), "nothing was committed");

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn typing_a_bad_path_reports_an_error_without_closing_the_picker() {
    let mut app = make_app(vec![], None, Config::default());
    app.open_path_picker(PickerTarget::JobCwd, "");
    app.dir_picker_activate_input();
    app.dir_browser.as_mut().unwrap().path_input = Input::from("/definitely/not/here".to_string());
    app.dir_picker_commit_input();

    assert_eq!(app.mode, AppMode::DirPicker);
    assert!(app.dir_browser.as_ref().unwrap().error.is_some());
}

#[test]
fn the_job_form_directory_field_can_be_browsed_or_typed() {
    let mut app = make_app(vec![], None, Config::default());
    app.open_job_form_new();
    app.job_form_field = 1; // Directory

    // `b` browses.
    app.handle_job_form_event(key(crossterm::event::KeyCode::Char('b')));
    assert_eq!(app.mode, AppMode::DirPicker);
    assert_eq!(app.dir_picker_target, PickerTarget::JobCwd);
    app.dir_picker_escape();
    assert_eq!(app.mode, AppMode::JobForm);

    // `i` types the path by hand instead.
    app.handle_job_form_event(key(crossterm::event::KeyCode::Char('i')));
    assert!(app.job_form_editing);
    assert_eq!(app.mode, AppMode::JobForm);
}

#[test]
fn editing_a_job_form_field_supports_cursor_movement_and_mid_string_edits() {
    use crossterm::event::KeyCode;

    let mut app = make_app(vec![], None, Config::default());
    app.open_job_form_new();
    app.job_form_field = 0; // Name
    app.handle_job_form_event(key(KeyCode::Enter));
    assert!(app.job_form_editing);

    for c in "abcd".chars() {
        app.handle_job_form_event(key(KeyCode::Char(c)));
    }
    assert_eq!(app.job_form_input.value(), "abcd");
    assert_eq!(app.job_form_input.visual_cursor(), 4);

    // Left twice, then delete backwards: removes 'b', not the last character.
    app.handle_job_form_event(key(KeyCode::Left));
    app.handle_job_form_event(key(KeyCode::Left));
    assert_eq!(app.job_form_input.visual_cursor(), 2);
    app.handle_job_form_event(key(KeyCode::Backspace));
    assert_eq!(app.job_form_input.value(), "acd");

    // Home/End move to the extremes, and typing inserts at the cursor.
    app.handle_job_form_event(key(KeyCode::Home));
    app.handle_job_form_event(key(KeyCode::Char('X')));
    assert_eq!(app.job_form_input.value(), "Xacd");
    app.handle_job_form_event(key(KeyCode::End));
    app.handle_job_form_event(key(KeyCode::Char('Z')));
    assert_eq!(app.job_form_input.value(), "XacdZ");

    app.handle_job_form_event(key(KeyCode::Enter));
    assert!(!app.job_form_editing);
    assert_eq!(app.job_form_name, "XacdZ");
}

#[test]
fn editing_a_config_path_supports_cursor_movement() {
    use crossterm::event::KeyCode;

    let _guard = crate::config::test_lock();
    let cfg_dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", cfg_dir.path());

    let mut app = make_app(vec![], None, Config::default());
    app.main_tab = MainTab::Config;
    app.config_selected = 3; // Claude binary
    app.handle_config_tab_event(key(KeyCode::Char('i')));
    assert!(app.config_editing, "`i` types the path by hand");

    for c in "/bin/x".chars() {
        app.handle_config_tab_event(key(KeyCode::Char(c)));
    }
    app.handle_config_tab_event(key(KeyCode::Left));
    app.handle_config_tab_event(key(KeyCode::Backspace));
    assert_eq!(app.config_path_input.value(), "/binx", "deleted the '/' before 'x'");

    app.handle_config_tab_event(key(KeyCode::Enter));
    assert!(!app.config_editing);
    assert_eq!(app.config.claude_path, Some("/binx".to_string()));

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn enter_on_a_config_path_row_opens_the_file_picker() {
    use crossterm::event::KeyCode;

    let mut app = make_app(vec![], None, Config::default());
    app.main_tab = MainTab::Config;
    app.config_selected = 6; // usage history file (after agent path row)
    app.handle_config_tab_event(key(KeyCode::Enter));

    assert_eq!(app.mode, AppMode::DirPicker);
    assert_eq!(app.dir_picker_target, PickerTarget::ConfigUsage);
    assert_eq!(app.dir_browser.as_ref().unwrap().kind, PickerKind::File);
}

// --- Config tab: default continue prompt ---

/// Drive the Config tab's continue-prompt row: select it, press Enter to
/// start editing, replace the text, press Enter to commit.
fn commit_continue_prompt(app: &mut App, text: &str) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    app.main_tab = MainTab::Config;
    app.config_selected = crate::ui::config_tab::CONTINUE_PROMPT_ROW;
    app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(app.config_editing, "Enter should start editing the field");
    app.config_path_input = tui_input::Input::from(text.to_string());
    app.handle_config_tab_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!app.config_editing);
}

#[test]
fn continue_prompt_row_edits_the_configured_default() {
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut app = make_app(vec![], None, Config::default());
    assert_eq!(app.config.continue_prompt, "Continue where you left off.");
    commit_continue_prompt(&mut app, "Keep going.");
    assert_eq!(app.config.continue_prompt, "Keep going.");

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn clearing_the_continue_prompt_restores_the_built_in_default() {
    // An empty value would paste nothing and leave a paused job stuck, so the
    // field falls back rather than storing "".
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut app = make_app(vec![], None, Config::default());
    commit_continue_prompt(&mut app, "Keep going.");
    commit_continue_prompt(&mut app, "   ");
    assert_eq!(app.config.continue_prompt, Config::default().continue_prompt);
    assert!(!app.config.continue_prompt.is_empty());

    std::env::remove_var("CCSM_CONFIG_DIR");
}


// --- Keymap refactor ---

/// Press one key through the real Normal-mode dispatcher.
fn press(app: &mut App, code: crossterm::event::KeyCode) {
    press_mod(app, code, crossterm::event::KeyModifiers::NONE);
}

fn press_mod(
    app: &mut App,
    code: crossterm::event::KeyCode,
    mods: crossterm::event::KeyModifiers,
) {
    app.dispatch_normal_key(crossterm::event::KeyEvent::new(code, mods));
}

#[test]
fn esc_does_not_quit_the_app() {
    let mut app = make_app(make_sessions(), None, Config::default());
    press(&mut app, crossterm::event::KeyCode::Esc);
    assert!(
        !app.should_quit,
        "Esc must back out, never quit — q and Ctrl+C quit"
    );
}

#[test]
fn esc_clears_an_active_filter() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.filter_input = tui_input::Input::from("proj".to_string());
    app.recompute_filter();
    press(&mut app, crossterm::event::KeyCode::Esc);
    assert_eq!(app.filter_input.value(), "");
    assert!(!app.should_quit);
}

#[test]
fn q_still_quits() {
    let mut app = make_app(make_sessions(), None, Config::default());
    press(&mut app, crossterm::event::KeyCode::Char('q'));
    assert!(app.should_quit);
}

#[test]
fn space_toggles_favorite() {
    let mut app = make_app(make_sessions(), None, Config::default());
    let before = app.favorites.len();
    press(&mut app, crossterm::event::KeyCode::Char(' '));
    assert_ne!(app.favorites.len(), before, "Space should toggle a favorite");
}

#[test]
fn v_cycles_the_view_mode() {
    let mut app = make_app(make_sessions(), None, Config::default());
    let before = app.tree_view;
    let before_display = app.display_mode;
    press(&mut app, crossterm::event::KeyCode::Char('v'));
    assert!(
        app.tree_view != before || app.display_mode != before_display,
        "v should change the view"
    );
}

#[test]
fn x_asks_before_stopping_a_live_session() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.live_sessions = vec![make_live("sess", "/tmp", None)];
    app.recompute_flat_rows();
    app.recompute_tree();
    // Land the cursor on the live row.
    let live_row = (0..app.visible_item_count()).find(|&i| {
        app.selected = i;
        app.selected_live_index().is_some()
    });
    assert!(live_row.is_some(), "expected a live row to select");

    press(&mut app, crossterm::event::KeyCode::Char('x'));
    assert_eq!(
        app.mode,
        AppMode::StopSessionConfirm,
        "x must confirm, matching the Jobs tab"
    );
    assert_eq!(app.stop_confirm_name.as_deref(), Some("sess"));
}

#[test]
fn the_new_session_popup_cycles_launch_modes() {
    let mut app = make_app(make_sessions(), None, Config::default());
    // A non-repo cwd, so `worktree` must be skipped rather than offered.
    let dir = tempfile::tempdir().unwrap();
    app.naming_cwd = Some(dir.path().to_string_lossy().to_string());
    app.naming_mode = NewSessionMode::Plain;

    app.cycle_naming_mode(true);
    assert_eq!(app.naming_mode, NewSessionMode::Dangerous);
    app.cycle_naming_mode(true);
    assert_eq!(
        app.naming_mode,
        NewSessionMode::Direct,
        "worktree is unselectable outside a git repo"
    );
    app.cycle_naming_mode(true);
    assert_eq!(app.naming_mode, NewSessionMode::Plain);
}

#[test]
fn a_direct_new_session_ignores_the_name() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.mode = AppMode::NamingSession;
    app.naming_mode = NewSessionMode::Direct;
    app.naming_cwd = Some("/tmp".into());
    app.naming_input = tui_input::Input::from("ignored".to_string());
    app.handle_naming_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Enter,
        crossterm::event::KeyModifiers::NONE,
    ));
    match &app.launch_session {
        Some(LaunchRequest::NewDirect { cwd, backend }) => {
            assert_eq!(cwd, "/tmp");
            assert_eq!(*backend, AgentBackend::ClaudeCode);
        }
        other => panic!("expected NewDirect, got {other:?}"),
    }
}

#[test]
fn the_help_overlay_scrolls_and_resets_per_page() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.open_help();
    assert_eq!(app.help_scroll, 0);
    let help_key = |c| crossterm::event::KeyEvent::new(c, crossterm::event::KeyModifiers::NONE);
    app.handle_help_event(help_key(crossterm::event::KeyCode::Char('j')));
    app.handle_help_event(help_key(crossterm::event::KeyCode::Char('j')));
    assert_eq!(app.help_scroll, 2);
    app.handle_help_event(help_key(crossterm::event::KeyCode::Tab));
    assert_eq!(app.help_scroll, 0, "switching page resets the scroll");
    assert_eq!(app.mode, AppMode::Help, "scrolling must not close help");
}

// --- usage polling -----------------------------------------------------------
//
// The chip used to mirror only what the watch daemon persisted, so it froze
// whenever no daemon was running. These cover the TUI's own reading and the
// fallback to the daemon's.

/// Writes a two-sample `plan-usage-history.json` whose newest sample is
/// `age_seconds` old and reports `fh` percent on the 5-hour window.
fn write_history(dir: &tempfile::TempDir, fh: f64, age_seconds: i64) -> String {
    let now = crate::usage::now_ms();
    let path = dir.path().join("plan-usage-history.json");
    let json = format!(
        r#"{{"version":2,"samples":[
            {{"t":{},"org":"o","u":{{"fh":0,"sd":10}}}},
            {{"t":{},"org":"o","u":{{"fh":{fh},"sd":10}}}}
        ]}}"#,
        now - (age_seconds + 300) * 1000,
        now - age_seconds * 1000,
    );
    std::fs::write(&path, json).unwrap();
    path.to_string_lossy().to_string()
}

/// A config pointed at a throwaway history file rather than the host machine's.
fn usage_config(history: String) -> Config {
    let mut config = Config::default();
    config.usage_history_path = Some(history);
    config
}

#[test]
fn poll_usage_reads_the_local_history_file() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = make_app(vec![], None, usage_config(write_history(&dir, 42.0, 60)));
    app.usage = None;
    app.usage_polled_at = None;

    assert!(app.poll_usage(), "first poll produces a new reading");
    assert_eq!(app.usage_reading().pct, Some(42.0));
}

#[test]
fn poll_usage_rate_limits_itself() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = make_app(vec![], None, usage_config(write_history(&dir, 42.0, 60)));
    app.usage = None;
    app.usage_polled_at = None;

    assert!(app.poll_usage());
    assert!(
        !app.poll_usage(),
        "a second poll inside the interval must not re-read the file"
    );
}

#[test]
fn poll_usage_never_touches_the_api_source() {
    // Reaching the api source from the UI thread would block on HTTP and can
    // raise a macOS Keychain prompt, so a pinned `api` source is the daemon's
    // job alone.
    let dir = tempfile::tempdir().unwrap();
    let mut config = usage_config(write_history(&dir, 42.0, 60));
    config.usage_source = "api".to_string();
    let mut app = make_app(vec![], None, config);
    app.usage = None;
    app.usage_polled_at = None;

    assert!(!app.poll_usage());
    assert!(app.usage.is_none());
}

#[test]
fn poll_usage_keeps_the_last_sample_when_the_file_goes_bad() {
    let dir = tempfile::tempdir().unwrap();
    let history = write_history(&dir, 42.0, 60);
    let mut app = make_app(vec![], None, usage_config(history.clone()));
    app.usage = None;
    app.usage_polled_at = None;
    assert!(app.poll_usage());

    std::fs::write(&history, "{ truncated").unwrap();
    app.usage_polled_at = None;
    assert!(!app.poll_usage(), "an unreadable file is not a new reading");
    assert_eq!(
        app.usage_reading().pct,
        Some(42.0),
        "the previous sample survives a bad read"
    );
}

#[test]
fn usage_reading_prefers_our_own_sample_over_the_daemons() {
    let dir = tempfile::tempdir().unwrap();
    let mut app = make_app(vec![], None, usage_config(write_history(&dir, 42.0, 60)));
    app.usage = None;
    app.usage_polled_at = None;
    app.watch_state = Some(schedule::store::WatchState {
        pid: 1,
        started_at_ms: 0,
        heartbeat_ms: 0,
        last_usage_pct: Some(99.0),
        last_usage_at_ms: Some(0),
        reset_at_ms: None,
        usage_error: None,
        version: None,
    });

    app.poll_usage();
    assert_eq!(app.usage_reading().pct, Some(42.0));
}

#[test]
fn usage_reading_falls_back_to_the_daemon_and_then_to_nothing() {
    let mut app = make_app(vec![], None, Config::default());
    app.usage = None;
    app.watch_state = Some(schedule::store::WatchState {
        pid: 1,
        started_at_ms: 0,
        heartbeat_ms: 0,
        last_usage_pct: Some(99.0),
        last_usage_at_ms: Some(1234),
        reset_at_ms: Some(5678),
        usage_error: None,
        version: None,
    });

    let reading = app.usage_reading();
    assert_eq!(reading.pct, Some(99.0));
    assert_eq!(reading.sampled_at_ms, Some(1234));
    assert_eq!(reading.reset_at_ms, Some(5678));

    app.watch_state = None;
    assert_eq!(app.usage_reading().pct, None);
}

#[test]
fn an_up_to_date_watcher_is_not_restarted() {
    let mut app = make_app(vec![], None, Config::default());
    app.watch_state = Some(schedule::store::WatchState {
        pid: 1,
        started_at_ms: 0,
        heartbeat_ms: 0,
        last_usage_pct: None,
        last_usage_at_ms: None,
        reset_at_ms: None,
        usage_error: None,
        version: Some(env!("CARGO_PKG_VERSION").to_string()),
    });
    // No daemon runs under the test's tmux socket, so this exercises the
    // not-running guard as well: either way it must not report a restart.
    assert!(!app.restart_outdated_watcher());
    assert!(app.status_error.is_none());
}

#[test]
fn source_filter_hides_backends_before_other_filters() {
    let sessions = vec![
        SessionInfo {
            session_id: "claude-1".into(),
            project: "/p/claude".into(),
            project_name: "claude".into(),
            first_timestamp: 1,
            last_timestamp: 3000,
            entry_count: 1,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::ClaudeCode,
        },
        SessionInfo {
            session_id: "cursor-1".into(),
            project: "/p/cursor".into(),
            project_name: "cursor".into(),
            first_timestamp: 1,
            last_timestamp: 4000,
            entry_count: 1,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::CursorAgent,
        },
    ];
    let mut app = make_app(sessions, None, Config::default());
    app.tree_view = false;
    app.hide_empty = false;
    app.group_chains = false;

    app.source_filter = SourceFilter::Both;
    app.recompute_filter();
    assert_eq!(app.filtered_indices.len(), 2);

    app.source_filter = SourceFilter::Claude;
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![0]);

    app.source_filter = SourceFilter::Cursor;
    app.recompute_filter();
    assert_eq!(app.filtered_indices, vec![1]);
}

#[test]
fn cursor_session_never_enters_chain_map() {
    let sessions = vec![
        SessionInfo {
            session_id: "c1".into(),
            project: "/p".into(),
            project_name: "p".into(),
            first_timestamp: 1000,
            last_timestamp: 2000,
            entry_count: 1,
            has_data: true,
            name: None,
            // Even if a slug were somehow set, Cursor loaders always use None.
            slug: None,
            backend: AgentBackend::CursorAgent,
        },
        SessionInfo {
            session_id: "c2".into(),
            project: "/p".into(),
            project_name: "p".into(),
            first_timestamp: 1500,
            last_timestamp: 2500,
            entry_count: 1,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::CursorAgent,
        },
    ];
    let mut app = make_app(sessions, None, Config::default());
    app.tree_view = false;
    app.group_chains = true;
    app.recompute_filter();
    assert!(app.chain_map.is_empty());
    assert_eq!(app.filtered_indices.len(), 2);
}

#[test]
fn source_filter_key_s_cycles_and_persists() {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let _guard = crate::config::test_lock();
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("CCSM_CONFIG_DIR", dir.path());

    let mut app = make_app(make_sessions(), None, Config::default());
    assert_eq!(app.source_filter, SourceFilter::Both);

    let key = KeyEvent::new(KeyCode::Char('s'), KeyModifiers::NONE);
    app.dispatch_normal_key_with_shift(key, false);
    assert_eq!(app.source_filter, SourceFilter::Claude);
    assert_eq!(app.config.source_filter, "claude");

    app.dispatch_normal_key_with_shift(key, false);
    assert_eq!(app.source_filter, SourceFilter::Cursor);
    assert_eq!(app.config.source_filter, "cursor");

    app.dispatch_normal_key_with_shift(key, false);
    assert_eq!(app.source_filter, SourceFilter::Both);
    assert_eq!(app.config.source_filter, "both");

    std::env::remove_var("CCSM_CONFIG_DIR");
}

#[test]
fn source_filter_cycle_order() {
    assert_eq!(SourceFilter::Both.cycle(), SourceFilter::Claude);
    assert_eq!(SourceFilter::Claude.cycle(), SourceFilter::Cursor);
    assert_eq!(SourceFilter::Cursor.cycle(), SourceFilter::Both);
}

#[test]
fn deps_blocking_matrix() {
    let mut app = make_app(vec![], None, Config::default());
    app.missing_claude = false;
    app.missing_agent = false;
    app.missing_tmux = false;
    assert!(!app.deps_blocking(), "all present");

    app.missing_claude = true;
    assert!(!app.deps_blocking(), "claude-only missing is soft");

    app.missing_claude = false;
    app.missing_agent = true;
    assert!(!app.deps_blocking(), "agent-only missing is soft");

    app.missing_claude = true;
    app.missing_agent = true;
    assert!(app.deps_blocking(), "both agents missing blocks");

    app.missing_claude = false;
    app.missing_agent = false;
    app.missing_tmux = true;
    assert!(app.deps_blocking(), "tmux missing always blocks");
}

#[test]
fn ensure_backend_available_sets_status_error() {
    let mut app = make_app(vec![], None, Config::default());
    app.config.claude_path = Some("/nonexistent/ccsm-no-claude".into());
    app.config.agent_path = Some("/nonexistent/ccsm-no-agent".into());
    assert!(!app.ensure_backend_available(AgentBackend::ClaudeCode));
    assert!(app.status_error.as_deref().unwrap().contains("claude"));
    app.status_error = None;
    assert!(!app.ensure_backend_available(AgentBackend::CursorAgent));
    assert!(app.status_error.as_deref().unwrap().contains("agent"));
}

#[test]
fn rename_refuses_cursor_history_rows() {
    let sessions = vec![SessionInfo {
        session_id: "cursor-1".into(),
        project: "/p".into(),
        project_name: "p".into(),
        first_timestamp: 1,
        last_timestamp: 2,
        entry_count: 2,
        has_data: true,
        name: None,
        slug: None,
        backend: AgentBackend::CursorAgent,
    }];
    let mut app = make_app(sessions, None, Config::default());
    app.tree_view = false;
    app.recompute_filter();
    app.selected = 0;
    app.dispatch_normal_key_with_shift(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Char('r'),
            crossterm::event::KeyModifiers::NONE,
        ),
        false,
    );
    assert_eq!(app.mode, AppMode::Normal);
    assert!(
        app.status_error
            .as_deref()
            .unwrap()
            .contains("/rename"),
        "{:?}",
        app.status_error
    );
}

#[test]
fn job_form_refuses_cursor_rows() {
    let sessions = vec![SessionInfo {
        session_id: "cursor-1".into(),
        project: "/p".into(),
        project_name: "p".into(),
        first_timestamp: 1,
        last_timestamp: 2,
        entry_count: 2,
        has_data: true,
        name: None,
        slug: None,
        backend: AgentBackend::CursorAgent,
    }];
    let mut app = make_app(sessions, None, Config::default());
    app.tree_view = false;
    app.recompute_filter();
    app.selected = 0;
    app.job_form_from_selection();
    assert_ne!(app.mode, AppMode::JobForm);
    assert!(
        app.status_error
            .as_deref()
            .unwrap()
            .contains("Claude-only"),
        "{:?}",
        app.status_error
    );
}

#[test]
fn naming_backend_follows_source_filter_and_cycles_when_both() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.source_filter = SourceFilter::Cursor;
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    assert_eq!(app.naming_backend, AgentBackend::CursorAgent);

    app.mode = AppMode::Normal;
    app.source_filter = SourceFilter::Both;
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    // make_sessions are all Claude, so Both falls back to that last-used backend.
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);
    app.cycle_naming_backend();
    assert_eq!(app.naming_backend, AgentBackend::CursorAgent);
    app.cycle_naming_backend();
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);
}

#[test]
fn naming_backend_defaults_to_last_used_in_directory() {
    let sessions = vec![
        SessionInfo {
            session_id: "old-claude".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1000,
            last_timestamp: 2000,
            entry_count: 5,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::ClaudeCode,
        },
        SessionInfo {
            session_id: "newer-cursor".into(),
            project: "/Users/sane/Dev/alpha".into(),
            project_name: "alpha".into(),
            first_timestamp: 1500,
            last_timestamp: 5000,
            entry_count: 3,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::CursorAgent,
        },
        SessionInfo {
            session_id: "other-dir".into(),
            project: "/Users/sane/Dev/beta".into(),
            project_name: "beta".into(),
            first_timestamp: 1000,
            last_timestamp: 9000,
            entry_count: 1,
            has_data: true,
            name: None,
            slug: None,
            backend: AgentBackend::ClaudeCode,
        },
    ];
    let mut app = make_app(sessions, None, Config::default());
    app.source_filter = SourceFilter::Both;
    // Select the alpha project (most recent overall is beta, so find alpha).
    app.selected = app
        .tree_rows
        .iter()
        .position(|r| matches!(r, TreeRow::Header { project, .. } if project == "/Users/sane/Dev/alpha"))
        .expect("alpha header");
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    assert_eq!(app.naming_backend, AgentBackend::CursorAgent);

    // Claude-only filter still forces Claude even when last-used was Cursor.
    app.mode = AppMode::Normal;
    app.source_filter = SourceFilter::Claude;
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);
}

#[test]
fn naming_backend_falls_back_to_claude_for_unknown_directory() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.source_filter = SourceFilter::Both;
    assert_eq!(
        app.default_naming_backend("/tmp/brand-new-project"),
        AgentBackend::ClaudeCode
    );
}

#[test]
fn naming_focus_down_up_and_arrows_cycle_switchers() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.source_filter = SourceFilter::Both;
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    assert_eq!(app.naming_focus, NamingFocus::Name);
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);

    let down = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    );
    let up = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Up,
        crossterm::event::KeyModifiers::NONE,
    );
    let right = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Right,
        crossterm::event::KeyModifiers::NONE,
    );
    let left = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Left,
        crossterm::event::KeyModifiers::NONE,
    );
    let a = crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('a'),
        crossterm::event::KeyModifiers::NONE,
    );

    // `a` always types into the name while Name is focused.
    app.handle_naming_event(a);
    assert_eq!(app.naming_input.value(), "a");
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);

    app.handle_naming_event(down);
    assert_eq!(app.naming_focus, NamingFocus::Agent);
    app.handle_naming_event(right);
    assert_eq!(app.naming_backend, AgentBackend::CursorAgent);
    app.handle_naming_event(left);
    assert_eq!(app.naming_backend, AgentBackend::ClaudeCode);

    app.handle_naming_event(down);
    assert_eq!(app.naming_focus, NamingFocus::Type);
    assert_eq!(app.naming_mode, NewSessionMode::Plain);
    app.handle_naming_event(right);
    assert_eq!(app.naming_mode, NewSessionMode::Dangerous);

    // Typing on Type does not edit the name.
    app.handle_naming_event(a);
    assert_eq!(app.naming_input.value(), "a");

    app.handle_naming_event(up);
    assert_eq!(app.naming_focus, NamingFocus::Agent);
    app.handle_naming_event(up);
    assert_eq!(app.naming_focus, NamingFocus::Name);
}

#[test]
fn naming_focus_skips_agent_when_filter_is_not_both() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.source_filter = SourceFilter::Claude;
    assert!(app.open_naming_popup(NewSessionMode::Plain));
    app.handle_naming_event(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Down,
        crossterm::event::KeyModifiers::NONE,
    ));
    assert_eq!(app.naming_focus, NamingFocus::Type);
}

#[test]
fn naming_carries_backend_into_launch_request() {
    let mut app = make_app(make_sessions(), None, Config::default());
    app.source_filter = SourceFilter::Both;
    app.naming_backend = AgentBackend::CursorAgent;
    confirm_naming(&mut app, NewSessionMode::Plain);
    match &app.launch_session {
        Some(LaunchRequest::NewLive { backend, .. }) => {
            assert_eq!(*backend, AgentBackend::CursorAgent);
        }
        other => panic!("expected NewLive, got {other:?}"),
    }
}

#[test]
fn enter_on_cursor_row_resumes_with_cursor_backend() {
    let sessions = vec![SessionInfo {
        session_id: "cursor-resume-1".into(),
        project: "/p".into(),
        project_name: "p".into(),
        first_timestamp: 1,
        last_timestamp: 2,
        entry_count: 2,
        has_data: true,
        name: None,
        slug: None,
        backend: AgentBackend::CursorAgent,
    }];
    let mut app = make_app(sessions, None, Config::default());
    // Point agent at a real binary so ensure_backend_available does not block.
    app.config.agent_path = Some("/bin/sh".into());
    app.tree_view = false;
    app.recompute_filter();
    app.selected = 0;
    app.dispatch_normal_key_with_shift(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::NONE,
        ),
        false,
    );
    match &app.launch_session {
        Some(LaunchRequest::Resume {
            session_id,
            cwd,
            backend,
        }) => {
            assert_eq!(session_id, "cursor-resume-1");
            assert_eq!(cwd, "/p");
            assert_eq!(*backend, AgentBackend::CursorAgent);
        }
        other => panic!("expected Resume with CursorAgent, got {other:?}"),
    }
}

#[test]
fn shift_enter_on_cursor_row_opens_direct_with_cursor_backend() {
    let sessions = vec![SessionInfo {
        session_id: "cursor-direct-1".into(),
        project: "/p".into(),
        project_name: "p".into(),
        first_timestamp: 1,
        last_timestamp: 2,
        entry_count: 2,
        has_data: true,
        name: None,
        slug: None,
        backend: AgentBackend::CursorAgent,
    }];
    let mut app = make_app(sessions, None, Config::default());
    app.config.agent_path = Some("/bin/sh".into());
    app.tree_view = false;
    app.recompute_filter();
    app.selected = 0;
    app.dispatch_normal_key_with_shift(
        crossterm::event::KeyEvent::new(
            crossterm::event::KeyCode::Enter,
            crossterm::event::KeyModifiers::SHIFT,
        ),
        true,
    );
    match &app.launch_session {
        Some(LaunchRequest::Direct {
            session_id,
            cwd,
            backend,
        }) => {
            assert_eq!(session_id, "cursor-direct-1");
            assert_eq!(cwd, "/p");
            assert_eq!(*backend, AgentBackend::CursorAgent);
        }
        other => panic!("expected Direct with CursorAgent, got {other:?}"),
    }
}

#[test]
fn apply_session_names_fills_cursor_title_and_entry_count() {
    let sessions = vec![SessionInfo {
        session_id: "cursor-1".into(),
        project: "/p".into(),
        project_name: "p".into(),
        first_timestamp: 1,
        last_timestamp: 2,
        entry_count: 0,
        has_data: true,
        name: None,
        slug: None,
        backend: AgentBackend::CursorAgent,
    }];
    let mut app = make_app(sessions, None, Config::default());
    let mut updates = std::collections::HashMap::new();
    updates.insert(
        "cursor-1".into(),
        crate::app::SessionMetaUpdate {
            name: Some("Fixture Chat".into()),
            entry_count: Some(4),
        },
    );
    app.apply_session_names(updates);
    assert_eq!(app.sessions[0].name.as_deref(), Some("Fixture Chat"));
    assert_eq!(app.sessions[0].entry_count, 4);
}
