use crate::data::{self, PreviewMessage, SessionInfo};
use crossterm::event::{self, Event, KeyCode, KeyModifiers};
use std::collections::HashMap;
use std::process::Command;

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
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>, filter_path: Option<String>) -> Self {
        let filtered_indices: Vec<usize> = (0..sessions.len()).collect();
        Self {
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
        }
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
        if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len().saturating_sub(1);
        }
    }

    /// Get the raw session index for the currently selected filtered item
    pub fn selected_session_index(&self) -> Option<usize> {
        self.filtered_indices.get(self.selected).copied()
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
                        if !self.filtered_indices.is_empty() {
                            self.selected =
                                (self.selected + 1).min(self.filtered_indices.len() - 1);
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
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    if !self.filtered_indices.is_empty() {
                        self.selected =
                            (self.selected + 1).min(self.filtered_indices.len() - 1);
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
                    if let Some(idx) = self.selected_session_index() {
                        let session = &self.sessions[idx];
                        self.launch_session =
                            Some((session.session_id.clone(), session.project.clone()));
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
}
