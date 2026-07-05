use std::collections::{HashMap, HashSet};

pub type BroadcastClientId = u64;
pub type BroadcastGroupId = i64;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BroadcastDelivery {
    pub target: BroadcastClientId,
    pub data: Vec<u8>,
}

#[derive(Debug, Default)]
pub struct BroadcastInputHub {
    next_client_id: BroadcastClientId,
    clients: HashMap<BroadcastClientId, BroadcastGroupId>,
    enabled: HashSet<BroadcastClientId>,
}

impl BroadcastInputHub {
    pub fn register(&mut self, group: BroadcastGroupId) -> BroadcastClientId {
        self.next_client_id = self.next_client_id.saturating_add(1).max(1);
        let id = self.next_client_id;
        self.clients.insert(id, group);
        id
    }

    pub fn unregister(&mut self, id: BroadcastClientId) {
        self.clients.remove(&id);
        self.enabled.remove(&id);
    }

    pub fn set_enabled(&mut self, id: BroadcastClientId, enabled: bool) -> bool {
        if !self.clients.contains_key(&id) {
            return false;
        }
        if enabled {
            self.enabled.insert(id);
        } else {
            self.enabled.remove(&id);
        }
        true
    }

    pub fn is_enabled(&self, id: BroadcastClientId) -> bool {
        self.enabled.contains(&id)
    }

    pub fn deliveries_from(
        &self,
        source: BroadcastClientId,
        data: &[u8],
    ) -> Vec<BroadcastDelivery> {
        if data.is_empty() || !self.enabled.contains(&source) {
            return Vec::new();
        }

        let Some(group) = self.clients.get(&source) else {
            return Vec::new();
        };

        self.clients
            .iter()
            .filter_map(|(target, target_group)| {
                (*target != source && target_group == group).then(|| BroadcastDelivery {
                    target: *target,
                    data: data.to_vec(),
                })
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcasts_input_to_peers_in_same_group_only() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register(10);
        let peer = hub.register(10);
        let other_group = hub.register(20);
        hub.set_enabled(source, true);
        hub.set_enabled(other_group, true);

        let deliveries = hub.deliveries_from(source, b"uptime\n");

        assert_eq!(
            vec![BroadcastDelivery {
                target: peer,
                data: b"uptime\n".to_vec(),
            }],
            deliveries
        );
    }

    #[test]
    fn skips_broadcast_when_source_is_disabled() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register(10);
        hub.register(10);

        assert!(hub.deliveries_from(source, b"ls\n").is_empty());
    }

    #[test]
    fn unregister_removes_client_from_future_deliveries() {
        let mut hub = BroadcastInputHub::default();
        let source = hub.register(10);
        let peer = hub.register(10);
        hub.set_enabled(source, true);
        hub.unregister(peer);

        assert!(hub.deliveries_from(source, b"pwd\n").is_empty());
        assert!(!hub.is_enabled(peer));
    }
}
