use crate::config::{Config, DisplayMode};
use crate::data::{self, PreviewMessage, SessionInfo, SessionMeta};
use crate::update;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, ModifierKeyCode};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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

#[derive(Debug, Clone)]
pub enum LaunchRequest {
    Resume { session_id: String, cwd: String },
    New { cwd: String },
}

#[derive(Debug, Clone, PartialEq)]
pub enum AppMode {
    Normal,
    DirBrowser,
    Renaming,
    UpdatePrompt,
}

#[derive(Debug, Clone)]
pub struct DirEntry {
    pub name: String,
    pub is_dir: bool,
}

#[derive(Debug, Clone)]
pub struct DirBrowser {
    pub current_dir: PathBuf,
    pub entries: Vec<DirEntry>,
    pub selected: usize,
    pub input_text: String,
    pub input_active: bool,
}

impl DirBrowser {
    pub fn new(start_dir: PathBuf) -> Self {
        let mut browser = Self {
            current_dir: start_dir,
            entries: Vec::new(),
            selected: 0,
            input_text: String::new(),
            input_active: false,
        };
        browser.refresh();
        browser
    }

    pub fn refresh(&mut self) {
        self.entries.clear();
        self.entries.push(DirEntry {
            name: "..".to_string(),
            is_dir: true,
        });

        if let Ok(read_dir) = std::fs::read_dir(&self.current_dir) {
            let mut dirs: Vec<String> = read_dir
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().map(|ft| ft.is_dir()).unwrap_or(false))
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect();
            dirs.sort_by(|a, b| a.to_lowercase().cmp(&b.to_lowercase()));
            for name in dirs {
                self.entries.push(DirEntry { name, is_dir: true });
            }
        }

        if self.selected >= self.entries.len() {
            self.selected = self.entries.len().saturating_sub(1);
        }
    }

    pub fn enter_selected(&mut self) {
        if let Some(entry) = self.entries.get(self.selected) {
            if entry.name == ".." {
                self.go_up();
            } else if entry.is_dir {
                self.current_dir = self.current_dir.join(&entry.name);
                self.selected = 0;
                self.refresh();
            }
        }
    }

    pub fn go_up(&mut self) {
        if let Some(parent) = self.current_dir.parent() {
            self.current_dir = parent.to_path_buf();
            self.selected = 0;
            self.refresh();
        }
    }

    pub fn apply_input(&mut self) {
        let path = PathBuf::from(&self.input_text);
        if path.is_dir() {
            self.current_dir = path;
            self.selected = 0;
            self.refresh();
        }
        self.input_text.clear();
        self.input_active = false;
    }
}

pub struct App {
    pub sessions: Vec<SessionInfo>,
    pub selected: usize,
    pub preview_cache: HashMap<String, (SessionMeta, Vec<PreviewMessage>)>,
    pub preview_scroll: u16,
    pub should_quit: bool,
    pub launch_session: Option<LaunchRequest>,
    pub filter_text: String,
    pub filter_active: bool,
    pub filtered_indices: Vec<usize>,
    pub filter_path: Option<String>,
    pub tree_view: bool,
    pub display_mode: DisplayMode,
    pub tree_rows: Vec<TreeRow>,
    pub collapsed: HashSet<String>,
    pub hide_empty: bool,
    pub mode: AppMode,
    pub dir_browser: Option<DirBrowser>,
    pub config: Config,
    pub shift_active: bool,
    pub rename_text: String,
    pub rename_session_id: Option<String>,
    pub rename_project: Option<String>,
    pub update_status: update::UpdateStatus,
    pub perform_update: Option<update::UpdateInfo>,
    pub update_receiver: Option<std::sync::mpsc::Receiver<update::UpdateInfo>>,
    pub names_receiver: Option<std::sync::mpsc::Receiver<HashMap<String, String>>>,
}

/// Truncate a path to its last 2 components (e.g. "/Users/sane/Dev/ccsm" -> "Dev/ccsm").
fn truncate_path(path: &str) -> String {
    let parts: Vec<&str> = path.rsplitn(3, '/').collect();
    if parts.len() >= 2 {
        format!("{}/{}", parts[1], parts[0])
    } else {
        path.to_string()
    }
}

