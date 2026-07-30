use super::*;

impl App {
    /// Return the preview data for the currently selected session, loading and caching it on
    /// first access. Returns empty slices when no session is selected or a live item is selected.
    pub fn current_preview(&mut self) -> (&SessionMeta, &[PreviewMessage]) {
        static EMPTY_META: std::sync::OnceLock<SessionMeta> = std::sync::OnceLock::new();

        let idx = match self.selected_session_index() {
            Some(i) => i,
            None => return (EMPTY_META.get_or_init(SessionMeta::default), &[]),
        };

        let backend = self.sessions[idx].backend;
        // Prefix the cache key with the backend so ids cannot collide across stores.
        let chain_indices: Option<Vec<usize>> = if backend == AgentBackend::ClaudeCode {
            self.chain_map.get(&idx).cloned()
        } else {
            // Cursor has no chains; never enter load_chain_preview.
            None
        };
        let id_key = match &chain_indices {
            Some(_) => self.sessions[idx]
                .slug
                .clone()
                .unwrap_or_else(|| self.sessions[idx].session_id.clone()),
            None => self.sessions[idx].session_id.clone(),
        };
        let cache_key = match backend {
            AgentBackend::ClaudeCode => format!("claude:{id_key}"),
            AgentBackend::CursorAgent => format!("cursor:{id_key}"),
        };

        if !self.preview_cache.contains_key(&cache_key) {
            let result = match backend {
                AgentBackend::CursorAgent => {
                    let project = self.sessions[idx].project.clone();
                    let session_id = self.sessions[idx].session_id.clone();
                    data::load_cursor_preview(&project, &session_id)
                }
                AgentBackend::ClaudeCode => {
                    if let Some(ref indices) = chain_indices {
                        let chain_sessions: Vec<&SessionInfo> =
                            indices.iter().map(|&i| &self.sessions[i]).collect();
                        data::load_chain_preview(&chain_sessions)
                    } else {
                        let project = self.sessions[idx].project.clone();
                        let session_id = self.sessions[idx].session_id.clone();
                        data::load_preview(&project, &session_id)
                    }
                }
            };
            self.preview_cache.insert(cache_key.clone(), result);
        }

        let session = &self.sessions[idx];
        let (meta, messages) = self.preview_cache.get_mut(&cache_key).unwrap();
        // For single sessions, keep meta in sync with live session data
        if chain_indices.is_none() {
            meta.session_id = Some(session.session_id.clone());
            meta.session_name = session.name.clone();
        }
        (meta, messages)
    }

    /// Return the most recently captured pane output (with ANSI codes) for the selected live session,
    /// refreshing from tmux at most once every second.
    pub fn current_live_preview(&mut self) -> String {
        let idx = match self.selected_live_index() {
            Some(i) => i,
            None => return String::new(),
        };
        let name = self.live_sessions[idx].tmux_name.clone();
        let now = Instant::now();
        let should_refresh = self.live_preview_cache.get(&name)
            .map(|(_, last)| now.duration_since(*last).as_secs() >= 1)
            .unwrap_or(true);
        if should_refresh {
            let output = live::poll_pane_buffer(self.config.tmux_bin(), &name, 100);
            self.live_preview_cache.insert(name.clone(), (output, now));
        }
        self.live_preview_cache.get(&name).map(|(s, _)| s.clone()).unwrap_or_default()
    }
}
