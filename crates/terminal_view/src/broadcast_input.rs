use std::collections::{BTreeMap, BTreeSet};

pub type BroadcastClientId = u64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastTarget {
    pub id: BroadcastClientId,
    pub label: String,
    pub selected: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastInputSnapshot {
    pub enabled: bool,
    pub targets: Vec<BroadcastTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastDelivery {
    pub target: BroadcastClientId,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct BroadcastInputHub {
    next_client_id: BroadcastClientId,
    clients: BTreeMap<BroadcastClientId, String>,
    selected: BTreeSet<BroadcastClientId>,
    enabled: bool,
}

impl BroadcastInputHub {
    pub fn register(&mut self, label: impl Into<String>) -> BroadcastClientId {
        self.next_client_id = self.next_client_id.saturating_add(1).max(1);
        let id = self.next_client_id;
        self.clients.insert(id, label.into());
        id
    }

    pub fn unregister(&mut self, id: BroadcastClientId) -> bool {
        self.selected.remove(&id);
        self.clients.remove(&id).is_some()
    }

    pub fn set_enabled(&mut self, enabled: bool) -> bool {
        let changed = self.enabled != enabled;
        self.enabled = enabled;
        changed
    }

    pub fn set_selected(&mut self, id: BroadcastClientId, selected: bool) -> bool {
        if !self.clients.contains_key(&id) {
            return false;
        }
        if selected {
            self.selected.insert(id)
        } else {
            self.selected.remove(&id)
        }
    }

    pub fn toggle_selected(&mut self, id: BroadcastClientId) -> bool {
        if !self.clients.contains_key(&id) {
            return false;
        }
        if !self.selected.remove(&id) {
            self.selected.insert(id);
        }
        true
    }

    pub fn select_all(&mut self) -> bool {
        let previous = self.selected.len();
        self.selected.extend(self.clients.keys().copied());
        self.selected.len() != previous
    }

    pub fn clear_selection(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        self.selected.clear();
        true
    }

    pub fn selected_count(&self) -> usize {
        self.selected.len()
    }

    pub fn all_selected(&self) -> bool {
        !self.clients.is_empty() && self.selected.len() == self.clients.len()
    }

    pub fn snapshot(&self) -> BroadcastInputSnapshot {
        let targets = self
            .clients
            .iter()
            .map(|(id, label)| BroadcastTarget {
                id: *id,
                label: label.clone(),
                selected: self.selected.contains(id),
            })
            .collect();
        BroadcastInputSnapshot {
            enabled: self.enabled,
            targets,
        }
    }

    pub fn deliveries_from(
        &self,
        source: BroadcastClientId,
        data: &[u8],
    ) -> Vec<BroadcastDelivery> {
        if data.is_empty() || !self.enabled || !self.clients.contains_key(&source) {
            return Vec::new();
        }

        self.selected
            .iter()
            .filter(|target| **target != source && self.clients.contains_key(target))
            .map(|target| BroadcastDelivery {
                target: *target,
                data: data.to_vec(),
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcasts_input_to_selected_targets_across_connections() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register("source");
        let selected = hub.register("selected");
        let unselected = hub.register("unselected");
        hub.set_enabled(true);
        hub.set_selected(selected, true);

        let deliveries = hub.deliveries_from(source, b"uptime\n");

        assert_eq!(
            vec![BroadcastDelivery {
                target: selected,
                data: b"uptime\n".to_vec(),
            }],
            deliveries
        );
        assert_ne!(selected, unselected);
    }

    #[test]
    fn skips_broadcast_when_broadcasting_is_disabled() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register("source");
        let target = hub.register("target");
        hub.set_selected(target, true);

        assert!(hub.deliveries_from(source, b"ls\n").is_empty());
    }

    #[test]
    fn source_is_never_delivered_back_to_itself() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register("source");
        hub.set_enabled(true);
        hub.set_selected(source, true);

        assert!(hub.deliveries_from(source, b"pwd\n").is_empty());
    }

    #[test]
    fn unregister_removes_target_and_selection() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register("source");
        let target = hub.register("target");
        hub.set_enabled(true);
        hub.set_selected(target, true);

        hub.unregister(target);

        assert!(hub.deliveries_from(source, b"pwd\n").is_empty());
        assert_eq!(
            vec![BroadcastTarget {
                id: source,
                label: "source".to_string(),
                selected: false,
            }],
            hub.snapshot().targets
        );
    }

    #[test]
    fn select_all_and_clear_selection_update_the_shared_snapshot() {
        let mut hub = BroadcastInputHub::default();
        let first = hub.register("first");
        let second = hub.register("second");

        hub.select_all();

        assert!(hub.all_selected());
        assert_eq!(2, hub.selected_count());
        assert_eq!(
            vec![(first, true), (second, true)],
            hub.snapshot()
                .targets
                .into_iter()
                .map(|target| (target.id, target.selected))
                .collect::<Vec<_>>()
        );

        hub.clear_selection();

        assert!(!hub.all_selected());
        assert_eq!(0, hub.selected_count());
    }

    #[test]
    fn toggle_selected_uses_the_latest_shared_state() {
        let mut hub = BroadcastInputHub::default();
        let target = hub.register("target");

        hub.toggle_selected(target);
        assert_eq!(1, hub.selected_count());

        hub.toggle_selected(target);
        assert_eq!(0, hub.selected_count());
    }
}
