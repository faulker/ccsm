use super::*;

impl App {
    /// Recompute `filtered_indices` from the current filter text, hide-empty flag, and chain
    /// grouping setting, then rebuild both tree and flat views and clamp the selection.
    pub(crate) fn recompute_filter(&mut self) {
        let query = self.filter_input.value().to_lowercase();
        let initial_indices: Vec<usize> = if query.is_empty() {
            (0..self.sessions.len())
                .filter(|&i| !self.hide_empty || self.sessions[i].has_data)
                .collect()
        } else {
            self.sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    (!self.hide_empty || s.has_data)
                        && (s.project_name.to_lowercase().contains(&query)
                            || s.project.to_lowercase().contains(&query))
                })
                .map(|(i, _)| i)
                .collect()
        };

        if self.group_chains {
            // Group indices by slug
            let mut slug_groups: HashMap<String, Vec<usize>> = HashMap::new();
            let mut slugless: Vec<usize> = Vec::new();

            for &idx in &initial_indices {
                if let Some(slug) = self.sessions[idx].slug.clone() {
                    slug_groups.entry(slug).or_default().push(idx);
                } else {
                    slugless.push(idx);
                }
            }

            self.chain_map.clear();

            let mut result_indices: Vec<usize> = slugless;

            for (_slug, mut indices) in slug_groups {
                if indices.len() == 1 {
                    // Single session with a slug — treat as standalone
                    result_indices.push(indices[0]);
                } else {
                    // Pick canonical = highest last_timestamp
                    let canonical = *indices
                        .iter()
                        .max_by_key(|&&i| self.sessions[i].last_timestamp)
                        .unwrap();
                    // Sort chain members oldest→newest
                    indices.sort_by_key(|&i| self.sessions[i].first_timestamp);
                    self.chain_map.insert(canonical, indices);
                    result_indices.push(canonical);
                }
            }

            // Sort all results by last_timestamp descending
            result_indices
                .sort_by(|&a, &b| self.sessions[b].last_timestamp.cmp(&self.sessions[a].last_timestamp));
            self.filtered_indices = result_indices;
        } else {
            self.chain_map.clear();
            self.filtered_indices = initial_indices;
        }

        if self.tree_view {
            self.recompute_tree();
        }
        self.recompute_flat_rows();
        if self.selected >= self.visible_item_count() {
            self.selected = self.visible_item_count().saturating_sub(1);
        }
    }
}
