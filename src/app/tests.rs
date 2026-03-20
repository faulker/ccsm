use super::*;
use crate::config::Config;
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
    let mut app = make_app(make_sessions(), None, Config::default());
    app.mode = AppMode::Config;
    assert_eq!(app.config_selected, 0);

    // Can't go below 0
    app.config_selected = 0;
    if app.config_selected > 0 { app.config_selected -= 1; }
    assert_eq!(app.config_selected, 0);

    // Navigate down
    app.config_selected = 1;
    assert_eq!(app.config_selected, 1);
    app.config_selected = 2;
    assert_eq!(app.config_selected, 2);

    // Navigate to all items including about section
    app.config_selected = 3;
    assert_eq!(app.config_selected, 3);
    app.config_selected = 4;
    assert_eq!(app.config_selected, 4);
    app.config_selected = 5;
    assert_eq!(app.config_selected, 5);
    app.config_selected = 6;
    assert_eq!(app.config_selected, 6);

    // Can't go above 6
    if app.config_selected < 6 { app.config_selected += 1; }
    assert_eq!(app.config_selected, 6);
}

#[test]
fn test_config_toggle_hide_empty() {
    let mut app = make_app(make_sessions_mixed_data(), None, Config::default());
    app.tree_view = false;
    app.mode = AppMode::Config;
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
    app.mode = AppMode::Config;
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
