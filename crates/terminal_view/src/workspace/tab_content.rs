use std::sync::Arc;

use gpui::{
    App, AppContext as _, Context, EventEmitter, FocusHandle, Focusable, SharedString, Task, Window,
};
use gpui_component::{Icon, IconName};
use one_core::sidebar_contribution::SidebarContribution;
use one_core::tab_container::{TabContent, TabContentEvent, TabContentView};
use terminal::terminal::TerminalConnectionKind;

use super::TerminalWorkspace;
use crate::view::TerminalView;

impl EventEmitter<TabContentEvent> for TerminalWorkspace {}

impl Focusable for TerminalWorkspace {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.active_pane().read(cx).focus_handle(cx)
    }
}

impl TabContent for TerminalWorkspace {
    fn content_key(&self) -> &'static str {
        "Terminal"
    }

    fn title(&self, cx: &App) -> SharedString {
        self.active_pane().read(cx).title(cx)
    }

    fn icon(&self, cx: &App) -> Option<Icon> {
        if self.connection_kind(cx) == TerminalConnectionKind::Serial {
            Some(IconName::SerialPort.color())
        } else {
            Some(IconName::TerminalColor.color())
        }
    }

    fn can_duplicate(&self, cx: &App) -> bool {
        self.active_pane().read(cx).duplicate_supported()
    }

    fn duplicate(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Arc<dyn TabContentView>> {
        let source = self.active_pane().read(cx).duplicate_source_snapshot(cx);
        let duplicate = cx.new(|cx| {
            let main = cx.new(|cx| {
                TerminalView::new_from_duplicate_source(source, window, cx).with_workspace_pane()
            });
            Self::from_pane(main, window, cx)
        });
        Some(Arc::new(duplicate))
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        if self
            .panes
            .values()
            .any(|pane| pane.read(cx).requires_close_confirmation(cx))
        {
            self.confirm_close_all(window, cx)
        } else {
            self.close_all_task(cx)
        }
    }

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        Vec::new()
    }
}
