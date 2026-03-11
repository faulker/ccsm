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
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>) -> Self {
        Self {
            sessions,
            selected: 0,
            preview_cache: HashMap::new(),
            preview_scroll: 0,
            should_quit: false,
            launch_session: None,
        }
    }

    pub fn current_preview(&mut self) -> &[PreviewMessage] {
        if self.sessions.is_empty() {
            return &[];
        }
        let session = &self.sessions[self.selected];
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
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) | (KeyCode::Esc, _) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, _) => {
                    if !self.sessions.is_empty() {
                        self.selected = (self.selected + 1).min(self.sessions.len() - 1);
                        self.preview_scroll = 0;
                    }
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, _) => {
                    if !self.sessions.is_empty() {
                        self.selected = self.selected.saturating_sub(1);
                        self.preview_scroll = 0;
                    }
                }
                (KeyCode::Char('J'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                (KeyCode::Char('K'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                (KeyCode::Enter, _) => {
                    if !self.sessions.is_empty() {
                        let session = &self.sessions[self.selected];
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

        Command::new("claude")
            .arg("--resume")
            .arg(session_id)
            .current_dir(dir)
            .status()?;

        Ok(())
    }
}
