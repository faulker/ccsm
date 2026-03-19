use crate::app::{App, AppMode, DuplicateSource, FlatRow, LaunchRequest, TreeRow};
use crate::{data, live};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, ModifierKeyCode};

/// Normalize a key event so that Shift+letter produces an uppercase `Char`.
///
/// With the enhanced keyboard protocol, crossterm reports `Char('a')` with
/// `KeyModifiers::SHIFT` rather than `Char('A')`.  `tui_input` inserts the
/// char as-is, so we uppercase it here before delegation.
fn normalize_key(mut key: crossterm::event::KeyEvent) -> crossterm::event::KeyEvent {
    if let KeyCode::Char(c) = key.code {
        if key.modifiers.contains(KeyModifiers::SHIFT) && c.is_ascii_lowercase() {
            key.code = KeyCode::Char(c.to_ascii_uppercase());
        }
    }
    key
}

impl App {
    /// Handle a key event while the rename popup is open.
    ///
    /// Esc cancels, Enter commits the new name, all other editing keys
    /// (arrows, Home/End, Backspace, Delete, printable chars) are delegated
    /// to the `rename_input` state via `tui_input`.
    fn handle_rename_event(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::Event;
        use tui_input::backend::crossterm::EventHandler;

        // Live session rename (rename_project is None)
        if self.rename_project.is_none() {
            match key.code {
                KeyCode::Esc => {
                    self.mode = AppMode::Normal;
                    self.rename_input = tui_input::Input::default();
                    self.rename_session_id = None;
                }
                KeyCode::Enter => {
                    if let Some(tmux_name) = self.rename_session_id.clone() {
                        let new_name = self.rename_input.value().trim().to_string();
                        if !new_name.is_empty() {
                            // Check for a duplicate name (ignoring the session being renamed)
                            let is_duplicate = new_name != tmux_name
                                && self.live_sessions.iter().any(|ls| ls.tmux_name == new_name);
                            if is_duplicate {
                                self.duplicate_name = Some(new_name);
                                self.duplicate_source = Some(DuplicateSource::Renaming);
                                self.duplicate_cwd = None;
                                self.mode = AppMode::DuplicateSession;
                                return;
                            }

                            let cwd = self.live_sessions.iter()
                                .find(|ls| ls.tmux_name == tmux_name)
                                .map(|ls| ls.cwd.clone());
                            if let Some(cwd) = cwd {
                                for session in &mut self.sessions {
                                    if session.project == cwd && session.name.as_deref() == Some(&tmux_name) {
                                        if let Err(e) = data::save_custom_title(&session.project, &session.session_id, &new_name) {
                                            eprintln!("Failed to save custom title: {e}");
                                        }
                                        session.name = Some(new_name.clone());
                                    }
                                }
                                self.preview_cache.clear();
                            }
                            let tmux = crate::config::Config::load().tmux_bin().to_string();
                            match std::process::Command::new(&tmux)
                                .args(["-L", live::TMUX_SOCKET, "rename-session", "-t", &tmux_name, &new_name])
                                .output()
                            {
                                Err(e) => eprintln!("Failed to rename tmux session: {e}"),
                                Ok(out) if !out.status.success() => {
                                    eprintln!("Failed to rename tmux session: {}", String::from_utf8_lossy(&out.stderr).trim());
                                }
                                Ok(_) => {}
                            }
                            self.live_sessions = live::discover_live_sessions();
                            self.live_preview_cache.clear();
                            self.recompute_flat_rows();
                            self.recompute_tree();
                        }
                    }
                    self.rename_session_id = None;
                    self.rename_input = tui_input::Input::default();
                    self.mode = AppMode::Normal;
                }
                _ => {
                    self.rename_input.handle_event(&Event::Key(normalize_key(key)));
                }
            }
            return;
        }

        // Historical session rename
        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.rename_input = tui_input::Input::default();
                self.rename_session_id = None;
                self.rename_project = None;
            }
            KeyCode::Enter => {
                if let Some(session_id) = self.rename_session_id.take() {
                    let project = self.rename_project.take().unwrap_or_default();
                    let name = self.rename_input.value().trim().to_string();
                    if let Err(e) = data::save_custom_title(&project, &session_id, &name) {
                        eprintln!("Failed to save custom title: {e}");
                    }
                    let name_opt = if name.is_empty() { None } else { Some(name) };
                    for s in &mut self.sessions {
                        if s.session_id == session_id {
                            s.name = name_opt.clone();
                        }
                    }
                    self.preview_cache.clear();
                }
                self.rename_input = tui_input::Input::default();
                self.mode = AppMode::Normal;
            }
            _ => {
                self.rename_input.handle_event(&Event::Key(normalize_key(key)));
            }
        }
    }

    /// Handle a key event while the new-session naming popup is open.
    ///
    /// Esc cancels, Enter confirms (using the placeholder if empty), all other
    /// editing keys are delegated to `naming_input` via `tui_input`.
    fn handle_naming_event(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::Event;
        use tui_input::backend::crossterm::EventHandler;

        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.naming_input = tui_input::Input::default();
                self.naming_cwd = None;
            }
            KeyCode::Enter => {
                let raw = if self.naming_input.value().is_empty() {
                    self.naming_placeholder.clone()
                } else {
                    self.naming_input.value().to_string()
                };
                // Sanitize: tmux disallows '.' ':' and whitespace in session names
                let name: String = raw
                    .chars()
                    .map(|c| if c == '.' || c == ':' || c.is_whitespace() { '-' } else { c })
                    .collect();
                let name = if name.is_empty() { self.naming_placeholder.clone() } else { name };
                // Check for a duplicate before consuming state
                if self.live_sessions.iter().any(|ls| ls.tmux_name == name) {
                    self.duplicate_name = Some(name);
                    self.duplicate_source = Some(DuplicateSource::NamingSession);
                    self.duplicate_cwd = self.naming_cwd.take();
                    self.mode = AppMode::DuplicateSession;
                    return;
                }
                let cwd = self.naming_cwd.take().unwrap_or_else(|| ".".to_string());
                self.mode = AppMode::Normal;
                self.naming_input = tui_input::Input::default();
                self.launch_session = Some(LaunchRequest::NewLive { name, cwd });
            }
            _ => {
                self.naming_input.handle_event(&Event::Key(normalize_key(key)));
            }
        }
    }

    /// Handle a key event while the duplicate-session confirmation popup is open.
    ///
    /// `o`/Enter opens the existing session, `r` returns to naming/renaming, `Esc` cancels.
    fn handle_duplicate_event(&mut self, key: crossterm::event::KeyEvent) {
        let name = match self.duplicate_name.clone() {
            Some(n) => n,
            None => {
                self.mode = AppMode::Normal;
                return;
            }
        };

        match key.code {
            KeyCode::Char('o') | KeyCode::Enter => {
                self.launch_session = Some(LaunchRequest::AttachLive { tmux_name: name });
                self.duplicate_name = None;
                self.duplicate_source = None;
                self.duplicate_cwd = None;
                self.naming_input = tui_input::Input::default();
                self.rename_input = tui_input::Input::default();
                self.rename_session_id = None;
                self.mode = AppMode::Normal;
            }
            KeyCode::Char('r') => {
                match self.duplicate_source.take() {
                    Some(DuplicateSource::NamingSession) => {
                        self.naming_cwd = self.duplicate_cwd.take();
                        self.mode = AppMode::NamingSession;
                    }
                    Some(DuplicateSource::Renaming) | None => {
                        self.duplicate_cwd = None;
                        self.mode = AppMode::Renaming;
                    }
                }
                self.duplicate_name = None;
            }
            KeyCode::Esc => {
                self.duplicate_name = None;
                self.duplicate_source = None;
                self.duplicate_cwd = None;
                self.naming_input = tui_input::Input::default();
                self.naming_cwd = None;
                self.rename_input = tui_input::Input::default();
                self.rename_session_id = None;
                self.rename_project = None;
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
    }

    /// Read one terminal event and dispatch it based on the current `AppMode`.
    ///
    /// Tracks Shift state, delegates to modal handlers when a popup is open, and
    /// processes navigation, filter, and action keys in Normal mode.
    pub fn handle_event(&mut self) -> anyhow::Result<()> {
        if let Event::Key(key) = event::read()? {
            // Track shift state for UI highlighting
            // Capture before updating — needed for terminals (e.g. Ghostty) that don't
            // populate KeyModifiers::SHIFT on Enter, so the pre-update value is used
            // as a fallback in the Shift+Enter match arm below.
            let prev_shift_active = self.shift_active;
            match (&key.code, key.kind) {
                // Bare shift press/release — update flag and consume event
                (KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift), KeyEventKind::Press) => {
                    self.shift_active = true;
                    self.needs_redraw = true;
                    return Ok(());
                }
                (KeyCode::Modifier(ModifierKeyCode::LeftShift | ModifierKeyCode::RightShift), KeyEventKind::Release) => {
                    self.shift_active = false;
                    self.needs_redraw = true;
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
                        if let crate::update::UpdateStatus::Available(ref info) = self.update_status {
                            self.perform_update = Some(info.clone());
                            self.update_status = crate::update::UpdateStatus::Downloading;
                        }
                        self.mode = AppMode::Normal;
                    }
                    KeyCode::Char('n') | KeyCode::Esc => {
                        self.update_status = crate::update::UpdateStatus::None;
                        self.mode = AppMode::Normal;
                    }
                    _ => {}
                }
                return Ok(());
            }

            if self.mode == AppMode::Help {
                self.mode = AppMode::Normal;
                return Ok(());
            }

            if self.mode == AppMode::NamingSession {
                self.handle_naming_event(key);
                return Ok(());
            }

            if self.mode == AppMode::Renaming {
                self.handle_rename_event(key);
                return Ok(());
            }

            if self.mode == AppMode::DuplicateSession {
                self.handle_duplicate_event(key);
                return Ok(());
            }

            if self.mode == AppMode::Config {
                self.handle_config_event(key);
                return Ok(());
            }

            if self.mode == AppMode::MissingDeps {
                self.handle_missing_deps_event(key);
                return Ok(());
            }

            if self.filter_active {
                use crossterm::event::Event;
                use tui_input::backend::crossterm::EventHandler;
                match key.code {
                    KeyCode::Esc => {
                        self.filter_active = false;
                        self.filter_input = tui_input::Input::default();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                    }
                    KeyCode::Enter => {
                        self.filter_active = false;
                    }
                    KeyCode::Down => {
                        let count = self.visible_item_count();
                        if count > 0 {
                            self.selected = (self.selected + 1).min(count - 1);
                            self.preview_scroll = u16::MAX;
                        }
                    }
                    KeyCode::Up => {
                        self.selected = self.selected.saturating_sub(1);
                        self.preview_scroll = u16::MAX;
                    }
                    _ => {
                        if self.filter_input.handle_event(&Event::Key(normalize_key(key))).is_some() {
                            self.recompute_filter();
                            self.preview_scroll = u16::MAX;
                        }
                    }
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
                // '?' is Shift+/ on US keyboards; some terminals send Char('?') and
                // others send Char('/') with SHIFT — handle both before the '/' filter.
                (KeyCode::Char('?'), _) | (KeyCode::Char('/'), KeyModifiers::SHIFT) => {
                    self.mode = AppMode::Help;
                }
                (KeyCode::Char('/'), _) => {
                    self.filter_active = true;
                }
                (KeyCode::Char('o'), KeyModifiers::NONE) => {
                    self.mode = AppMode::Config;
                }
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, KeyModifiers::NONE) => {
                    let count = self.visible_item_count();
                    if count > 0 {
                        self.selected =
                            (self.selected + 1).min(count - 1);
                        self.preview_scroll = u16::MAX;
                    }
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, KeyModifiers::NONE) => {
                    self.selected = self.selected.saturating_sub(1);
                    self.preview_scroll = u16::MAX;
                }
                (KeyCode::Char('J' | 'j'), KeyModifiers::SHIFT) | (KeyCode::Down, KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                (KeyCode::Char('K' | 'k'), KeyModifiers::SHIFT) | (KeyCode::Up, KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    if let Some(cwd) = self.selected_cwd() {
                        let path = std::path::Path::new(&cwd);
                        let dir = if path.exists() {
                            cwd
                        } else {
                            ".".to_string()
                        };
                        let placeholder = live::generate_auto_name(&dir, &self.live_sessions);
                        self.naming_placeholder = placeholder;
                        self.naming_cwd = Some(dir);
                        self.naming_input = tui_input::Input::default();
                        self.mode = AppMode::NamingSession;
                    }
                }
                (KeyCode::Char('f'), KeyModifiers::NONE) => {
                    self.toggle_favorite();
                    self.recompute_flat_rows();
                    self.recompute_tree();
                }
                (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    self.live_filter = !self.live_filter;
                    self.recompute_flat_rows();
                    self.recompute_tree();
                    self.save_config();
                }
                (KeyCode::Char('x'), KeyModifiers::NONE) => {
                    if let Some(idx) = self.selected_live_index() {
                        let name = self.live_sessions[idx].tmux_name.clone();
                        if let Err(e) = live::stop_live_session(&name) {
                            eprintln!("Failed to stop session: {e}");
                        }
                        self.live_sessions = live::discover_live_sessions();
                        self.live_preview_cache.remove(&name);
                        self.recompute_flat_rows();
                        self.recompute_tree();
                    }
                }
                (KeyCode::Char('r'), KeyModifiers::NONE) => {
                    // Check if a live session is selected first
                    if let Some(idx) = self.selected_live_index() {
                        let session = &self.live_sessions[idx];
                        self.rename_input = tui_input::Input::from(session.display_name.clone());
                        self.rename_session_id = Some(session.tmux_name.clone());
                        self.rename_project = None;
                        self.mode = AppMode::Renaming;
                        return Ok(());
                    }
                    if let Some(idx) = self.selected_session_index() {
                        // For chains, always rename the most recent session
                        let resume_idx = self
                            .chain_map
                            .get(&idx)
                            .and_then(|chain| {
                                chain
                                    .iter()
                                    .max_by_key(|&&i| self.sessions[i].last_timestamp)
                                    .copied()
                            })
                            .unwrap_or(idx);
                        let session = &self.sessions[resume_idx];
                        self.rename_session_id = Some(session.session_id.clone());
                        self.rename_project = Some(session.project.clone());
                        // Pre-fill with the chain's effective name (may come from any member)
                        self.rename_input = tui_input::Input::from(
                            self.chain_name_for(idx).unwrap_or("").to_string()
                        );
                        self.mode = AppMode::Renaming;
                    }
                }
                (KeyCode::Char('N' | 'n'), KeyModifiers::SHIFT) => {
                    let cwd = self
                        .selected_cwd()
                        .filter(|p| std::path::Path::new(p).exists())
                        .unwrap_or_else(|| ".".to_string());
                    self.launch_session = Some(LaunchRequest::NewDirect { cwd });
                }
                (KeyCode::Enter, _) if (key.modifiers.contains(KeyModifiers::SHIFT) || prev_shift_active) && self.is_historical_selected() => {
                    // Shift+Enter: open historical session directly (no tmux)
                    if self.tree_view {
                        if let Some(TreeRow::Session { session_index }) =
                            self.tree_rows.get(self.selected).cloned()
                        {
                            let session_id = self.resume_session_id_for(session_index).to_string();
                            let cwd = self.sessions[session_index].project.clone();
                            self.launch_session = Some(LaunchRequest::Direct { session_id, cwd });
                        }
                    } else if let Some(FlatRow::HistoryItem { session_index }) =
                        self.flat_rows.get(self.selected).cloned()
                    {
                        let session_id = self.resume_session_id_for(session_index).to_string();
                        let cwd = self.sessions[session_index].project.clone();
                        self.launch_session = Some(LaunchRequest::Direct { session_id, cwd });
                    }
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
                                let session_id = self.resume_session_id_for(session_index).to_string();
                                let cwd = self.sessions[session_index].project.clone();
                                self.launch_session = Some(LaunchRequest::Resume { session_id, cwd });
                            }
                            Some(TreeRow::LiveItem { live_index }) => {
                                let name = self.live_sessions[live_index].tmux_name.clone();
                                self.launch_session = Some(LaunchRequest::AttachLive { tmux_name: name });
                            }
                            Some(TreeRow::RunningHeader { project, .. }) => {
                                let key = format!("running:{}", project);
                                if self.collapsed.contains(&key) {
                                    self.collapsed.remove(&key);
                                } else {
                                    self.collapsed.insert(key);
                                }
                                self.recompute_tree();
                            }
                            Some(TreeRow::HistoryHeader { project, .. }) => {
                                let key = format!("history:{}", project);
                                if self.collapsed.contains(&key) {
                                    self.collapsed.remove(&key);
                                } else {
                                    self.collapsed.insert(key);
                                }
                                self.recompute_tree();
                            }
                            Some(TreeRow::FavoritesSeparator) | None => {}
                        }
                    } else {
                        match self.flat_rows.get(self.selected).cloned() {
                            Some(FlatRow::LiveItem { live_index }) => {
                                let name = self.live_sessions[live_index].tmux_name.clone();
                                self.launch_session = Some(LaunchRequest::AttachLive { tmux_name: name });
                            }
                            Some(FlatRow::HistoryItem { session_index }) => {
                                let session_id = self.resume_session_id_for(session_index).to_string();
                                let cwd = self.sessions[session_index].project.clone();
                                self.launch_session = Some(LaunchRequest::Resume { session_id, cwd });
                            }
                            _ => {}
                        }
                    }
                }
                (KeyCode::Right, _)
                    if self.tree_view =>
                {
                    match self.tree_rows.get(self.selected).cloned() {
                        Some(TreeRow::Header { project, .. }) => {
                            if self.collapsed.contains(&project) {
                                self.collapsed.remove(&project);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::RunningHeader { project, .. }) => {
                            let key = format!("running:{}", project);
                            if self.collapsed.contains(&key) {
                                self.collapsed.remove(&key);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::HistoryHeader { project, .. }) => {
                            let key = format!("history:{}", project);
                            if self.collapsed.contains(&key) {
                                self.collapsed.remove(&key);
                                self.recompute_tree();
                            }
                        }
                        _ => {}
                    }
                }
                (KeyCode::Left, _)
                    if self.tree_view =>
                {
                    match self.tree_rows.get(self.selected).cloned() {
                        Some(TreeRow::Header { project, .. }) => {
                            if !self.collapsed.contains(&project) {
                                self.collapsed.insert(project);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::RunningHeader { project, .. }) => {
                            let key = format!("running:{}", project);
                            if !self.collapsed.contains(&key) {
                                self.collapsed.insert(key);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::HistoryHeader { project, .. }) => {
                            let key = format!("history:{}", project);
                            if !self.collapsed.contains(&key) {
                                self.collapsed.insert(key);
                                self.recompute_tree();
                            }
                        }
                        Some(TreeRow::Session { .. }) => {
                            // Move cursor to nearest HistoryHeader above
                            for i in (0..self.selected).rev() {
                                if matches!(self.tree_rows.get(i), Some(TreeRow::HistoryHeader { .. })) {
                                    self.selected = i;
                                    self.preview_scroll = u16::MAX;
                                    break;
                                }
                            }
                        }
                        Some(TreeRow::LiveItem { .. }) => {
                            // Move cursor to nearest RunningHeader above
                            for i in (0..self.selected).rev() {
                                if matches!(self.tree_rows.get(i), Some(TreeRow::RunningHeader { .. })) {
                                    self.selected = i;
                                    self.preview_scroll = u16::MAX;
                                    break;
                                }
                            }
                        }
                        Some(TreeRow::FavoritesSeparator) | None => {}
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }
}
