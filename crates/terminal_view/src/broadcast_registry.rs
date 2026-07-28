use std::collections::HashMap;

use gpui::{App, AppContext, Context, Entity, WeakEntity};

use crate::broadcast_input::{
    BroadcastClientId, BroadcastInputHub, BroadcastInputSnapshot, TerminalInputKind,
};
use crate::view::TerminalView;

#[derive(Default)]
pub(crate) struct BroadcastInputRegistry {
    hub: BroadcastInputHub,
    clients: HashMap<BroadcastClientId, WeakEntity<TerminalView>>,
}

#[derive(Clone)]
pub(crate) struct GlobalBroadcastInputRegistry(pub Entity<BroadcastInputRegistry>);

impl gpui::Global for GlobalBroadcastInputRegistry {}

impl BroadcastInputRegistry {
    pub(crate) fn register(
        &mut self,
        label: String,
        view: WeakEntity<TerminalView>,
        cx: &mut Context<Self>,
    ) -> BroadcastClientId {
        let id = self.hub.register(label);
        self.clients.insert(id, view);
        cx.notify();
        id
    }

    pub(crate) fn unregister(&mut self, id: BroadcastClientId, cx: &mut Context<Self>) {
        self.clients.remove(&id);
        if self.hub.unregister(id) {
            cx.notify();
        }
    }

    pub(crate) fn set_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.hub.set_enabled(enabled) {
            cx.notify();
        }
    }

    pub(crate) fn toggle_selected(&mut self, id: BroadcastClientId, cx: &mut Context<Self>) {
        if self.hub.toggle_selected(id) {
            cx.notify();
        }
    }

    pub(crate) fn toggle_all(&mut self, cx: &mut Context<Self>) {
        let changed = if self.hub.all_selected() {
            self.hub.clear_selection()
        } else {
            self.hub.select_all()
        };
        if changed {
            cx.notify();
        }
    }

    pub(crate) fn snapshot(&self) -> BroadcastInputSnapshot {
        self.hub.snapshot()
    }

    pub(crate) fn deliveries_from(
        &self,
        source: BroadcastClientId,
        kind: TerminalInputKind,
        data: &[u8],
    ) -> Vec<(WeakEntity<TerminalView>, TerminalInputKind, Vec<u8>)> {
        self.hub
            .deliveries_from(source, kind, data)
            .into_iter()
            .filter_map(|delivery| {
                self.clients
                    .get(&delivery.target)
                    .cloned()
                    .map(|view| (view, delivery.kind, delivery.data))
            })
            .collect()
    }
}

pub(crate) fn init_broadcast_input_registry(cx: &mut App) {
    if cx.try_global::<GlobalBroadcastInputRegistry>().is_none() {
        let registry = cx.new(|_| BroadcastInputRegistry::default());
        cx.set_global(GlobalBroadcastInputRegistry(registry));
    }
}

pub(crate) fn broadcast_input_registry(cx: &App) -> Option<Entity<BroadcastInputRegistry>> {
    cx.try_global::<GlobalBroadcastInputRegistry>()
        .map(|global| global.0.clone())
}
