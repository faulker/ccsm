use super::*;

impl App {
    /// Returns the number of rows in the currently active view (tree or flat).
    pub fn visible_item_count(&self) -> usize {
        if self.tree_view {
            self.tree_rows.len()
        } else {
            self.flat_rows.len()
        }
    }

    /// Get the raw session index for the currently selected item.
    /// Returns None for headers in tree mode, live items, separators, or when no item is selected.
    pub fn selected_session_index(&self) -> Option<usize> {
        if self.tree_view {
            match self.tree_rows.get(self.selected) {
                Some(TreeRow::Session { session_index }) => Some(*session_index),
                _ => None,
            }
        } else {
            match self.flat_rows.get(self.selected) {
                Some(FlatRow::HistoryItem { session_index }) => Some(*session_index),
                _ => None,
            }
        }
    }

    /// Returns true if the currently selected item is a historical (non-live) session.
    pub fn is_historical_selected(&self) -> bool {
        if self.tree_view {
            matches!(self.tree_rows.get(self.selected), Some(TreeRow::Session { .. }))
        } else {
            matches!(self.flat_rows.get(self.selected), Some(FlatRow::HistoryItem { .. }))
        }
    }

    /// Get the live session index for the currently selected item.
    /// Returns None if the selection is not a live session.
    pub fn selected_live_index(&self) -> Option<usize> {
        if self.tree_view {
            match self.tree_rows.get(self.selected) {
                Some(TreeRow::LiveItem { live_index }) => Some(*live_index),
                _ => None,
            }
        } else {
            match self.flat_rows.get(self.selected) {
                Some(FlatRow::LiveItem { live_index }) => Some(*live_index),
                _ => None,
            }
        }
    }

    /// Get the CWD for the currently selected session (or header group).
    pub fn selected_cwd(&self) -> Option<String> {
        if self.tree_view {
            match self.tree_rows.get(self.selected) {
                Some(TreeRow::Session { session_index }) => {
                    Some(self.sessions[*session_index].project.clone())
                }
                Some(TreeRow::Header { project, .. }) => Some(project.clone()),
                Some(TreeRow::LiveItem { live_index }) => {
                    Some(self.live_sessions[*live_index].cwd.clone())
                }
                Some(TreeRow::RunningHeader { project, .. }) => Some(project.clone()),
                Some(TreeRow::HistoryHeader { project, .. }) => Some(project.clone()),
                Some(TreeRow::FavoritesSeparator) | None => None,
            }
        } else {
            match self.flat_rows.get(self.selected) {
                Some(FlatRow::HistoryItem { session_index }) => {
                    Some(self.sessions[*session_index].project.clone())
                }
                Some(FlatRow::LiveItem { live_index }) => {
                    Some(self.live_sessions[*live_index].cwd.clone())
                }
                _ => None,
            }
        }
    }

    /// Toggle the favorite status of the currently selected project path and save config.
    pub fn toggle_favorite(&mut self) {
        if let Some(project) = self.selected_cwd() {
            if self.favorites.contains(&project) {
                self.favorites.remove(&project);
            } else {
                self.favorites.insert(project);
            }
            self.save_config();
        }
    }
}
