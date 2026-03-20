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

        let chain_indices: Option<Vec<usize>> = self.chain_map.get(&idx).cloned();
        let cache_key = match &chain_indices {
            Some(_) => self.sessions[idx]
                .slug
                .clone()
                .unwrap_or_else(|| self.sessions[idx].session_id.clone()),
            None => self.sessions[idx].session_id.clone(),
        };

        if !self.preview_cache.contains_key(&cache_key) {
            let result = if let Some(ref indices) = chain_indices {
                let chain_sessions: Vec<&SessionInfo> =
                    indices.iter().map(|&i| &self.sessions[i]).collect();
                data::load_chain_preview(&chain_sessions)
            } else {
                let project = self.sessions[idx].project.clone();
                let session_id = self.sessions[idx].session_id.clone();
                data::load_preview(&project, &session_id)
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
