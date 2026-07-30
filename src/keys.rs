use crate::app::{
    App, AppMode, DuplicateSource, FlatRow, HelpTab, LaunchRequest, MainTab, NamingFocus,
    NewSessionMode, TreeRow,
};
use crate::data::AgentBackend;
use crate::{data, live};
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers, ModifierKeyCode, MouseEventKind};

/// The character a US-layout key produces when Shift is held. Covers the
/// number row and the punctuation keys; letters are handled by uppercasing.
///
/// This is a layout assumption, and the only one available: without
/// `REPORT_ALTERNATE_KEYS` the terminal sends the *base* key and never says
/// what the layout would have produced. See `normalize_key` for why that flag
/// is not requested.
fn shifted_char(c: char) -> Option<char> {
    Some(match c {
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '0' => ')',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        '`' => '~',
        _ => return None,
    })
}

/// Normalize a key event so a Shift-held key produces the character it types.
///
/// With the enhanced keyboard protocol, crossterm reports the *unshifted* key
/// plus `KeyModifiers::SHIFT`: `Shift+a` arrives as `Char('a')` and `Shift+2`
/// as `Char('2')`. `tui_input` inserts the char as-is, so without this an `@`
/// lands in the field as a `2`.
///
/// Requesting `KeyboardEnhancementFlags::REPORT_ALTERNATE_KEYS` would make the
/// terminal resolve this per layout, which is strictly more correct, but it
/// also clears `SHIFT` on every shifted keypress — and `handle_event` reads
/// that modifier to drive the status bar's Shift hints, which would then go
/// dark while Shift is still held. Fixing the text path only keeps that
/// affordance intact.
///
/// A key that already carries its shifted character (a terminal that resolved
/// it) is left alone, so this never double-shifts.
pub(crate) fn normalize_key(mut key: crossterm::event::KeyEvent) -> crossterm::event::KeyEvent {
    if let KeyCode::Char(c) = key.code {
        if key.modifiers.contains(KeyModifiers::SHIFT) {
            if c.is_ascii_lowercase() {
                key.code = KeyCode::Char(c.to_ascii_uppercase());
            } else if let Some(shifted) = shifted_char(c) {
                key.code = KeyCode::Char(shifted);
            }
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
                            let tmux = self.config.tmux_bin();
                            match std::process::Command::new(tmux)
                                .args(["-L", live::TMUX_SOCKET, "rename-session", "-t", &tmux_name, &new_name])
                                .output()
                            {
                                Err(e) => eprintln!("Failed to rename tmux session: {e}"),
                                Ok(out) if !out.status.success() => {
                                    eprintln!("Failed to rename tmux session: {}", String::from_utf8_lossy(&out.stderr).trim());
                                }
                                Ok(_) => {}
                            }
                            // Migrate activity state and poll timestamp to the new name
                            if let Some(state) = self.activity_states.remove(&tmux_name) {
                                self.activity_states.insert(new_name.clone(), state);
                            }
                            if let Some(ts) = self.activity_last_poll.remove(&tmux_name) {
                                self.activity_last_poll.insert(new_name.clone(), ts);
                            }
                            self.live_sessions = live::discover_live_sessions(self.config.tmux_bin());
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
    /// Focus starts on the name. Down moves to Agent (when both backends are
    /// shown) then Type; Left/Right cycle the focused switcher. Esc cancels,
    /// Enter confirms (using the placeholder if empty). Letter keys only reach
    /// the name field while it is focused.
    pub(crate) fn handle_naming_event(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::Event;
        use tui_input::backend::crossterm::EventHandler;

        match key.code {
            KeyCode::Esc => {
                self.mode = AppMode::Normal;
                self.naming_input = tui_input::Input::default();
                self.naming_cwd = None;
                self.naming_mode = NewSessionMode::Plain;
                self.naming_backend = AgentBackend::ClaudeCode;
                self.naming_focus = NamingFocus::Name;
            }
            KeyCode::Down => self.naming_focus_down(),
            KeyCode::Up => self.naming_focus_up(),
            KeyCode::Left | KeyCode::Right
                if self.naming_focus == NamingFocus::Agent =>
            {
                self.cycle_naming_backend();
            }
            KeyCode::Left | KeyCode::Right
                if self.naming_focus == NamingFocus::Type =>
            {
                self.cycle_naming_mode(key.code == KeyCode::Right);
            }
            // Left/Right on the name row still move the text cursor.
            KeyCode::Left | KeyCode::Right
                if self.naming_focus == NamingFocus::Name && self.naming_mode.needs_name() =>
            {
                self.naming_input
                    .handle_event(&Event::Key(normalize_key(key)));
            }
            // A direct session never reaches tmux, so there is no name to take
            // and no duplicate to check for.
            KeyCode::Enter if self.naming_mode == NewSessionMode::Direct => {
                let backend = self.naming_backend;
                if !self.ensure_backend_available(backend) {
                    return;
                }
                let cwd = self.naming_cwd.take().unwrap_or_else(|| ".".to_string());
                self.mode = AppMode::Normal;
                self.naming_input = tui_input::Input::default();
                self.naming_mode = NewSessionMode::Plain;
                self.naming_backend = AgentBackend::ClaudeCode;
                self.naming_focus = NamingFocus::Name;
                self.launch_session = Some(LaunchRequest::NewDirect { cwd, backend });
            }
            KeyCode::Enter => {
                let backend = self.naming_backend;
                if !self.ensure_backend_available(backend) {
                    return;
                }
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
                let name = if name.is_empty() {
                    self.naming_placeholder
                        .chars()
                        .map(|c| if c == '.' || c == ':' || c.is_whitespace() { '-' } else { c })
                        .collect()
                } else {
                    name
                };
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
                let mode = std::mem::replace(&mut self.naming_mode, NewSessionMode::Plain);
                self.naming_backend = AgentBackend::ClaudeCode;
                self.naming_focus = NamingFocus::Name;
                self.launch_session = Some(LaunchRequest::NewLive {
                    name,
                    cwd,
                    dangerous: mode == NewSessionMode::Dangerous,
                    worktree: mode == NewSessionMode::Worktree,
                    backend,
                });
            }
            // Typing only edits the name while that row is focused.
            _ if self.naming_focus == NamingFocus::Name && self.naming_mode.needs_name() => {
                self.naming_input
                    .handle_event(&Event::Key(normalize_key(key)));
            }
            _ => {}
        }
    }

    /// Handle a key event while the directory-picker modal is open.
    ///
    /// Actual state changes are delegated to the `App` methods defined in
    /// `app/dir_browser.rs` so they stay unit-testable without needing a raw
    /// `KeyEvent`; this function only handles routing keys to `path_input`
    /// (which does need the raw event, for Shift normalization) while typing a path.
    fn handle_dir_picker_event(&mut self, key: crossterm::event::KeyEvent) {
        use crossterm::event::Event;
        use tui_input::backend::crossterm::EventHandler;

        let input_active = self.dir_browser.as_ref().is_some_and(|b| b.input_active);

        if input_active {
            match key.code {
                KeyCode::Esc => self.dir_picker_escape(),
                KeyCode::Enter => self.dir_picker_commit_input(),
                _ => {
                    if let Some(browser) = self.dir_browser.as_mut() {
                        browser.path_input.handle_event(&Event::Key(normalize_key(key)));
                    }
                }
            }
            return;
        }

        match key.code {
            KeyCode::Esc => self.dir_picker_escape(),
            KeyCode::Char('j') | KeyCode::Down => self.dir_picker_move_down(),
            KeyCode::Char('k') | KeyCode::Up => self.dir_picker_move_up(),
            KeyCode::Enter => self.dir_picker_enter(),
            KeyCode::Char(' ') => self.dir_picker_select(),
            KeyCode::Char('/') => self.dir_picker_activate_input(),
            _ => {}
        }
    }

    /// Open the help overlay on the page matching the current tab, so help
    /// about jobs is one keystroke away while looking at jobs.
    pub fn open_help(&mut self) {
        self.help_tab = match self.main_tab {
            MainTab::Jobs => HelpTab::Jobs,
            MainTab::Sessions => HelpTab::Sessions,
            // The config keys live on the General page, alongside the global
            // ones and the directory picker they open.
            MainTab::Config => HelpTab::General,
        };
        self.help_scroll = 0;
        self.mode = AppMode::Help;
    }

    /// Handle a key event while the tabbed help overlay is open: Tab/`h`/`l`
    /// switch pages, `1`..`3` jump to one, `j`/`k`/arrows/PgUp/PgDn scroll the
    /// current page, and Esc/`?`/`q` close it.
    ///
    /// The help overlay is the fallback for every hint the status bar drops on
    /// a narrow terminal, so it has to be scrollable: at 80x24 its content area
    /// is 15 rows against a Sessions page of roughly 24 lines.
    pub(crate) fn handle_help_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Tab | KeyCode::Right | KeyCode::Char('l') => {
                self.help_tab = self.help_tab.next();
                self.help_scroll = 0;
            }
            KeyCode::BackTab | KeyCode::Left | KeyCode::Char('h') => {
                self.help_tab = self.help_tab.prev();
                self.help_scroll = 0;
            }
            KeyCode::Char(c @ '1'..='3') => {
                let idx = c as usize - '1' as usize;
                self.help_tab = HelpTab::ALL[idx];
                self.help_scroll = 0;
            }
            KeyCode::Char('j') | KeyCode::Down => {
                self.help_scroll = self.help_scroll.saturating_add(1);
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.help_scroll = self.help_scroll.saturating_sub(1);
            }
            KeyCode::PageDown => {
                self.help_scroll = self.help_scroll.saturating_add(10);
            }
            KeyCode::PageUp => {
                self.help_scroll = self.help_scroll.saturating_sub(10);
            }
            KeyCode::Home => self.help_scroll = 0,
            _ => self.mode = AppMode::Normal,
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
                self.request_attach(name);
                self.duplicate_name = None;
                self.duplicate_source = None;
                self.duplicate_cwd = None;
                self.naming_input = tui_input::Input::default();
                self.naming_mode = NewSessionMode::Plain;
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
                self.naming_mode = NewSessionMode::Plain;
                self.rename_input = tui_input::Input::default();
                self.rename_session_id = None;
                self.rename_project = None;
                self.mode = AppMode::Normal;
            }
            _ => {}
        }
    }

    /// Handle a key event while the stop-live-session confirmation is open.
    ///
    /// Uses the same `y`/`n`/Enter/Esc vocabulary as the job confirm and the
    /// update prompt, so every confirmation in the app answers to the same keys.
    fn handle_stop_confirm_event(&mut self, key: crossterm::event::KeyEvent) {
        match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                self.stop_confirm_name = None;
                self.mode = AppMode::Normal;
                self.stop_selected_live_session();
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                self.stop_confirm_name = None;
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
        let event = event::read()?;

        if let Event::Mouse(mouse) = event {
            match mouse.kind {
                MouseEventKind::ScrollDown => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                MouseEventKind::ScrollUp => {
                    self.preview_auto_scroll = false;
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                _ => {}
            }
            return Ok(());
        }

        if let Event::Key(key) = event {
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

            // A status-bar alert describes the action that just failed, so the
            // next keypress supersedes it. Cleared before dispatch so a handler
            // that raises a new alert for *this* key still wins.
            self.status_error = None;

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
                self.handle_help_event(key);
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

            if self.mode == AppMode::MissingDeps {
                self.handle_missing_deps_event(key);
                return Ok(());
            }

            if self.mode == AppMode::DirPicker {
                self.handle_dir_picker_event(key);
                return Ok(());
            }

            if self.mode == AppMode::StopSessionConfirm {
                self.handle_stop_confirm_event(key);
                return Ok(());
            }

            if self.mode == AppMode::JobConfirm {
                self.handle_job_confirm_event(key);
                return Ok(());
            }

            if self.mode == AppMode::JobForm {
                self.handle_job_form_event(key);
                return Ok(());
            }

            // The Jobs and Config tabs have their own full key maps (each
            // including quit/help/tab switching), so they are dispatched before
            // the Sessions-tab bindings below.
            if self.main_tab == MainTab::Jobs && !self.filter_active {
                self.handle_jobs_tab_event(key);
                return Ok(());
            }

            if self.main_tab == MainTab::Config && !self.filter_active {
                self.handle_config_tab_event(key);
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
                        self.preview_auto_scroll = true;
                    }
                    KeyCode::Enter => {
                        self.filter_active = false;
                    }
                    KeyCode::Down => {
                        let count = self.visible_item_count();
                        if count > 0 {
                            self.selected = (self.selected + 1).min(count - 1);
                            self.preview_scroll = u16::MAX;
                            self.preview_auto_scroll = true;
                        }
                    }
                    KeyCode::Up => {
                        self.selected = self.selected.saturating_sub(1);
                        self.preview_scroll = u16::MAX;
                        self.preview_auto_scroll = true;
                    }
                    _ => {
                        if self.filter_input.handle_event(&Event::Key(normalize_key(key))).is_some() {
                            self.recompute_filter();
                            self.preview_scroll = u16::MAX;
                            self.preview_auto_scroll = true;
                        }
                    }
                }
                return Ok(());
            }

            self.dispatch_normal_key_with_shift(key, prev_shift_active);
        }
        Ok(())
    }
    /// Dispatch a key press in Normal mode on the Sessions tab.
    ///
    /// Split out of `handle_event` so it can be driven directly from tests:
    /// `handle_event` blocks on `event::read()`, which makes the largest key
    /// map in the app the only one that was untestable.
    /// Dispatch a Normal-mode key with no prior Shift state held.
    #[cfg(test)]
    pub(crate) fn dispatch_normal_key(&mut self, key: crossterm::event::KeyEvent) {
        self.dispatch_normal_key_with_shift(key, false);
    }

    /// `prev_shift_active` is the Shift flag as it stood *before* this event
    /// updated it, which some terminals (Ghostty) need for Shift+Enter: they
    /// do not set `KeyModifiers::SHIFT` on Enter itself.
    pub(crate) fn dispatch_normal_key_with_shift(
        &mut self,
        key: crossterm::event::KeyEvent,
        prev_shift_active: bool,
    ) {
            match (key.code, key.modifiers) {
                (KeyCode::Char('q'), _) => {
                    self.should_quit = true;
                }
                (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
                    self.should_quit = true;
                }
                // Esc never quits. Every modal in the app treats Esc as
                // "back out", so an Esc pressed one time too many while
                // dismissing a popup must not take the whole app down. It
                // clears an active filter, and is otherwise inert.
                (KeyCode::Esc, _) => {
                    if !self.filter_input.value().is_empty() {
                        self.filter_input = tui_input::Input::default();
                        self.recompute_filter();
                        self.preview_scroll = u16::MAX;
                        self.preview_auto_scroll = true;
                    }
                }
                // '?' is Shift+/ on US keyboards; some terminals send Char('?') and
                // others send Char('/') with SHIFT — handle both before the '/' filter.
                (KeyCode::Char('?'), _) | (KeyCode::Char('/'), KeyModifiers::SHIFT) => {
                    self.open_help();
                }
                (KeyCode::Tab, _) => {
                    self.cycle_main_tab(true);
                }
                (KeyCode::BackTab, _) => {
                    self.cycle_main_tab(false);
                }
                (KeyCode::Char('/'), _) => {
                    self.filter_active = true;
                }
                (KeyCode::Char('o'), KeyModifiers::NONE) => {
                    self.open_config_tab();
                }
                (KeyCode::Char('j'), KeyModifiers::NONE) | (KeyCode::Down, KeyModifiers::NONE) => {
                    let count = self.visible_item_count();
                    if count > 0 {
                        self.selected =
                            (self.selected + 1).min(count - 1);
                        self.preview_scroll = u16::MAX;
                        self.preview_auto_scroll = true;
                    }
                }
                (KeyCode::Char('k'), KeyModifiers::NONE) | (KeyCode::Up, KeyModifiers::NONE) => {
                    self.selected = self.selected.saturating_sub(1);
                    self.preview_scroll = u16::MAX;
                    self.preview_auto_scroll = true;
                }
                (KeyCode::Char('J' | 'j'), KeyModifiers::SHIFT) | (KeyCode::Down, KeyModifiers::SHIFT) => {
                    self.preview_scroll = self.preview_scroll.saturating_add(3);
                }
                (KeyCode::Char('K' | 'k'), KeyModifiers::SHIFT) | (KeyCode::Up, KeyModifiers::SHIFT) => {
                    self.preview_auto_scroll = false;
                    self.preview_scroll = self.preview_scroll.saturating_sub(3);
                }
                (KeyCode::Char('n'), KeyModifiers::NONE) => {
                    self.open_naming_popup(NewSessionMode::Plain);
                }
                // Space is the toggle key in both tabs (favorite here,
                // auto-resume on Jobs), which frees `f` from meaning two
                // unrelated things depending on which tab is open.
                (KeyCode::Char(' '), _) => {
                    self.toggle_favorite();
                    self.recompute_flat_rows();
                    self.recompute_tree();
                }
                (KeyCode::Char('v'), KeyModifiers::NONE) => {
                    self.cycle_view_forward();
                    self.recompute_flat_rows();
                    self.recompute_tree();
                    self.save_config();
                }
                (KeyCode::Char('b'), KeyModifiers::NONE) => {
                    self.open_dir_picker();
                }
                (KeyCode::Char('m'), KeyModifiers::NONE) => {
                    self.job_form_from_selection();
                }
                (KeyCode::Char('l'), KeyModifiers::NONE) => {
                    self.live_filter = !self.live_filter;
                    self.recompute_flat_rows();
                    self.recompute_tree();
                    self.save_config();
                }
                (KeyCode::Char('s'), KeyModifiers::NONE) => {
                    self.source_filter = self.source_filter.cycle();
                    self.recompute_filter();
                    self.preview_scroll = u16::MAX;
                    self.preview_auto_scroll = true;
                    self.save_config();
                }
                // Confirmed, to match `x` on the Jobs tab. Stopping a session
                // is not recoverable, so both tabs ask first.
                (KeyCode::Char('x'), KeyModifiers::NONE) => {
                    if let Some(idx) = self.selected_live_index() {
                        self.stop_confirm_name = Some(self.live_sessions[idx].display_name.clone());
                        self.mode = AppMode::StopSessionConfirm;
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
                        return;
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
                        // Cursor titles are changed with `/rename` inside the agent;
                        // writing Claude-shaped JSONL into store.db would corrupt it.
                        if session.backend == AgentBackend::CursorAgent {
                            self.status_error = Some(
                                "Cursor chat titles can't be renamed here — use /rename inside the agent"
                                    .to_string(),
                            );
                            return;
                        }
                        self.rename_session_id = Some(session.session_id.clone());
                        self.rename_project = Some(session.project.clone());
                        // Pre-fill with the chain's effective name (may come from any member)
                        self.rename_input = tui_input::Input::from(
                            self.chain_name_for(idx).unwrap_or("").to_string()
                        );
                        self.mode = AppMode::Renaming;
                    }
                }
                // Shift+N/D/W used to be three separate launch keys. They are
                // now modes cycled with Tab inside the naming popup, so `n` is
                // the only entry point and the status bar carries one "new"
                // hint instead of four.
                (KeyCode::Enter, _) if (key.modifiers.contains(KeyModifiers::SHIFT) || prev_shift_active) && self.is_historical_selected() => {
                    // Shift+Enter: open historical session directly (no tmux)
                    if self.tree_view {
                        if let Some(TreeRow::Session { session_index }) =
                            self.tree_rows.get(self.selected).cloned()
                        {
                            let session_id = self.resume_session_id_for(session_index).to_string();
                            let cwd = self.sessions[session_index].project.clone();
                            let backend = self.sessions[session_index].backend;
                            if self.ensure_backend_available(backend) {
                                self.launch_session =
                                    Some(LaunchRequest::Direct { session_id, cwd, backend });
                            }
                        }
                    } else if let Some(FlatRow::HistoryItem { session_index }) =
                        self.flat_rows.get(self.selected).cloned()
                    {
                        let session_id = self.resume_session_id_for(session_index).to_string();
                        let cwd = self.sessions[session_index].project.clone();
                        let backend = self.sessions[session_index].backend;
                        if self.ensure_backend_available(backend) {
                            self.launch_session =
                                Some(LaunchRequest::Direct { session_id, cwd, backend });
                        }
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
                                let backend = self.sessions[session_index].backend;
                                if self.ensure_backend_available(backend) {
                                    self.launch_session =
                                        Some(LaunchRequest::Resume { session_id, cwd, backend });
                                }
                            }
                            Some(TreeRow::LiveItem { live_index }) => {
                                let name = self.live_sessions[live_index].tmux_name.clone();
                                self.request_attach(name);
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
                                self.request_attach(name);
                            }
                            Some(FlatRow::HistoryItem { session_index }) => {
                                let session_id = self.resume_session_id_for(session_index).to_string();
                                let cwd = self.sessions[session_index].project.clone();
                                let backend = self.sessions[session_index].backend;
                                if self.ensure_backend_available(backend) {
                                    self.launch_session =
                                        Some(LaunchRequest::Resume { session_id, cwd, backend });
                                }
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
                                    self.preview_auto_scroll = true;
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
                                    self.preview_auto_scroll = true;
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

}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::event::KeyEvent;

    /// A key as the enhanced protocol reports it: the unshifted char plus SHIFT.
    fn shifted(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::SHIFT)
    }

    #[test]
    fn shift_and_a_digit_types_its_symbol() {
        // The reported bug: Shift+2 landed in the field as "2".
        assert_eq!(normalize_key(shifted('2')).code, KeyCode::Char('@'));
    }

    #[test]
    fn every_shifted_key_on_the_number_row_is_covered() {
        let pairs = [
            ('1', '!'),
            ('2', '@'),
            ('3', '#'),
            ('4', '$'),
            ('5', '%'),
            ('6', '^'),
            ('7', '&'),
            ('8', '*'),
            ('9', '('),
            ('0', ')'),
            ('-', '_'),
            ('=', '+'),
        ];
        for (base, expected) in pairs {
            assert_eq!(
                normalize_key(shifted(base)).code,
                KeyCode::Char(expected),
                "Shift+{base} should type {expected}"
            );
        }
    }

    #[test]
    fn every_shifted_punctuation_key_is_covered() {
        let pairs = [
            ('[', '{'),
            (']', '}'),
            ('\\', '|'),
            (';', ':'),
            ('\'', '"'),
            (',', '<'),
            ('.', '>'),
            ('/', '?'),
            ('`', '~'),
        ];
        for (base, expected) in pairs {
            assert_eq!(
                normalize_key(shifted(base)).code,
                KeyCode::Char(expected),
                "Shift+{base} should type {expected}"
            );
        }
    }

    #[test]
    fn shift_and_a_letter_still_uppercases() {
        assert_eq!(normalize_key(shifted('a')).code, KeyCode::Char('A'));
        assert_eq!(normalize_key(shifted('z')).code, KeyCode::Char('Z'));
    }

    #[test]
    fn a_key_without_shift_is_untouched() {
        for c in ['2', 'a', '/', '-'] {
            let key = KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE);
            assert_eq!(normalize_key(key).code, KeyCode::Char(c));
        }
    }

    #[test]
    fn an_already_shifted_char_is_not_shifted_twice() {
        // A terminal that resolved the layout itself sends the final character;
        // remapping it again would turn a typed "@" into something else.
        for c in ['@', '!', 'A', '?'] {
            assert_eq!(normalize_key(shifted(c)).code, KeyCode::Char(c));
        }
    }

    #[test]
    fn a_non_char_key_is_untouched() {
        let key = KeyEvent::new(KeyCode::Enter, KeyModifiers::SHIFT);
        assert_eq!(normalize_key(key).code, KeyCode::Enter);
    }

    #[test]
    fn normalizing_preserves_the_modifiers_and_kind() {
        let key = normalize_key(shifted('2'));
        assert!(key.modifiers.contains(KeyModifiers::SHIFT));
        assert_eq!(key.kind, KeyEventKind::Press);
    }
}
