use crate::data::{self, PreviewMessage, SessionInfo};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::collections::{HashMap, HashSet};
use std::process::Command;

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Header {
        project: String,
        project_name: String,
        session_count: usize,
    },
    Session {
        session_index: usize,
    },
}

pub struct App {
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
    pub preview_cache: HashMap<String, Vec<PreviewMessage>>,
    pub preview_scroll: u16,
    pub should_quit: bool,
    pub launch_session: Option<(String, String)>, // (session_id, project_path)
    pub filter_text: String,
    pub filter_active: bool,
    pub filtered_indices: Vec<usize>,
    pub filter_path: Option<String>,
    pub tree_view: bool,
    pub tree_rows: Vec<TreeRow>,
    pub collapsed: HashSet<String>,
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>, filter_path: Option<String>) -> Self {
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();
        let mut app = Self {
            sessions,
            selected: 0,
            preview_cache: HashMap::new(),
            preview_scroll: u16::MAX,
            should_quit: false,
            launch_session: None,
            filter_text: String::new(),
            filter_active: false,
            filtered_indices,
            filter_path,
            tree_view: true,
            tree_rows: Vec::new(),
            collapsed: HashSet::new(),
        };
        app.init_tree();
        app
    }

    fn init_tree(&mut self) {
        for session in &self.sessions {
            self.collapsed.insert(session.project.clone());
        }
        self.recompute_tree();
    }

    fn recompute_filter(&mut self) {
        let query = self.filter_text.to_lowercase();
        if query.is_empty() {
            self.filtered_indices = (0..self.sessions.len()).collect();
        } else {
            self.filtered_indices = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.project_name.to_lowercase().contains(&query)
                        || s.project.to_lowercase().contains(&query)
                })
                .map(|(i, _)| i)
                .collect();
        }
        if self.tree_view {
            self.recompute_tree();
        }
        if self.selected >= self.visible_item_count() {
            self.selected = self.visible_item_count().saturating_sub(1);
        }
    }

    fn recompute_tree(&mut self) {
        // Group filtered sessions by project, ordered by most-recent session in each group
        let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new(); // (project, project_name, indices)
        let mut group_map: HashMap<String, usize> = HashMap::new(); // project -> index in groups

        for &idx in &self.filtered_indices {
            let session = &self.sessions[idx];
            if let Some(&group_idx) = group_map.get(&session.project) {
                groups[group_idx].2.push(idx);
            } else {
                group_map.insert(session.project.clone(), groups.len());
                groups.push((
                    session.project.clone(),
                    session.project_name.clone(),
                    vec![idx],
                ));
            }
        }

        // Sort groups by most-recent session (highest last_timestamp)
        groups.sort_by(|a, b| {
            let max_a = a.2.iter().map(|&i| self.sessions[i].last_timestamp).max().unwrap_or(0);
            let max_b = b.2.iter().map(|&i| self.sessions[i].last_timestamp).max().unwrap_or(0);
            max_b.cmp(&max_a)
        });

        self.tree_rows.clear();
        for (project, project_name, indices) in groups {
            let is_collapsed = self.collapsed.contains(&project);
            self.tree_rows.push(TreeRow::Header {
                project: project.clone(),
                project_name,
                session_count: indices.len(),
            });
            if !is_collapsed {
                for idx in indices {
                    self.tree_rows.push(TreeRow::Session { session_index: idx });
                }
            }
        }
    }

    pub fn visible_item_count(&self) -> usize {
        if self.tree_view {
            self.tree_rows.len()
        } else {
            self.filtered_indices.len()
        }
    }

    /// Get the raw session index for the currently selected item.
    /// Returns None for headers in tree mode or when no item is selected.
    pub fn selected_session_index(&self) -> Option<usize> {
        if self.tree_view {
            match self.tree_rows.get(self.selected) {
                Some(TreeRow::Session { session_index }) => Some(*session_index),
                _ => None,
            }
        } else {
            self.filtered_indices.get(self.selected).copied()
        }
    }

    pub fn current_preview(&mut self) -> &[PreviewMessage] {
        let idx = match self.selected_session_index() {
            Some(i) => i,
            None => return &[],
        };
        let session = &self.sessions[idx];
        let key = session.session_id.clone();
        let project = session.project.clone();

        if !self.preview_cache.contains_key(&key) {
            let preview = data::load_preview(&project, &key);
            self.preview_cache.insert(key.clone(), preview);
        }

        &self.preview_cache[&key]
    }

    pub fn handle_event(&mut self) -> anyhow::Result<()> {
        if let Event::Key(key) = event::read()? {
            if self.filter_active {
                match key.code {
                    KeyCode::Esc => {
                        self.filter_active = false;
                        self.filter_text.clear();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                    }
                    KeyCode::Enter => {
                        self.filter_active = false;
                    }
                    KeyCode::Backspace => {
                        self.filter_text.pop();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                    }
                    KeyCode::Char(c) => {
                        self.filter_text.push(c);
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                    }
                    KeyCode::Down => {
                        let count = self.visible_item_count();
                        if count > 0 {
                            self.selected =
                                (self.selected + 1).min(count - 1);
                            self.preview_scroll = u16::MAX;
                        }
                    }
                    KeyCode::Up => {
                        self.selected = self.selected.saturating_sub(1);
                        self.preview_scroll = u16::MAX;
                    }
                    _ => {}
                }
                return Ok(());
            }

            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('/'), _) => {
                    self.filter_active = true;
                }
                (KeyCode::Tab, _) => {
                    self.tree_view = !self.tree_view;
                    if self.tree_view {
                        self.recompute_tree();
                    }
                    self.selected = 0;
                    self.preview_scroll = u16::MAX;
                }
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    let count = self.visible_item_count();
                    if count > 0 {
                        self.selected =
                            (self.selected + 1).min(count - 1);
                        self.preview_scroll = u16::MAX;
                    }
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    self.selected = self.selected.saturating_sub(1);
                    self.preview_scroll = u16::MAX;
                }
                (KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                (KeyCode::Enter, _) => {
                    if self.tree_view {
                        match self.tree_rows.get(self.selected).cloned() {
                            Some(TreeRow::Header { project, .. }) => {
                                if self.collapsed.contains(&project) {
                                    self.collapsed.remove(&project);
                                } else {
                                    self.collapsed.insert(project);
                                }
                                self.recompute_tree();
                            }
                            Some(TreeRow::Session { session_index }) => {
                                let session = &self.sessions[session_index];
                                self.launch_session =
                                    Some((session.session_id.clone(), session.project.clone()));
                            }
                            None => {}
                        }
                    } else if let Some(idx) = self.selected_session_index() {
                        let session = &self.sessions[idx];
                        self.launch_session =
                            Some((session.session_id.clone(), session.project.clone()));
                    }
                }
                (KeyCode::Right, _) | (KeyCode::Char('l'), KeyModifiers::NONE)
                    if self.tree_view =>
                {
                    if let Some(TreeRow::Header { project, .. }) =
                        self.tree_rows.get(self.selected).cloned()
                    {
                        if self.collapsed.contains(&project) {
                            self.collapsed.remove(&project);
                            self.recompute_tree();
                        }
                    }
                }
                (KeyCode::Left, _) | (KeyCode::Char('h'), KeyModifiers::NONE)
                    if self.tree_view =>
                {
                    match self.tree_rows.get(self.selected).cloned() {
                        Some(TreeRow::Header { project, .. }) => {
                            if !self.collapsed.contains(&project) {
                                self.collapsed.insert(project);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::Session { .. }) => {
                            // Find parent header and collapse it
                            for i in (0..self.selected).rev() {
                                if let Some(TreeRow::Header { project, .. }) =
                                    self.tree_rows.get(i).cloned()
                                {
                                    self.collapsed.insert(project);
                                    self.recompute_tree();
                                    self.selected = i;
                                    self.preview_scroll = u16::MAX;
                                    break;
                                }
                            }
                        }
                        None => {}
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub fn launch_claude(session_id: &str, cwd: &str) -> anyhow::Result<()> {
        let cwd_path = std::path::Path::new(cwd);
        let dir = if cwd_path.exists() { cwd } else { "." };

        let status = Command::new("claude")
            .arg("--resume")
            .arg(session_id)
            .current_dir(dir)
            .status()?;

        if !status.success() {
            anyhow::bail!("claude exited with status: {}", status);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_sessions() -> Vec<SessionInfo> {
        vec![
            SessionInfo {
                session_id: "s1".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 1000,
                last_timestamp: 2000,
                entry_count: 5,
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/gamma".into(),
                project_name: "gamma".into(),
                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
            },
        ]
    }

    #[test]
    fn test_new_app_initializes_all_indices() {
        let app = App::new(make_sessions(), None);
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
        assert_eq!(app.selected, 0);
        assert!(!app.filter_active);
        assert!(app.filter_text.is_empty());
        assert!(app.tree_view);
    }

    #[test]
    fn test_new_app_starts_all_collapsed() {
        let app = App::new(make_sessions_with_shared_projects(), None);
        // All groups collapsed: only headers visible
        assert!(app.tree_rows.iter().all(|r| matches!(r, TreeRow::Header { .. })));
        assert_eq!(app.tree_rows.len(), 2); // beta header + alpha header
    }

    #[test]
    fn test_right_arrow_expands_collapsed_header() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        // All collapsed, selected=0 is first header (beta)
        app.selected = 0;
        let project = match &app.tree_rows[0] {
            TreeRow::Header { project, .. } => project.clone(),
            _ => panic!("expected header"),
        };
        assert!(app.collapsed.contains(&project));

        // Simulate expand
        app.collapsed.remove(&project);
        app.recompute_tree();

        // beta now expanded: header + 2 sessions
        assert!(!app.collapsed.contains(&project));
        assert!(matches!(&app.tree_rows[1], TreeRow::Session { .. }));
    }

    #[test]
    fn test_left_arrow_collapses_expanded_header() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
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
        let mut app = App::new(make_sessions(), None);
        app.filter_text = "beta".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![1]);
    }

    #[test]
    fn test_filter_case_insensitive() {
        let mut app = App::new(make_sessions(), None);
        app.filter_text = "ALPHA".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn test_filter_matches_path() {
        let mut app = App::new(make_sessions(), None);
        app.filter_text = "/Dev/gamma".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![2]);
    }

    #[test]
    fn test_filter_no_match() {
        let mut app = App::new(make_sessions(), None);
        app.filter_text = "nonexistent".into();
        app.recompute_filter();
        assert!(app.filtered_indices.is_empty());
        assert_eq!(app.selected_session_index(), None);
    }

    #[test]
    fn test_clear_filter_restores_all() {
        let mut app = App::new(make_sessions(), None);
        app.filter_text = "beta".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices.len(), 1);

        app.filter_text.clear();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_selected_clamps_on_filter() {
        let mut app = App::new(make_sessions(), None);
        app.tree_view = false;
        app.selected = 2;
        app.filter_text = "alpha".into();
        app.recompute_filter();
        // selected was 2 but only 1 match, should clamp to 0
        assert_eq!(app.selected, 0);
        assert_eq!(app.selected_session_index(), Some(0));
    }

    #[test]
    fn test_selected_session_index() {
        let mut app = App::new(make_sessions(), None);
        app.tree_view = false;
        app.filter_text = "amma".into(); // matches only gamma
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![2]);
        app.selected = 0;
        assert_eq!(app.selected_session_index(), Some(2));
    }

    #[test]
    fn test_filter_path_stored() {
        let app = App::new(make_sessions(), Some("/Users/sane/Dev".into()));
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
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
            },
            SessionInfo {
                session_id: "s4".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 2000,
                last_timestamp: 6000,
                entry_count: 2,
            },
        ]
    }

    #[test]
    fn test_tree_grouping() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        // Expand all groups to test full tree structure
        app.collapsed.clear();
        app.recompute_tree();

        // beta group first (s4 has last_timestamp=6000), then alpha (s1 has 5000)
        assert_eq!(app.tree_rows.len(), 6); // 2 headers + 4 sessions
        assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, session_count, .. } if project_name == "beta" && *session_count == 2));
        assert!(matches!(&app.tree_rows[1], TreeRow::Session { session_index: 1 }));
        assert!(matches!(&app.tree_rows[2], TreeRow::Session { session_index: 3 }));
        assert!(matches!(&app.tree_rows[3], TreeRow::Header { project_name, session_count, .. } if project_name == "alpha" && *session_count == 2));
        assert!(matches!(&app.tree_rows[4], TreeRow::Session { session_index: 0 }));
        assert!(matches!(&app.tree_rows[5], TreeRow::Session { session_index: 2 }));
    }

    #[test]
    fn test_tree_collapse_expand() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        // Start: all collapsed, only headers
        assert_eq!(app.tree_rows.len(), 2);

        // Expand all
        app.collapsed.clear();
        app.recompute_tree();
        assert_eq!(app.tree_rows.len(), 6);

        // Collapse beta
        app.collapsed.insert("/Users/sane/Dev/beta".into());
        app.recompute_tree();
        assert_eq!(app.tree_rows.len(), 4); // beta header + alpha header + 2 alpha sessions
        assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, .. } if project_name == "beta"));
        assert!(matches!(&app.tree_rows[1], TreeRow::Header { project_name, .. } if project_name == "alpha"));

        // Expand beta
        app.collapsed.remove("/Users/sane/Dev/beta");
        app.recompute_tree();
        assert_eq!(app.tree_rows.len(), 6);
    }

    #[test]
    fn test_selected_session_index_returns_none_for_header() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        app.selected = 0; // header row (all collapsed)
        assert_eq!(app.selected_session_index(), None);
    }

    #[test]
    fn test_selected_session_index_returns_some_for_session_in_tree() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        app.collapsed.clear();
        app.recompute_tree();
        app.selected = 1; // first session under beta
        assert_eq!(app.selected_session_index(), Some(1));
    }

    #[test]
    fn test_visible_item_count_flat_vs_tree() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        // Default is tree view, all collapsed: 2 headers
        assert_eq!(app.visible_item_count(), 2);

        // Expand all
        app.collapsed.clear();
        app.recompute_tree();
        assert_eq!(app.visible_item_count(), 6); // 2 headers + 4 sessions

        // Switch to flat
        app.tree_view = false;
        assert_eq!(app.visible_item_count(), 4); // 4 sessions
    }

    #[test]
    fn test_tree_with_filter() {
        let mut app = App::new(make_sessions_with_shared_projects(), None);
        app.filter_text = "alpha".into();
        app.recompute_filter();
        // Only alpha sessions should appear, but collapsed
        assert_eq!(app.tree_rows.len(), 1); // 1 header (collapsed)
        assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, .. } if project_name == "alpha"));

        // Expand to see sessions
        app.collapsed.remove("/Users/sane/Dev/alpha");
        app.recompute_filter();
        assert_eq!(app.tree_rows.len(), 3); // 1 header + 2 sessions
    }
}
