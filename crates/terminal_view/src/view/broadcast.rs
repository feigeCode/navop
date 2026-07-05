use std::collections::HashMap;

use gpui::{App, WeakEntity};

use crate::broadcast_input::{BroadcastClientId, BroadcastInputHub};

use super::TerminalView;

#[derive(Default)]
pub(super) struct BroadcastInputRegistry {
    hub: BroadcastInputHub,
    clients: HashMap<BroadcastClientId, WeakEntity<TerminalView>>,
}

impl gpui::Global for BroadcastInputRegistry {}

impl BroadcastInputRegistry {
    pub(super) fn register(
        &mut self,
        connection_id: i64,
        view: WeakEntity<TerminalView>,
    ) -> BroadcastClientId {
        let id = self.hub.register(connection_id);
        self.clients.insert(id, view);
        id
    }

    pub(super) fn unregister(&mut self, id: BroadcastClientId) {
        self.hub.unregister(id);
        self.clients.remove(&id);
    }

    pub(super) fn set_enabled(&mut self, id: BroadcastClientId, enabled: bool) {
        self.hub.set_enabled(id, enabled);
    }

    pub(super) fn is_enabled(&self, id: BroadcastClientId) -> bool {
        self.hub.is_enabled(id)
    }

    pub(super) fn deliveries_from(
        &self,
        source: BroadcastClientId,
        data: &[u8],
    ) -> Vec<(WeakEntity<TerminalView>, Vec<u8>)> {
        self.hub
            .deliveries_from(source, data)
            .into_iter()
            .filter_map(|delivery| {
                self.clients
                    .get(&delivery.target)
                    .cloned()
                    .map(|view| (view, delivery.data))
            })
            .collect()
    }
}

pub(super) fn init_broadcast_input_registry(cx: &mut App) {
    if cx.try_global::<BroadcastInputRegistry>().is_none() {
        cx.set_global(BroadcastInputRegistry::default());
    }
}
