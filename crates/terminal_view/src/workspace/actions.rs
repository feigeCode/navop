use gpui::{Context, Entity, Focusable as _, Window};
use gpui_component::Placement;
use one_core::tab_container::{TabContent as _, TabContentEvent};

use super::pane_tab_transfer::TerminalPaneTabMetadata;
use super::{TerminalPaneId, TerminalWorkspace};
use crate::view::{TerminalPaneEvent, TerminalView};

impl TerminalWorkspace {
    pub(super) fn subscribe_to_pane(
        &mut self,
        pane_id: TerminalPaneId,
        pane: Entity<TerminalView>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let focus_subscription = cx.subscribe_in(
            &pane,
            window,
            |this, pane, event: &TerminalPaneEvent, _window, cx| match event {
                TerminalPaneEvent::Focused => this.activate_entity(pane.clone(), cx),
            },
        );
        let content_subscription = cx.subscribe_in(
            &pane,
            window,
            |_this, _pane, event: &TabContentEvent, _window, cx| {
                cx.emit(event.clone());
                cx.notify();
            },
        );
        self.pane_subscriptions
            .insert(pane_id, vec![focus_subscription, content_subscription]);
    }

    pub(super) fn insert_pane(
        &mut self,
        target: TerminalPaneId,
        placement: Placement,
        pane: Entity<TerminalView>,
        tab_metadata: TerminalPaneTabMetadata,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let pane_id = TerminalPaneId::new(self.next_pane_id);
        if !self.split_tree.split(target, pane_id, placement) {
            return false;
        }

        self.next_pane_id += 1;
        self.panes.insert(pane_id, pane.clone());
        self.pane_tab_metadata.insert(pane_id, tab_metadata);
        self.subscribe_to_pane(pane_id, pane.clone(), window, cx);
        self.active_pane_id = pane_id;
        pane.read(cx).focus_handle(cx).focus(window, cx);
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
        true
    }

    pub(super) fn request_close_pane(
        &mut self,
        pane_id: TerminalPaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panes.len() <= 1 {
            return;
        }
        let Some(pane) = self.panes.get(&pane_id).cloned() else {
            return;
        };
        let close_task = pane.update(cx, |pane, cx| pane.try_close("", window, cx));
        let workspace = cx.entity().downgrade();
        cx.spawn(async move |_this, cx| {
            if !close_task.await {
                return;
            }
            let _ = workspace.update(cx, |workspace, cx| {
                workspace.finish_close_pane(pane_id, cx);
            });
        })
        .detach();
    }

    fn activate_entity(&mut self, pane: Entity<TerminalView>, cx: &mut Context<Self>) {
        let Some(pane_id) = self.pane_id_for_entity(&pane) else {
            return;
        };
        if self.active_pane_id != pane_id {
            self.active_pane_id = pane_id;
            cx.emit(TabContentEvent::StateChanged);
            cx.notify();
        }
    }

    fn pane_id_for_entity(&self, pane: &Entity<TerminalView>) -> Option<TerminalPaneId> {
        self.panes
            .iter()
            .find_map(|(pane_id, candidate)| (candidate == pane).then_some(*pane_id))
    }

    fn finish_close_pane(&mut self, pane_id: TerminalPaneId, cx: &mut Context<Self>) {
        let Some(fallback) = self.split_tree.remove(pane_id) else {
            return;
        };
        self.panes.remove(&pane_id);
        self.pane_tab_metadata.remove(&pane_id);
        self.pane_subscriptions.remove(&pane_id);
        if self.active_pane_id == pane_id {
            self.active_pane_id = fallback;
        }
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
    }
}
