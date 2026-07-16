use std::collections::HashMap;
use std::sync::Arc;

use gpui::{App, AppContext as _, Context, Focusable as _, WeakEntity, Window};
use one_core::tab_container::{ExternalTabDragSource, TabContentEvent, TabItem, TabOpenMode};
use uuid::Uuid;

use super::{TerminalPaneId, TerminalWorkspace};

#[derive(Clone)]
pub(super) struct TerminalPaneTabMetadata {
    id: String,
    from: String,
    metadata: HashMap<String, String>,
}

impl TerminalPaneTabMetadata {
    pub(super) fn generated() -> Self {
        Self {
            id: format!("terminal-pane-{}", Uuid::new_v4()),
            from: "terminal".to_string(),
            metadata: HashMap::new(),
        }
    }

    pub(super) fn from_tab(tab: &TabItem) -> Self {
        Self {
            id: tab.id().to_string(),
            from: tab.from().to_string(),
            metadata: tab.metadata().clone(),
        }
    }
}

struct TerminalPaneExternalTabSource {
    workspace: WeakEntity<TerminalWorkspace>,
    pane_id: TerminalPaneId,
}

impl ExternalTabDragSource for TerminalPaneExternalTabSource {
    fn take_tab(&self, window: &mut Window, cx: &mut App) -> Option<TabItem> {
        self.workspace
            .update(cx, |workspace, cx| {
                workspace.detach_pane_as_tab(self.pane_id, window, cx)
            })
            .ok()
            .flatten()
    }
}

impl TerminalWorkspace {
    pub(super) fn external_tab_drag_source(
        &self,
        pane_id: TerminalPaneId,
        workspace: WeakEntity<Self>,
    ) -> Option<Arc<dyn ExternalTabDragSource>> {
        self.pane_tab_metadata.contains_key(&pane_id).then(|| {
            Arc::new(TerminalPaneExternalTabSource { workspace, pane_id })
                as Arc<dyn ExternalTabDragSource>
        })
    }

    pub(super) fn restore_pane_to_tab(
        &mut self,
        pane_id: TerminalPaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.panes.len() <= 1 {
            return;
        }
        let Some(tab) = self.detach_pane_as_tab(pane_id, window, cx) else {
            return;
        };
        self.active_pane()
            .read(cx)
            .focus_handle(cx)
            .focus(window, cx);
        cx.emit(TabContentEvent::OpenTab {
            tab,
            mode: TabOpenMode::Background,
        });
    }

    fn detach_pane_as_tab(
        &mut self,
        pane_id: TerminalPaneId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<TabItem> {
        let pane = self.panes.get(&pane_id)?.clone();
        let metadata = self.pane_tab_metadata.get(&pane_id)?.clone();
        let fallback = self.split_tree.remove(pane_id)?;

        self.panes.remove(&pane_id);
        self.pane_tab_metadata.remove(&pane_id);
        self.pane_subscriptions.remove(&pane_id);
        if self.active_pane_id == pane_id {
            self.active_pane_id = fallback;
        }
        let workspace = cx.new(|cx| TerminalWorkspace::from_pane(pane, window, cx));
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
        Some(TabItem::new(metadata.id, metadata.from, workspace).with_metadata(metadata.metadata))
    }
}