impl App {
    pub fn new(sessions: Vec<SessionInfo>, filter_path: Option<String>, config: Config) -> Self {
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
            tree_view: config.tree_view,
            display_mode: config.display_mode,
            hide_empty: config.hide_empty,
            tree_rows: Vec::new(),
            collapsed: HashSet::new(),
            mode: AppMode::Normal,
            dir_browser: None,
            config,
            shift_active: false,
            rename_text: String::new(),
            rename_session_id: None,
            rename_project: None,
            update_status: update::UpdateStatus::None,
            perform_update: None,
            update_receiver: None,
            names_receiver: None,
        };
        app.spawn_load_session_names();
        app.init_tree();
        app.recompute_filter();
        app
    }

    fn spawn_load_session_names(&mut self) {
        let sessions: Vec<(String, String)> = self
            .sessions
            .iter()
            .filter(|s| s.has_data)
            .map(|s| (s.project.clone(), s.session_id.clone()))
            .collect();

        let (tx, rx) = std::sync::mpsc::channel();
        self.names_receiver = Some(rx);

        std::thread::spawn(move || {
            let mut names = HashMap::new();
            for (project, session_id) in sessions {
                if let Some(title) = data::load_custom_title(&project, &session_id) {
                    names.insert(session_id, title);
                }
            }
            let _ = tx.send(names);
        });
    }

    pub fn apply_session_names(&mut self, names: HashMap<String, String>) {
        for session in &mut self.sessions {
            if let Some(title) = names.get(&session.session_id) {
                session.name = Some(title.clone());
            }
        }
        self.preview_cache.clear();
        self.recompute_tree();
    }

    pub fn reload_sessions(&mut self, sessions: Vec<SessionInfo>) {
        self.sessions = sessions;
        self.spawn_load_session_names();
        self.preview_cache.clear();
        self.preview_scroll = u16::MAX;
        self.recompute_filter();
        self.recompute_tree();
        if self.selected >= self.filtered_indices.len() {
            self.selected = self.filtered_indices.len().saturating_sub(1);
        }
    }

    fn save_config(&mut self) {
        self.config.tree_view = self.tree_view;
        self.config.display_mode = self.display_mode;
        self.config.hide_empty = self.hide_empty;
        self.config.save();
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
            self.filtered_indices = (0..self.sessions.len())
                .filter(|&i| !self.hide_empty || self.sessions[i].has_data)
                .collect();
        } else {
            self.filtered_indices = self
                .sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    (!self.hide_empty || s.has_data)
                        && (s.project_name.to_lowercase().contains(&query)
                            || s.project.to_lowercase().contains(&query))
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

    /// Returns the display name for a session based on the display mode.
    pub fn display_name(&self, session: &SessionInfo) -> String {
        match self.display_mode {
            DisplayMode::Name => session.project_name.clone(),
            DisplayMode::ShortDir => truncate_path(&session.project),
            DisplayMode::FullDir => session.project.clone(),
        }
    }

    fn recompute_tree(&mut self) {
        // Group filtered sessions by project
        let mut groups: Vec<(String, String, Vec<usize>)> = Vec::new(); // (group_key, display_name, indices)
        let mut group_map: HashMap<String, usize> = HashMap::new(); // group_key -> index in groups

        for &idx in &self.filtered_indices {
            let session = &self.sessions[idx];
            let display_name = match self.display_mode {
                DisplayMode::Name => session.project_name.clone(),
                DisplayMode::ShortDir => truncate_path(&session.project),
                DisplayMode::FullDir => session.project.clone(),
            };
            let group_key = session.project.clone();
            if let Some(&group_idx) = group_map.get(&group_key) {
                groups[group_idx].2.push(idx);
            } else {
                group_map.insert(group_key.clone(), groups.len());
                groups.push((group_key, display_name, vec![idx]));
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

    pub fn current_preview(&mut self) -> (&SessionMeta, &[PreviewMessage]) {
        // Static default for the empty case
        static EMPTY_META: SessionMeta = SessionMeta { cwd: None, git_branch: None, session_id: None, session_name: None };

        let idx = match self.selected_session_index() {
            Some(i) => i,
            None => return (&EMPTY_META, &[]),
        };
        let session = &self.sessions[idx];
        let key = session.session_id.clone();
        let project = session.project.clone();

        if !self.preview_cache.contains_key(&key) {
            let result = data::load_preview(&project, &key);
            self.preview_cache.insert(key.clone(), result);
        }

        let session = &self.sessions[idx];
        let (meta, messages) = self.preview_cache.get_mut(&key).unwrap();
        meta.session_id = Some(session.session_id.clone());
        meta.session_name = session.name.clone();
        (meta, messages)
    }

    /// Get the CWD for the currently selected session (or header group).
    pub fn selected_cwd(&self) -> Option<String> {
        if self.tree_view {
            match self.tree_rows.get(self.selected) {
                Some(TreeRow::Session { session_index }) => {
                    Some(self.sessions[*session_index].project.clone())
                }
                Some(TreeRow::Header { project, .. }) => Some(project.clone()),
                None => None,
            }
        } else {
            self.selected_session_index()
                .map(|idx| self.sessions[idx].project.clone())
        }
    }

    fn handle_dir_browser_event(&mut self, key: crossterm::event::KeyEvent) {
        let browser = match self.dir_browser.as_mut() {
            Some(b) => b,
            None => return,
        };

        if browser.input_active {
            match key.code {
                KeyCode::Esc => {
                    browser.input_active = false;
                    browser.input_text.clear();
                }
                KeyCode::Enter => {
                    browser.apply_input();
                }
                KeyCode::Backspace => {
                    browser.input_text.pop();
                }
                KeyCode::Char(c) => {
                    let c = if key.modifiers.contains(KeyModifiers::SHIFT) {
                        c.to_ascii_uppercase()
                    } else {
                        c
                    };
                    browser.input_text.push(c);
                }
                _ => {}
            }
            return;
        }

        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => {
                self.mode = AppMode::Normal;
                self.dir_browser = None;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                if !browser.entries.is_empty() {
                    browser.selected = (browser.selected + 1).min(browser.entries.len() - 1);
                }
            }
            KeyCode::Char('k') | KeyCode::Up => {
                browser.selected = browser.selected.saturating_sub(1);
            }
            KeyCode::Enter | KeyCode::Char('l') | KeyCode::Right => {
                browser.enter_selected();
            }
            KeyCode::Char('h') | KeyCode::Left | KeyCode::Backspace => {
                browser.go_up();
            }
            KeyCode::Char('/') => {
                browser.input_active = true;
                browser.input_text = browser.current_dir.to_string_lossy().to_string();
            }
            KeyCode::Char(' ') => {
                let cwd = if let Some(entry) = browser.entries.get(browser.selected) {
                    if entry.is_dir && entry.name != ".." {
                        browser.current_dir.join(&entry.name)
                    } else {
                        browser.current_dir.clone()
                    }
                } else {
                    browser.current_dir.clone()
                };
                let cwd = cwd.to_string_lossy().to_string();
                self.launch_session = Some(LaunchRequest::New { cwd });
                self.mode = AppMode::Normal;
                self.dir_browser = None;
            }
            _ => {}
        }
    }

    fn handle_rename_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.rename_text.clear();
                self.rename_session_id = None;
                self.rename_project = None;
            }
            KeyCode::Enter => {
                if let Some(session_id) = self.rename_session_id.take() {
                    let project = self.rename_project.take().unwrap_or_default();
                    let name = self.rename_text.trim().to_string();
                    // Write to the session JSONL (even empty to clear)
                    let _ = data::save_custom_title(&project, &session_id, &name);
                    let name_opt = if name.is_empty() { None } else { Some(name) };
                    for s in &mut self.sessions {
                        if s.session_id == session_id {
                            s.name = name_opt.clone();
                        }
                    }
                    self.preview_cache.clear();
                }
                self.rename_text.clear();
                self.mode = AppMode::Normal;
            }
            KeyCode::Backspace => {
                self.rename_text.pop();
            }
            KeyCode::Char(c) => {
                let c = if key.modifiers.contains(KeyModifiers::SHIFT) {
                    c.to_ascii_uppercase()
                } else {
                    c
                };
                self.rename_text.push(c);
            }
            _ => {}
        }
    }

    pub fn handle_event(&mut self) -> anyhow::Result<()> {
        if let Event::Key(key) = event::read()? {
            // Track shift state for UI highlighting
            match (&key.code, key.kind) {
                // Bare shift press/release — update flag and consume event
                (KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift), KeyEventKind::Press) => {
                    self.shift_active = true;
                    return Ok(());
                }
                (KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift), KeyEventKind::Release) => {
                    self.shift_active = false;
                    return Ok(());
                }
                // For all other keys, track shift from modifiers field
                _ => {
                    self.shift_active = key.modifiers.contains(KeyModifiers::SHIFT);
                }
            }

            // Only process actions on key press, not release/repeat
            if key.kind != KeyEventKind::Press {
                return Ok(());
            }

            if self.mode == AppMode::UpdatePrompt {
                match key.code {
                    KeyCode::Char('y') => {
                        if let update::UpdateStatus::Available(ref info) = self.update_status {
                            self.perform_update = Some(info.clone());
                            self.update_status = update::UpdateStatus::Downloading;
                        }
                        self.mode = AppMode::Normal;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.update_status = update::UpdateStatus::None;
                        self.mode = AppMode::Normal;
                    }
                    _ => {}
                }
                return Ok(());
            }

            if self.mode == AppMode::Renaming {
                self.handle_rename_event(key);
                return Ok(());
            }

            if self.mode == AppMode::DirBrowser {
                self.handle_dir_browser_event(key);
                return Ok(());
            }

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
                        let c = if key.modifiers.contains(KeyModifiers::SHIFT) {
                            c.to_ascii_uppercase()
                        } else {
                            c
                        };
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
                    // Cycle: tree[Name] → tree[ShortDir] → tree[FullDir]
                    //      → flat[Name] → flat[ShortDir] → flat[FullDir] → tree[Name]
                    match (self.tree_view, self.display_mode) {
                        (true, DisplayMode::Name) => {
                            self.display_mode = DisplayMode::ShortDir;
                            self.recompute_tree();
                        }
                        (true, DisplayMode::ShortDir) => {
                            self.display_mode = DisplayMode::FullDir;
                            self.recompute_tree();
                        }
                        (true, DisplayMode::FullDir) => {
                            self.tree_view = false;
                            self.display_mode = DisplayMode::Name;
                        }
                        (false, DisplayMode::Name) => {
                            self.display_mode = DisplayMode::ShortDir;
                        }
                        (false, DisplayMode::ShortDir) => {
                            self.display_mode = DisplayMode::FullDir;
                        }
                        (false, DisplayMode::FullDir) => {
                            self.tree_view = true;
                            self.display_mode = DisplayMode::Name;
                            self.recompute_tree();
                        }
                    }
                    self.selected = 0;
                    self.preview_scroll = u16::MAX;
                    self.save_config();
                }
                (KeyCode::BackTab, _) => {
                    // Reverse cycle: opposite of Tab
                    match (self.tree_view, self.display_mode) {
                        (true, DisplayMode::Name) => {
                            self.tree_view = false;
                            self.display_mode = DisplayMode::FullDir;
                        }
                        (true, DisplayMode::ShortDir) => {
                            self.display_mode = DisplayMode::Name;
                            self.recompute_tree();
                        }
                        (true, DisplayMode::FullDir) => {
                            self.display_mode = DisplayMode::ShortDir;
                            self.recompute_tree();
                        }
                        (false, DisplayMode::Name) => {
                            self.tree_view = true;
                            self.display_mode = DisplayMode::FullDir;
                            self.recompute_tree();
                        }
                        (false, DisplayMode::ShortDir) => {
                            self.display_mode = DisplayMode::Name;
                        }
                        (false, DisplayMode::FullDir) => {
                            self.display_mode = DisplayMode::ShortDir;
                        }
                    }
                    self.selected = 0;
                    self.preview_scroll = u16::MAX;
                    self.save_config();
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
                (KeyCode::Char('J' | 'j'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                (KeyCode::Char('K' | 'k'), KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                (KeyCode::Char('e'), KeyModifiers::NONE) => {
                    self.hide_empty = !self.hide_empty;
                    self.recompute_filter();
                    self.preview_scroll = u16::MAX;
                    self.save_config();
                }
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    if let Some(cwd) = self.selected_cwd() {
                        let path = std::path::Path::new(&cwd);
                        let dir = if path.exists() {
                            cwd
                        } else {
                            ".".to_string()
                        };
                        self.launch_session = Some(LaunchRequest::New { cwd: dir });
                    }
                }
                (KeyCode::Char('r'), KeyModifiers::NONE) => {
                    if let Some(idx) = self.selected_session_index() {
                        let session = &self.sessions[idx];
                        self.rename_session_id = Some(session.session_id.clone());
                        self.rename_project = Some(session.project.clone());
                        self.rename_text = session.name.clone().unwrap_or_default();
                        self.mode = AppMode::Renaming;
                    }
                }
                (KeyCode::Char('N' | 'n'), KeyModifiers::SHIFT) => {
                    let start = self
                        .selected_cwd()
                        .map(PathBuf::from)
                        .filter(|p| p.exists())
                        .unwrap_or_else(|| {
                            dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
                        });
                    self.dir_browser = Some(DirBrowser::new(start));
                    self.mode = AppMode::DirBrowser;
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
                                self.launch_session = Some(LaunchRequest::Resume {
                                    session_id: session.session_id.clone(),
                                    cwd: session.project.clone(),
                                });
                            }
                            None => {}
                        }
                    } else if let Some(idx) = self.selected_session_index() {
                        let session = &self.sessions[idx];
                        self.launch_session = Some(LaunchRequest::Resume {
                            session_id: session.session_id.clone(),
                            cwd: session.project.clone(),
                        });
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

    pub fn launch_claude_new(cwd: &str) -> anyhow::Result<()> {
        let cwd_path = std::path::Path::new(cwd);
        let dir = if cwd_path.exists() { cwd } else { "." };

        let status = Command::new("claude").current_dir(dir).status()?;

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
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),

                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/gamma".into(),
                project_name: "gamma".into(),

                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
                has_data: true, name: None,
            },
        ]
    }

    #[test]
    fn test_new_app_initializes_all_indices() {
        let app = App::new(make_sessions(), None, Config::default());
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
        assert_eq!(app.selected, 0);
        assert!(!app.filter_active);
        assert!(app.filter_text.is_empty());
        assert!(app.tree_view);
        assert!(!app.shift_active);
    }

    #[test]
    fn test_new_app_starts_all_collapsed() {
        let app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        // All groups collapsed: only headers visible
        assert!(app.tree_rows.iter().all(|r| matches!(r, TreeRow::Header { .. })));
        assert_eq!(app.tree_rows.len(), 2); // beta header + alpha header
    }

    #[test]
    fn test_right_arrow_expands_collapsed_header() {
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
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
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
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
        let mut app = App::new(make_sessions(), None, Config::default());
        app.filter_text = "beta".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![1]);
    }

    #[test]
    fn test_filter_case_insensitive() {
        let mut app = App::new(make_sessions(), None, Config::default());
        app.filter_text = "ALPHA".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn test_filter_matches_path() {
        let mut app = App::new(make_sessions(), None, Config::default());
        app.filter_text = "/Dev/gamma".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![2]);
    }

    #[test]
    fn test_filter_no_match() {
        let mut app = App::new(make_sessions(), None, Config::default());
        app.filter_text = "nonexistent".into();
        app.recompute_filter();
        assert!(app.filtered_indices.is_empty());
        assert_eq!(app.selected_session_index(), None);
    }

    #[test]
    fn test_clear_filter_restores_all() {
        let mut app = App::new(make_sessions(), None, Config::default());
        app.filter_text = "beta".into();
        app.recompute_filter();
        assert_eq!(app.filtered_indices.len(), 1);

        app.filter_text.clear();
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_selected_clamps_on_filter() {
        let mut app = App::new(make_sessions(), None, Config::default());
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
        let mut app = App::new(make_sessions(), None, Config::default());
        app.tree_view = false;
        app.filter_text = "amma".into(); // matches only gamma
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![2]);
        app.selected = 0;
        assert_eq!(app.selected_session_index(), Some(2));
    }

    #[test]
    fn test_filter_path_stored() {
        let app = App::new(make_sessions(), Some("/Users/sane/Dev".into()), Config::default());
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
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s4".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 2000,
                last_timestamp: 6000,
                entry_count: 2,
                has_data: true, name: None,
            },
        ]
    }

    #[test]
    fn test_tree_grouping() {
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        app.display_mode = DisplayMode::Name;
        app.recompute_tree();
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
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        app.display_mode = DisplayMode::Name;
        app.recompute_tree();
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
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        app.selected = 0; // header row (all collapsed)
        assert_eq!(app.selected_session_index(), None);
    }

    #[test]
    fn test_selected_session_index_returns_some_for_session_in_tree() {
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        app.collapsed.clear();
        app.recompute_tree();
        app.selected = 1; // first session under beta
        assert_eq!(app.selected_session_index(), Some(1));
    }

    #[test]
    fn test_visible_item_count_flat_vs_tree() {
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
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
        let mut app = App::new(make_sessions_with_shared_projects(), None, Config::default());
        app.display_mode = DisplayMode::Name;
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

    fn make_sessions_with_projects() -> Vec<SessionInfo> {
        vec![
            SessionInfo {
                session_id: "s1".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 1000,
                last_timestamp: 5000,
                entry_count: 5,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/alpha".into(),
                project_name: "alpha".into(),
                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s4".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 2000,
                last_timestamp: 6000,
                entry_count: 2,
                has_data: true, name: None,
            },
        ]
    }

    #[test]
    fn test_short_dir_groups_by_project() {
        let mut app = App::new(make_sessions_with_projects(), None, Config::default());
        app.display_mode = DisplayMode::ShortDir;
        app.collapsed.clear();
        app.recompute_tree();

        // 2 groups: beta (ts=6000) and alpha (ts=5000)
        let headers: Vec<_> = app.tree_rows.iter().filter(|r| matches!(r, TreeRow::Header { .. })).collect();
        assert_eq!(headers.len(), 2);

        // First group: beta (truncated)
        assert!(matches!(&app.tree_rows[0], TreeRow::Header { project_name, session_count, .. }
            if project_name == "Dev/beta" && *session_count == 1));

        // Second group: alpha (3 sessions, truncated)
        assert!(matches!(&app.tree_rows[2], TreeRow::Header { project_name, session_count, .. }
            if project_name == "Dev/alpha" && *session_count == 3));
    }

    #[test]
    fn test_display_mode_toggle_changes_display_name() {
        let mut app = App::new(make_sessions_with_projects(), None, Config::default());
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
        let app = App::new(make_sessions_with_projects(), None, Config {
            display_mode: DisplayMode::ShortDir,
            ..Config::default()
        });
        assert_eq!(app.display_name(&app.sessions[0]), "Dev/alpha");
        assert_eq!(app.display_name(&app.sessions[3]), "Dev/beta");
    }

    #[test]
    fn test_display_name_project_name() {
        let app = App::new(make_sessions_with_projects(), None, Config::default());
        assert_eq!(app.display_name(&app.sessions[0]), "alpha");
        assert_eq!(app.display_name(&app.sessions[3]), "beta");
    }

    #[test]
    fn test_display_name_full_dir() {
        let app = App::new(make_sessions_with_projects(), None, Config {
            display_mode: DisplayMode::FullDir,
            ..Config::default()
        });
        assert_eq!(app.display_name(&app.sessions[0]), "/Users/sane/Dev/alpha");
        assert_eq!(app.display_name(&app.sessions[3]), "/Users/sane/Dev/beta");
    }

    #[test]
    fn test_app_default_mode_is_normal() {
        let app = App::new(make_sessions(), None, Config::default());
        assert_eq!(app.mode, AppMode::Normal);
        assert!(app.dir_browser.is_none());
    }

    #[test]
    fn test_dir_browser_new_reads_entries() {
        let browser = DirBrowser::new(std::env::temp_dir());
        // Should always have at least ".." entry
        assert!(!browser.entries.is_empty());
        assert_eq!(browser.entries[0].name, "..");
        assert_eq!(browser.selected, 0);
        assert!(!browser.input_active);
    }

    #[test]
    fn test_dir_browser_go_up() {
        let start = std::env::temp_dir();
        let mut browser = DirBrowser::new(start.clone());
        let original = browser.current_dir.clone();
        browser.go_up();
        if let Some(parent) = original.parent() {
            assert_eq!(browser.current_dir, parent.to_path_buf());
        }
    }

    #[test]
    fn test_dir_browser_apply_input_valid() {
        let mut browser = DirBrowser::new(std::env::temp_dir());
        browser.input_active = true;
        browser.input_text = "/tmp".to_string();
        browser.apply_input();
        assert!(!browser.input_active);
        assert!(browser.input_text.is_empty());
        // /tmp should resolve to the canonical temp dir
        assert!(browser.current_dir.exists());
    }

    #[test]
    fn test_dir_browser_apply_input_invalid() {
        let start = std::env::temp_dir();
        let mut browser = DirBrowser::new(start.clone());
        browser.input_active = true;
        browser.input_text = "/nonexistent_dir_xyz_123".to_string();
        browser.apply_input();
        // Should stay at original dir since path is invalid
        assert_eq!(browser.current_dir, start);
        assert!(!browser.input_active);
    }

    #[test]
    fn test_dir_browser_shows_hidden_dirs() {
        let browser = DirBrowser::new(dirs::home_dir().unwrap_or_else(|| PathBuf::from("/")));
        // Should include dot directories (e.g. .config, .local)
        let has_hidden = browser.entries.iter().any(|e| e.name != ".." && e.name.starts_with('.'));
        // Home dir almost certainly has hidden dirs
        assert!(has_hidden, "expected hidden dirs in home directory");
    }

    #[test]
    fn test_space_key_resolves_selected_directory() {
        let tmp = std::env::temp_dir();
        let mut browser = DirBrowser::new(tmp.clone());
        // Find a real subdirectory entry (not "..")
        let dir_idx = browser.entries.iter().position(|e| e.is_dir && e.name != "..");
        if let Some(idx) = dir_idx {
            browser.selected = idx;
            let entry = &browser.entries[idx];
            let expected = tmp.join(&entry.name);
            // Simulate what the space-key handler does
            let cwd = if let Some(entry) = browser.entries.get(browser.selected) {
                if entry.is_dir && entry.name != ".." {
                    browser.current_dir.join(&entry.name)
                } else {
                    browser.current_dir.clone()
                }
            } else {
                browser.current_dir.clone()
            };
            assert_eq!(cwd, expected);
        }
    }

    #[test]
    fn test_space_key_on_dotdot_uses_current_dir() {
        let tmp = std::env::temp_dir();
        let browser = DirBrowser::new(tmp.clone());
        // First entry is always ".."
        assert_eq!(browser.entries[0].name, "..");
        let cwd = if let Some(entry) = browser.entries.get(0) {
            if entry.is_dir && entry.name != ".." {
                browser.current_dir.join(&entry.name)
            } else {
                browser.current_dir.clone()
            }
        } else {
            browser.current_dir.clone()
        };
        assert_eq!(cwd, tmp);
    }

    #[test]
    fn test_selected_cwd_from_session() {
        let mut app = App::new(make_sessions_with_projects(), None, Config::default());
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
        let app = App::new(make_sessions_with_projects(), None, Config::default());
        // selected=0 is a header
        let cwd = app.selected_cwd();
        assert!(cwd.is_some());
    }

    #[test]
    fn test_launch_request_resume_variant() {
        let mut app = App::new(make_sessions(), None, Config::default());
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
    fn test_launch_request_new_variant() {
        let app_cwd = "/tmp".to_string();
        let req = LaunchRequest::New { cwd: app_cwd.clone() };
        match req {
            LaunchRequest::New { cwd } => assert_eq!(cwd, "/tmp"),
            _ => panic!("expected New variant"),
        }
    }

    #[test]
    fn test_reload_sessions_updates_list() {
        let mut app = App::new(make_sessions(), None, Config::default());
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
            has_data: true, name: None,
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
                has_data: true, name: None,
            },
            SessionInfo {
                session_id: "s2".into(),
                project: "/Users/sane/Dev/beta".into(),
                project_name: "beta".into(),
                first_timestamp: 1500,
                last_timestamp: 3000,
                entry_count: 3,
                has_data: false, name: None,
            },
            SessionInfo {
                session_id: "s3".into(),
                project: "/Users/sane/Dev/gamma".into(),
                project_name: "gamma".into(),
                first_timestamp: 500,
                last_timestamp: 4000,
                entry_count: 10,
                has_data: true, name: None,
            },
        ]
    }

    #[test]
    fn test_hide_empty_filters_sessions() {
        // Default config has hide_empty=true, so empty sessions are filtered at construction
        let mut app = App::new(make_sessions_mixed_data(), None, Config::default());
        app.tree_view = false;
        app.recompute_filter();
        // s2 (index 1) has_data=false, should be excluded
        assert_eq!(app.filtered_indices, vec![0, 2]);

        // Disabling hide_empty shows all sessions
        app.hide_empty = false;
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }

    #[test]
    fn test_hide_empty_with_text_filter() {
        let mut app = App::new(make_sessions_mixed_data(), None, Config::default());
        app.tree_view = false;
        app.hide_empty = true;
        app.filter_text = "a".into(); // matches alpha and gamma
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 2]);

        // beta matches text but has_data=false
        app.filter_text = "beta".into();
        app.recompute_filter();
        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn test_tab_cycles_through_view_modes() {
        let mut app = App::new(make_sessions(), None, Config::default());
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
        let app = App::new(make_sessions(), None, Config::default());
        assert!(!app.shift_active);
    }

    #[test]
    fn test_tab_cycles_all_six_modes() {
        let mut app = App::new(make_sessions(), None, Config::default());

        // Helper to simulate tab press logic
        fn simulate_tab(app: &mut App) {
            match (app.tree_view, app.display_mode) {
                (true, DisplayMode::Name) => {
                    app.display_mode = DisplayMode::ShortDir;
                    app.recompute_tree();
                }
                (true, DisplayMode::ShortDir) => {
                    app.display_mode = DisplayMode::FullDir;
                    app.recompute_tree();
                }
                (true, DisplayMode::FullDir) => {
                    app.tree_view = false;
                    app.display_mode = DisplayMode::Name;
                }
                (false, DisplayMode::Name) => {
                    app.display_mode = DisplayMode::ShortDir;
                }
                (false, DisplayMode::ShortDir) => {
                    app.display_mode = DisplayMode::FullDir;
                }
                (false, DisplayMode::FullDir) => {
                    app.tree_view = true;
                    app.display_mode = DisplayMode::Name;
                    app.recompute_tree();
                }
            }
        }

        // Start: tree + Name
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);

        simulate_tab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::ShortDir);

        simulate_tab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::FullDir);

        simulate_tab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);

        simulate_tab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::ShortDir);

        simulate_tab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::FullDir);

        // Full cycle back to tree + Name
        simulate_tab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);
    }

    #[test]
    fn test_backtab_cycles_reverse() {
        let mut app = App::new(make_sessions(), None, Config::default());

        fn simulate_backtab(app: &mut App) {
            match (app.tree_view, app.display_mode) {
                (true, DisplayMode::Name) => {
                    app.tree_view = false;
                    app.display_mode = DisplayMode::FullDir;
                }
                (true, DisplayMode::ShortDir) => {
                    app.display_mode = DisplayMode::Name;
                    app.recompute_tree();
                }
                (true, DisplayMode::FullDir) => {
                    app.display_mode = DisplayMode::ShortDir;
                    app.recompute_tree();
                }
                (false, DisplayMode::Name) => {
                    app.tree_view = true;
                    app.display_mode = DisplayMode::FullDir;
                    app.recompute_tree();
                }
                (false, DisplayMode::ShortDir) => {
                    app.display_mode = DisplayMode::Name;
                }
                (false, DisplayMode::FullDir) => {
                    app.display_mode = DisplayMode::ShortDir;
                }
            }
        }

        // Start: tree + Name
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);

        // Reverse: tree+Name → flat+FullDir
        simulate_backtab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::FullDir);

        simulate_backtab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::ShortDir);

        simulate_backtab(&mut app);
        assert!(!app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);

        simulate_backtab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::FullDir);

        simulate_backtab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::ShortDir);

        simulate_backtab(&mut app);
        assert!(app.tree_view);
        assert_eq!(app.display_mode, DisplayMode::Name);
    }

    #[test]
    fn test_session_name_set_directly() {
        let mut app = App::new(make_sessions(), None, Config::default());
        // Initially no names
        assert!(app.sessions[0].name.is_none());

        // Directly set a name (simulates what rename does)
        app.sessions[0].name = Some("My Session".to_string());
        assert_eq!(app.sessions[0].name, Some("My Session".to_string()));
    }

    #[test]
    fn test_rename_mode_transitions() {
        let mut app = App::new(make_sessions(), None, Config::default());
        // Select a session (expand first header, then move to session)
        app.tree_view = false;
        app.recompute_filter();
        app.selected = 0;

        // Start renaming
        let idx = app.selected_session_index().unwrap();
        let session_id = app.sessions[idx].session_id.clone();
        app.rename_session_id = Some(session_id.clone());
        app.rename_text = String::new();
        app.mode = AppMode::Renaming;

        assert_eq!(app.mode, AppMode::Renaming);
        assert_eq!(app.rename_session_id, Some(session_id));
    }

    #[test]
    fn test_hide_empty_toggle_restores() {
        let mut app = App::new(make_sessions_mixed_data(), None, Config::default());
        app.tree_view = false;

        app.hide_empty = true;
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 2]);

        app.hide_empty = false;
        app.recompute_filter();
        assert_eq!(app.filtered_indices, vec![0, 1, 2]);
    }
}
