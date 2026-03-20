use super::*;

impl App {
    /// Rebuild `self.flat_rows` from the current live sessions and filtered history indices.
    pub fn recompute_flat_rows(&mut self) {
        self.flat_rows.clear();
        let live_count = self.live_sessions.len();
        if self.live_filter {
            if live_count > 0 {
                self.flat_rows.push(FlatRow::RunningHeader { count: live_count });
                for i in 0..live_count {
                    self.flat_rows.push(FlatRow::LiveItem { live_index: i });
                }
            }
            return;
        }
        if live_count > 0 {
            self.flat_rows.push(FlatRow::RunningHeader { count: live_count });
            for i in 0..live_count {
                self.flat_rows.push(FlatRow::LiveItem { live_index: i });
            }
        }
        if live_count > 0 && !self.filtered_indices.is_empty() {
            self.flat_rows.push(FlatRow::Separator);
        }
        // Sort history items: favorites first (by project), then by recency
        let mut sorted_indices = self.filtered_indices.clone();
        sorted_indices.sort_by(|&a, &b| {
            let fav_a = self.favorites.contains(&self.sessions[a].project);
            let fav_b = self.favorites.contains(&self.sessions[b].project);
            if fav_a != fav_b {
                return fav_b.cmp(&fav_a);
            }
            self.sessions[b].last_timestamp.cmp(&self.sessions[a].last_timestamp)
        });
        let has_any_favorites = sorted_indices.iter().any(|&i| self.favorites.contains(&self.sessions[i].project));
        let mut separator_inserted = !has_any_favorites;
        for si in sorted_indices {
            if !separator_inserted && !self.favorites.contains(&self.sessions[si].project) {
                self.flat_rows.push(FlatRow::FavoritesSeparator);
                separator_inserted = true;
            }
            self.flat_rows.push(FlatRow::HistoryItem { session_index: si });
        }
    }
}
