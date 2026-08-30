use gpui::Pixels;
use one_core::settings::{AppSettings, ConnectionSidebarTreeState};

use super::{PersistentConnectionSidebar, PersistentConnectionSidebarEvent};

/// 拖拽过程中宽度落盘的最小增量，避免每个 mouse move 都写设置文件。
const TREE_WIDTH_PERSIST_DELTA: f32 = 24.0;

impl PersistentConnectionSidebar {
    pub(super) fn set_workspace_collapsed(
        &mut self,
        workspace_id: i64,
        collapsed: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        let current = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .map(|workspace| workspace.sidebar_collapsed);
        if current.is_some_and(|current| current != collapsed) {
            self.home_page.update(cx, |home, cx| {
                home.set_workspace_sidebar_collapsed(workspace_id, collapsed, cx);
            });
        }
    }

    pub(super) fn toggle_workspace_collapsed(
        &mut self,
        workspace_id: i64,
        cx: &mut gpui::Context<Self>,
    ) {
        let collapsed = self
            .home_page
            .read(cx)
            .workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .map(|workspace| !workspace.sidebar_collapsed);
        if let Some(collapsed) = collapsed {
            self.set_workspace_collapsed(workspace_id, collapsed, cx);
        }
    }

    pub(super) fn set_hide_empty_workspaces(&mut self, hide: bool, cx: &mut gpui::Context<Self>) {
        if self.hide_empty_workspaces != hide {
            self.hide_empty_workspaces = hide;
            self.persist_tree_state(cx);
            cx.notify();
        }
    }

    pub(super) fn set_auto_hide_tree(&mut self, auto_hide: bool, cx: &mut gpui::Context<Self>) {
        if self.auto_hide_tree != auto_hide {
            self.auto_hide_tree = auto_hide;
            self.persist_tree_state(cx);
            cx.notify();
        }
    }

    /// 更新连接树宽度（仅内存），拖拽结束或达到阈值时再落盘。
    pub(super) fn set_tree_width(&mut self, width: Pixels, cx: &mut gpui::Context<Self>) {
        if self.tree_width != width {
            self.tree_width = width;
            cx.notify();
        }
    }

    /// 将当前宽度写入设置（拖拽结束或状态持久化时调用）。
    pub(super) fn persist_tree_width(&mut self, cx: &mut gpui::Context<Self>) {
        let width = f32::from(self.tree_width).round().max(0.0) as u32;
        self.persisted_tree_width = self.tree_width;
        AppSettings::update_and_save(cx, |settings| {
            settings.connection_sidebar_tree_state.tree_width = width;
        });
    }

    /// 拖拽过程中按增量阈值落盘，避免高频写设置文件。
    pub(super) fn persist_tree_width_if_moved_far(&mut self, cx: &mut gpui::Context<Self>) {
        if (f32::from(self.tree_width) - f32::from(self.persisted_tree_width)).abs()
            >= TREE_WIDTH_PERSIST_DELTA
        {
            self.persist_tree_width(cx);
        }
    }

    /// 双击打开会话后，若开启了自动隐藏，则把连接树收起。
    pub(super) fn collapse_after_open(&mut self, cx: &mut gpui::Context<Self>) {
        if self.auto_hide_tree && self.tree_expanded {
            self.set_tree_expanded(false, cx);
            cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded: false });
        }
    }

    /// 点击设置、扩展、AI 工作台等非连接区域时，若开启了自动隐藏且连接树已展开，则收起连接树。
    pub(crate) fn collapse_if_auto_hide(&mut self, cx: &mut gpui::Context<Self>) {
        if self.auto_hide_tree && self.tree_expanded {
            self.set_tree_expanded(false, cx);
            cx.emit(PersistentConnectionSidebarEvent::TreeVisibilityChanged { expanded: false });
        }
    }

    pub(super) fn collapse_all_groups(&mut self, cx: &mut gpui::Context<Self>) {
        self.home_page.update(cx, |home, cx| {
            home.set_all_workspaces_sidebar_collapsed(true, cx);
        });
    }

    fn persist_tree_state(&self, cx: &mut gpui::Context<Self>) {
        let tree_state = stored_tree_state(
            self.hide_empty_workspaces,
            self.auto_hide_tree,
            self.tree_width,
        );
        AppSettings::update_and_save(cx, |settings| {
            settings.connection_sidebar_tree_state = tree_state;
        });
    }
}

fn stored_tree_state(
    hide_empty_workspaces: bool,
    auto_hide_tree: bool,
    tree_width: Pixels,
) -> ConnectionSidebarTreeState {
    ConnectionSidebarTreeState {
        hide_empty_workspaces,
        auto_hide_tree,
        tree_width: f32::from(tree_width).round().max(0.0) as u32,
    }
}

#[cfg(test)]
mod tests {
    use super::stored_tree_state;

    #[test]
    fn stored_tree_state_preserves_local_sidebar_preferences() {
        let state = stored_tree_state(true, false, gpui::px(320.0));

        assert!(state.hide_empty_workspaces);
        assert!(!state.auto_hide_tree);
        assert_eq!(state.tree_width, 320);
    }
}
