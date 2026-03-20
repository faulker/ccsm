use super::*;

impl App {
    /// Returns the effective display name for a session entry.
    /// For chains, returns the name of the most recently active member that has one,
    /// so the list matches what the details panel shows.
    pub fn chain_name_for(&self, idx: usize) -> Option<&str> {
        if let Some(chain) = self.chain_map.get(&idx) {
            chain
                .iter()
                .filter(|&&i| self.sessions[i].name.is_some())
                .max_by_key(|&&i| self.sessions[i].last_timestamp)
                .and_then(|&i| self.sessions[i].name.as_deref())
        } else {
            self.sessions[idx].name.as_deref()
        }
    }

    /// Returns the session_id to use when resuming, always the chain member with the
    /// highest last_timestamp (i.e. the most recently active session).
    pub fn resume_session_id_for(&self, idx: usize) -> &str {
        if let Some(chain) = self.chain_map.get(&idx) {
            let best = chain
                .iter()
                .max_by_key(|&&i| self.sessions[i].last_timestamp)
                .copied()
                .unwrap_or(idx);
            &self.sessions[best].session_id
        } else {
            &self.sessions[idx].session_id
        }
    }

    /// Returns the total entry count for a session, summing across all sessions in its chain.
    pub fn chain_entry_count(&self, canonical_idx: usize) -> usize {
        if let Some(indices) = self.chain_map.get(&canonical_idx) {
            indices.iter().map(|&i| self.sessions[i].entry_count).sum()
        } else {
            self.sessions[canonical_idx].entry_count
        }
    }
}
