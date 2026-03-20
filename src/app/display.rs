use super::*;

impl App {
    /// Returns the display name for a session based on the display mode.
    pub fn display_name(&self, session: &SessionInfo) -> String {
        match self.display_mode {
            DisplayMode::Name => session.project_name.clone(),
            DisplayMode::ShortDir => truncate_path(&session.project),
            DisplayMode::FullDir => session.project.clone(),
        }
    }

    /// Cycle view mode forward: tree[Name] → tree[ShortDir] → tree[FullDir]
    /// → flat[Name] → flat[ShortDir] → flat[FullDir] → tree[Name].
    pub fn cycle_view_forward(&mut self) {
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
    }

    /// Cycle view mode backward (reverse of `cycle_view_forward`).
    pub fn cycle_view_backward(&mut self) {
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
    }
}
