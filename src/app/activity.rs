use super::*;
use std::collections::HashSet;

impl App {
    /// Returns `(active_count, idle_count, waiting_count)` across all live sessions.
    pub fn total_activity_counts(&self) -> (usize, usize, usize) {
        let mut active = 0;
        let mut idle = 0;
        let mut waiting = 0;
        for ls in &self.live_sessions {
            match self.activity_states.get(&ls.tmux_name) {
                Some(ActivityState::Idle) => idle += 1,
                Some(ActivityState::Waiting) => waiting += 1,
                _ => active += 1,
            }
        }
        (active, idle, waiting)
    }

    /// Returns `(active_count, idle_count, waiting_count)` for live sessions in the given project.
    /// Only sessions with a confirmed `Idle` state count as idle; `Unknown` and
    /// not-yet-polled sessions count as active (matching individual dot behavior).
    pub fn project_activity_counts(&self, project: &str) -> (usize, usize, usize) {
        let mut active = 0;
        let mut idle = 0;
        let mut waiting = 0;
        for ls in self.live_sessions.iter().filter(|ls| ls.cwd == project) {
            match self.activity_states.get(&ls.tmux_name) {
                Some(ActivityState::Idle) => idle += 1,
                Some(ActivityState::Waiting) => waiting += 1,
                _ => active += 1,
            }
        }
        (active, idle, waiting)
    }

    /// Re-query the ccsm tmux server for live sessions, clear the live preview cache,
    /// and rebuild both flat and tree views.
    pub fn reload_live_sessions(&mut self) {
        self.live_sessions = live::discover_live_sessions(self.config.tmux_bin());
        self.live_preview_cache.clear();
        // Prune stale entries from activity maps
        let valid_names: HashSet<String> = self.live_sessions.iter().map(|ls| ls.tmux_name.clone()).collect();
        self.activity_states.retain(|k, _| valid_names.contains(k));
        self.activity_last_poll.retain(|k, _| valid_names.contains(k));
        self.recompute_flat_rows();
        self.recompute_tree();
    }

    /// Poll all live sessions for activity state, throttled to every ~3 seconds per session.
    /// For the currently selected session, reuses the preview cache content.
    /// Returns true if any session's activity state changed.
    pub fn poll_all_activity(&mut self) -> bool {
        let now = Instant::now();
        let selected_name = self.selected_live_index()
            .map(|i| self.live_sessions[i].tmux_name.clone());
        let mut changed = false;

        for i in 0..self.live_sessions.len() {
            let name = self.live_sessions[i].tmux_name.clone();

            // Throttle: skip if polled less than 3 seconds ago
            if let Some(last) = self.activity_last_poll.get(&name) {
                if now.duration_since(*last).as_secs() < 3 {
                    continue;
                }
            }

            let content = if selected_name.as_deref() == Some(&name) {
                // Reuse preview cache for selected session
                self.live_preview_cache.get(&name).map(|(s, _)| s.clone()).unwrap_or_default()
            } else {
                // 50 lines gives enough context for reliable activity detection
                // (status indicators can be preceded by multi-line tool output)
                live::poll_pane_tail(self.config.tmux_bin(), &name, 50)
            };

            let state = live::detect_activity(&content);
            let prev = self.activity_states.insert(name.clone(), state);
            if prev != Some(state) {
                changed = true;
            }
            self.activity_last_poll.insert(name, now);
        }
        changed
    }
}
