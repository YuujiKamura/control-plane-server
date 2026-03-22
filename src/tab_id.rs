use std::collections::HashMap;

/// Manages stable tab IDs that persist across index shifts caused by tab
/// close/reorder operations.  The Rust CP server owns this mapping
/// independently of the Zig side.
pub struct TabIdManager {
    next_id: u64,
    /// stable-id  →  current positional index
    id_to_index: HashMap<String, usize>,
    /// current positional index  →  stable-id
    index_to_id: HashMap<usize, String>,
}

impl TabIdManager {
    pub fn new() -> Self {
        Self {
            next_id: 0,
            id_to_index: HashMap::new(),
            index_to_id: HashMap::new(),
        }
    }

    /// Synchronise the manager with the actual tab count reported by the
    /// terminal provider.
    ///
    /// * New indices (≥ previous max) get a fresh stable ID.
    /// * Indices that disappeared (tab closed) are removed and higher
    ///   indices are shifted down.
    ///
    /// This is intentionally simple: it assumes tabs are only added at the
    /// end and removed at arbitrary positions (standard tab-bar behaviour).
    pub fn sync_tabs(&mut self, tab_count: usize) {
        let known_count = self.index_to_id.len();

        if tab_count > known_count {
            // New tabs were added at the end
            for i in known_count..tab_count {
                self.register_new_tab(i);
            }
        } else if tab_count < known_count {
            // Tabs were removed – we cannot know *which* index was removed
            // without external info, so we rebuild: keep the first
            // `tab_count` IDs in order and drop the rest.
            //
            // A more precise approach would require the caller to tell us
            // which index was removed (see `remove_tab_at_index`).  This
            // fallback keeps things working when sync is the only signal.
            let ordered: Vec<String> = (0..known_count)
                .filter_map(|i| self.index_to_id.get(&i).cloned())
                .collect();

            self.id_to_index.clear();
            self.index_to_id.clear();

            for (i, id) in ordered.into_iter().take(tab_count).enumerate() {
                self.id_to_index.insert(id.clone(), i);
                self.index_to_id.insert(i, id);
            }
        }
        // tab_count == known_count → nothing to do
    }

    /// Register a brand-new tab at `index` and return its stable ID.
    pub fn register_new_tab(&mut self, index: usize) -> String {
        let id = format!("t_{:03}", self.next_id);
        self.next_id += 1;
        self.id_to_index.insert(id.clone(), index);
        self.index_to_id.insert(index, id.clone());
        id
    }

    /// Resolve a stable tab ID to its current positional index.
    pub fn resolve(&self, id: &str) -> Option<usize> {
        self.id_to_index.get(id).copied()
    }

    /// Get the stable ID for a given positional index.
    pub fn get_id(&self, index: usize) -> Option<&str> {
        self.index_to_id.get(&index).map(|s| s.as_str())
    }

    /// Remove a tab by stable ID and shift all higher indices down by one.
    pub fn remove_tab(&mut self, id: &str) {
        if let Some(removed_idx) = self.id_to_index.remove(id) {
            self.index_to_id.remove(&removed_idx);
            self.shift_indices_down(removed_idx);
        }
    }

    /// Remove the tab at `index` (by looking up its ID first) and shift
    /// higher indices down.
    pub fn remove_tab_at_index(&mut self, index: usize) {
        if let Some(id) = self.index_to_id.remove(&index) {
            self.id_to_index.remove(&id);
            self.shift_indices_down(index);
        }
    }

    /// After removing `removed_idx`, every index > removed_idx must
    /// decrease by 1.
    fn shift_indices_down(&mut self, removed_idx: usize) {
        // Collect entries that need updating, sorted by ascending old index
        // so that we process lower indices first and avoid clobbering.
        let mut to_update: Vec<(String, usize)> = self
            .id_to_index
            .iter()
            .filter(|(_, &idx)| idx > removed_idx)
            .map(|(id, &idx)| (id.clone(), idx))
            .collect();
        to_update.sort_by_key(|(_, idx)| *idx);

        for (id, old_idx) in to_update {
            let new_idx = old_idx - 1;
            self.id_to_index.insert(id.clone(), new_idx);
            self.index_to_id.remove(&old_idx);
            self.index_to_id.insert(new_idx, id);
        }
    }

    /// Number of tracked tabs.
    pub fn len(&self) -> usize {
        self.index_to_id.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_register_and_resolve() {
        let mut mgr = TabIdManager::new();
        let id0 = mgr.register_new_tab(0);
        let id1 = mgr.register_new_tab(1);

        assert_eq!(id0, "t_000");
        assert_eq!(id1, "t_001");
        assert_eq!(mgr.resolve("t_000"), Some(0));
        assert_eq!(mgr.resolve("t_001"), Some(1));
        assert_eq!(mgr.get_id(0), Some("t_000"));
        assert_eq!(mgr.get_id(1), Some("t_001"));
    }

    #[test]
    fn test_remove_tab_shifts_indices() {
        let mut mgr = TabIdManager::new();
        mgr.register_new_tab(0); // t_000
        mgr.register_new_tab(1); // t_001
        mgr.register_new_tab(2); // t_002

        mgr.remove_tab("t_001");

        assert_eq!(mgr.resolve("t_000"), Some(0));
        assert_eq!(mgr.resolve("t_001"), None);
        // t_002 was at index 2, now shifted to 1
        assert_eq!(mgr.resolve("t_002"), Some(1));
        assert_eq!(mgr.len(), 2);
    }

    #[test]
    fn test_remove_tab_at_index() {
        let mut mgr = TabIdManager::new();
        mgr.register_new_tab(0); // t_000
        mgr.register_new_tab(1); // t_001
        mgr.register_new_tab(2); // t_002

        mgr.remove_tab_at_index(0);

        assert_eq!(mgr.resolve("t_000"), None);
        assert_eq!(mgr.resolve("t_001"), Some(0));
        assert_eq!(mgr.resolve("t_002"), Some(1));
        assert_eq!(mgr.len(), 2);
    }

    #[test]
    fn test_sync_tabs_grow() {
        let mut mgr = TabIdManager::new();
        mgr.sync_tabs(2);
        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.resolve("t_000"), Some(0));
        assert_eq!(mgr.resolve("t_001"), Some(1));

        mgr.sync_tabs(4);
        assert_eq!(mgr.len(), 4);
        assert_eq!(mgr.resolve("t_002"), Some(2));
        assert_eq!(mgr.resolve("t_003"), Some(3));
    }

    #[test]
    fn test_sync_tabs_shrink() {
        let mut mgr = TabIdManager::new();
        mgr.sync_tabs(3); // t_000, t_001, t_002
        assert_eq!(mgr.len(), 3);

        mgr.sync_tabs(2);
        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.resolve("t_000"), Some(0));
        assert_eq!(mgr.resolve("t_001"), Some(1));
        assert_eq!(mgr.resolve("t_002"), None);
    }

    #[test]
    fn test_sync_tabs_no_change() {
        let mut mgr = TabIdManager::new();
        mgr.sync_tabs(2);
        mgr.sync_tabs(2);
        assert_eq!(mgr.len(), 2);
    }
}
