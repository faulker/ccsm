use super::*;
use std::collections::HashSet;

impl App {
    /// Collapse all project groups and build the initial tree row list.
    pub(super) fn init_tree(&mut self) {
        for session in &self.sessions {
            self.collapsed.insert(session.project.clone());
            self.collapsed.insert(format!("history:{}", session.project));
        }
        self.recompute_tree();
    }

    /// Rebuild `self.tree_rows` from the current `filtered_indices`, `live_sessions`,
    /// `collapsed` set, and `live_filter` flag.
    pub(crate) fn recompute_tree(&mut self) {
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

        // Collect projects that have live sessions (but may not have history)
        let mut live_only_projects: Vec<String> = self
            .live_sessions
            .iter()
            .map(|ls| ls.cwd.clone())
            .filter(|cwd| !group_map.contains_key(cwd.as_str()))
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        live_only_projects.sort();

        // Sort groups: favorites first, then by most-recent session (highest last_timestamp)
        groups.sort_by(|a, b| {
            let fav_a = self.favorites.contains(&a.0);
            let fav_b = self.favorites.contains(&b.0);
            if fav_a != fav_b {
                return fav_b.cmp(&fav_a);
            }
            let max_a = a.2.iter().map(|&i| self.sessions[i].last_timestamp).max().unwrap_or(0);
            let max_b = b.2.iter().map(|&i| self.sessions[i].last_timestamp).max().unwrap_or(0);
            max_b.cmp(&max_a)
        });

        self.tree_rows.clear();

        // Add live-only projects first (no history sessions)
        for project in live_only_projects {
            let project_name = std::path::Path::new(&project)
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| project.clone());
            let live_indices: Vec<usize> = self
                .live_sessions
                .iter()
                .enumerate()
                .filter(|(_, ls)| ls.cwd == project)
                .map(|(i, _)| i)
                .collect();
            let is_collapsed = self.collapsed.contains(&project);
            self.tree_rows.push(TreeRow::Header {
                project: project.clone(),
                project_name,
                session_count: live_indices.len(),
            });
            if !is_collapsed {
                let running_key = format!("running:{}", project);
                let running_collapsed = self.collapsed.contains(&running_key);
                self.tree_rows.push(TreeRow::RunningHeader {
                    project: project.clone(),
                    count: live_indices.len(),
                });
                if !running_collapsed {
                    for live_index in live_indices {
                        self.tree_rows.push(TreeRow::LiveItem { live_index });
                    }
                }
            }
        }

        let has_any_favorites = groups.iter().any(|(p, _, _)| self.favorites.contains(p));
        let mut separator_inserted = !has_any_favorites;

        for (project, project_name, indices) in groups {
            let live_indices: Vec<usize> = self
                .live_sessions
                .iter()
                .enumerate()
                .filter(|(_, ls)| ls.cwd == project)
                .map(|(i, _)| i)
                .collect();
            let has_running = !live_indices.is_empty();
            let has_history = !indices.is_empty();

            // When live_filter is active, skip projects with no live sessions
            if self.live_filter && !has_running {
                continue;
            }

            // Insert separator between favorites and non-favorites
            if !separator_inserted && !self.favorites.contains(&project) {
                self.tree_rows.push(TreeRow::FavoritesSeparator);
                separator_inserted = true;
            }

            let is_collapsed = self.collapsed.contains(&project);
            self.tree_rows.push(TreeRow::Header {
                project: project.clone(),
                project_name,
                session_count: indices.len() + live_indices.len(),
            });

            if !is_collapsed {
                if has_running {
                    let running_key = format!("running:{}", project);
                    let running_collapsed = self.collapsed.contains(&running_key);
                    self.tree_rows.push(TreeRow::RunningHeader {
                        project: project.clone(),
                        count: live_indices.len(),
                    });
                    if !running_collapsed {
                        for live_index in live_indices {
                            self.tree_rows.push(TreeRow::LiveItem { live_index });
                        }
                    }
                }
                if has_history && !self.live_filter {
                    let history_key = format!("history:{}", project);
                    let history_collapsed = self.collapsed.contains(&history_key);
                    self.tree_rows.push(TreeRow::HistoryHeader {
                        project: project.clone(),
                        count: indices.len(),
                    });
                    if !history_collapsed {
                        for idx in indices {
                            self.tree_rows.push(TreeRow::Session { session_index: idx });
                        }
                    }
                }
            }
        }
    }
}
