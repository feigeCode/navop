//! 数据库筛选弹出面板。
//!
//! 该面板由 `DatabaseTabView` 作为 `DbTreeView` 的兄弟视图渲染，而不是挂在
//! 虚拟化树行内部的 `Popover` 元素里。筛选列表和搜索输入因此不再属于
//! `DbTreeView` 的 dispatch 子树，按键通知也不再直接使树视图成为脏视图；这避免
//! Windows 上 deferred overlay 随树行重建而丢失原生输入框的 IME/焦点上下文。

use std::collections::HashMap;

use gpui::{AppContext, Context, Entity, FocusHandle, Focusable, Pixels, Point, Window, px};
use gpui_component::list::ListState;

use crate::{db_filter_list::DatabaseListDelegate, db_tree_view::DbTreeView};

pub(super) const PANEL_WIDTH: Pixels = px(280.0);
pub(super) const PANEL_MAX_HEIGHT: Pixels = px(400.0);
pub(super) const LIST_MAX_HEIGHT: Pixels = px(320.0);

// ============================================================================
// DbFilterPopover - 独立承载的数据库筛选面板
// ============================================================================

pub struct DbFilterPopover {
    pub(super) tree_view: Entity<DbTreeView>,
    /// 当前打开的连接 ID（None 表示面板关闭）
    pub(super) open_connection: Option<String>,
    /// 每个连接触发按钮的锚点（窗口坐标，由行内 trigger 的 on_prepaint 静默同步）
    anchors: HashMap<String, Point<Pixels>>,
    /// 面板锚点（打开时从 anchors 快照）
    pub(super) anchor: Point<Pixels>,
    /// sibling 宿主在窗口中的原点，用于把树行锚点转换为本地绝对坐标。
    pub(super) host_origin: Point<Pixels>,
    /// 每个连接的筛选列表状态
    pub(super) list_states: HashMap<String, Entity<ListState<DatabaseListDelegate>>>,
    /// 打开前的焦点，关闭时恢复。
    previous_focus: Option<FocusHandle>,
}

impl DbFilterPopover {
    pub fn new(tree_view: Entity<DbTreeView>, _cx: &mut Context<Self>) -> Self {
        Self {
            tree_view,
            open_connection: None,
            anchors: HashMap::new(),
            anchor: Point::default(),
            host_origin: Point::default(),
            list_states: HashMap::new(),
            previous_focus: None,
        }
    }

    pub fn is_open_for(&self, connection_id: &str) -> bool {
        self.open_connection.as_deref() == Some(connection_id)
    }

    pub fn refresh_selection(&mut self, cx: &mut Context<Self>) {
        if self.open_connection.is_some() {
            cx.notify();
        }
    }

    /// 由行内触发按钮的 on_prepaint 静默同步锚点（不触发任何重渲染）。
    pub fn set_anchor(&mut self, connection_id: &str, anchor: Point<Pixels>) {
        self.anchors.insert(connection_id.to_string(), anchor);
    }

    pub(super) fn set_host_origin(&mut self, origin: Point<Pixels>, cx: &mut Context<Self>) {
        if self.host_origin != origin {
            self.host_origin = origin;
            cx.notify();
        }
    }

    /// 行内触发按钮点击：已打开则关闭，否则打开（或切换到其他连接）。
    pub fn toggle(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.is_open_for(connection_id) {
            self.dismiss(window, cx);
        } else {
            self.open(connection_id, window, cx);
        }
    }

    pub fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection_id) = self.open_connection.take() else {
            return;
        };
        if let Some(previous) = self.previous_focus.take() {
            let panel_focused = self
                .list_states
                .get(&connection_id)
                .is_some_and(|list| list.focus_handle(cx).contains_focused(window, cx));
            if panel_focused {
                previous.focus(window, cx);
            }
        }
        self.notify_panel_and_tree(cx);
    }

    /// 连接被移除时清理其筛选状态；若面板正打开该连接则直接关闭。
    pub fn remove_connection(&mut self, connection_id: &str, cx: &mut Context<Self>) {
        self.anchors.remove(connection_id);
        self.list_states.remove(connection_id);
        if self.is_open_for(connection_id) {
            self.open_connection = None;
            self.previous_focus = None;
            self.notify_panel_and_tree(cx);
        }
    }

    fn open(&mut self, connection_id: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.open_connection.is_none() {
            self.previous_focus = window.focused(cx);
        }
        self.anchor = self.anchors.get(connection_id).copied().unwrap_or_default();
        self.open_connection = Some(connection_id.to_string());
        let list_state = self.ensure_list_state(connection_id, window, cx);
        list_state.focus_handle(cx).focus(window, cx);
        self.notify_panel_and_tree(cx);
    }

    /// 获取或创建该连接的筛选列表；重开时用最新数据库列表刷新内容。
    fn ensure_list_state(
        &mut self,
        connection_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ListState<DatabaseListDelegate>> {
        let databases = self
            .tree_view
            .read(cx)
            .get_databases_for_connection(connection_id);

        if let Some(existing) = self.list_states.get(connection_id) {
            existing.update(cx, |state, _| {
                let delegate = state.delegate_mut();
                delegate.databases = databases.clone();
                delegate.filtered_databases = databases;
            });
            return existing.clone();
        }

        let list_state = cx.new(|cx| {
            ListState::new(
                DatabaseListDelegate::new(
                    self.tree_view.clone(),
                    connection_id.to_string(),
                    databases,
                ),
                window,
                cx,
            )
            .searchable(true)
        });
        self.list_states
            .insert(connection_id.to_string(), list_state.clone());
        list_state
    }

    /// 仅在打开/关闭时刷新树的 trigger 状态；筛选输入只刷新 ListState，不经过这里。
    fn notify_panel_and_tree(&self, cx: &mut Context<Self>) {
        cx.notify();
        self.tree_view.update(cx, |_, cx| cx.notify());
    }
}

#[cfg(test)]
#[path = "db_filter_popover_tests.rs"]
mod tests;
