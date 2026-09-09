//! 侧栏树的批量选择委托层。
//!
//! 选择状态已提升到 `HomePage`（卡片/列表/树共享），侧栏只负责把树行交互
//! 翻译成对 `HomePage` 的选择请求，并提供树序的可见 id 供 Shift 范围选择。

use super::PersistentConnectionSidebar;
use crate::home_tab::connection_selection::ConnectionSelectionRequest;

impl PersistentConnectionSidebar {
    pub(super) fn select_connection_from_row(
        &mut self,
        request: ConnectionSelectionRequest,
        cx: &mut gpui::Context<Self>,
    ) {
        let visible_ids = self.manageable_visible_connection_ids(&self.tree_rows(cx), cx);
        self.home_page.update(cx, |home, cx| {
            home.select_connection_in_batch(request, &visible_ids, cx);
        });
    }
}
