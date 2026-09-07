//! 主页连接批量选择状态。
//!
//! 选择状态由 `HomePage` 持有，卡片/列表/树三种布局共享同一份批量模式与选中集，
//! 切换布局不丢失选择；常驻侧栏树通过 `home_page` 读写该状态。

use std::collections::HashSet;

use gpui::{
    AnyElement, App, Context, Entity, InteractiveElement, IntoElement, ParentElement, SharedString,
    Styled, div,
};
use gpui_component::{Sizable, checkbox::Checkbox};

use super::HomePage;

#[derive(Default)]
pub(crate) struct ConnectionSelection {
    active: bool,
    ids: HashSet<i64>,
    anchor_id: Option<i64>,
}

#[derive(Clone, Copy)]
pub(crate) enum ConnectionSelectionMode {
    Replace,
    Toggle,
    Range,
}

pub(crate) struct ConnectionSelectionRequest {
    pub connection_id: i64,
    pub mode: ConnectionSelectionMode,
    pub manageable: bool,
}

/// 勾选框渲染所需的展示属性（元素 id 由调用方按区域命名空间生成）。
pub(crate) struct ConnectionCheckProps {
    pub element_id: SharedString,
    pub connection_id: i64,
    pub checked: bool,
}

impl ConnectionSelection {
    pub(crate) fn is_active(&self) -> bool {
        self.active
    }

    pub(crate) fn set_active(&mut self, active: bool) {
        self.active = active;
        if !active {
            self.clear();
        }
    }

    pub(crate) fn contains(&self, connection_id: i64) -> bool {
        self.ids.contains(&connection_id)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub(crate) fn len(&self) -> usize {
        self.ids.len()
    }

    fn toggle(&mut self, connection_id: i64) {
        if !self.ids.remove(&connection_id) {
            self.ids.insert(connection_id);
        }
        self.anchor_id = Some(connection_id);
    }

    fn replace(&mut self, connection_ids: impl IntoIterator<Item = i64>) {
        let ids = connection_ids.into_iter().collect::<Vec<_>>();
        self.anchor_id = ids.first().copied();
        self.ids = ids.into_iter().collect();
    }

    pub(crate) fn select_visible(&mut self, visible_ids: &[i64]) {
        if visible_ids.iter().all(|id| self.ids.contains(id)) {
            self.ids.retain(|id| !visible_ids.contains(id));
        } else {
            self.ids.extend(visible_ids.iter().copied());
        }
        self.anchor_id = visible_ids.first().copied();
    }

    pub(crate) fn clear(&mut self) {
        self.ids.clear();
        self.anchor_id = None;
    }

    pub(crate) fn ids(&self) -> Vec<i64> {
        let mut ids = self.ids.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub(crate) fn retain(&mut self, valid_ids: &HashSet<i64>) {
        self.ids.retain(|id| valid_ids.contains(id));
        if self.anchor_id.is_some_and(|id| !valid_ids.contains(&id)) {
            self.anchor_id = None;
        }
    }

    fn select(&mut self, connection_id: i64, mode: ConnectionSelectionMode, visible_ids: &[i64]) {
        match mode {
            ConnectionSelectionMode::Replace => self.replace([connection_id]),
            ConnectionSelectionMode::Toggle => self.toggle(connection_id),
            ConnectionSelectionMode::Range => self.select_range(connection_id, visible_ids),
        }
    }

    fn select_range(&mut self, connection_id: i64, visible_ids: &[i64]) {
        let anchor_id = self.anchor_id.unwrap_or(connection_id);
        let Some(anchor_index) = visible_ids.iter().position(|id| *id == anchor_id) else {
            self.replace([connection_id]);
            return;
        };
        let Some(connection_index) = visible_ids.iter().position(|id| *id == connection_id) else {
            return;
        };
        let start = anchor_index.min(connection_index);
        let end = anchor_index.max(connection_index);
        self.ids = visible_ids[start..=end].iter().copied().collect();
        self.anchor_id = Some(anchor_id);
    }
}

impl HomePage {
    pub(crate) fn batch_mode_active(&self) -> bool {
        self.connection_selection.is_active()
    }

    pub(crate) fn set_batch_mode(&mut self, active: bool, cx: &mut Context<Self>) {
        if self.connection_selection.is_active() == active {
            return;
        }
        self.connection_selection.set_active(active);
        cx.notify();
    }

    pub(crate) fn select_connection_in_batch(
        &mut self,
        request: ConnectionSelectionRequest,
        visible_ids: &[i64],
        cx: &mut Context<Self>,
    ) {
        if !self.connection_selection.is_active() || !request.manageable {
            return;
        }
        self.connection_selection
            .select(request.connection_id, request.mode, visible_ids);
        cx.notify();
    }

    pub(crate) fn select_visible_connections(
        &mut self,
        visible_ids: &[i64],
        cx: &mut Context<Self>,
    ) {
        self.connection_selection.select_visible(visible_ids);
        cx.notify();
    }

    /// 数据刷新后裁剪选中集：只保留仍存在且可管理的连接。
    pub(super) fn prune_connection_selection(&mut self) {
        let valid_ids = self
            .connections
            .iter()
            .filter_map(|connection| connection.id)
            .filter(|id| self.can_move_connection(*id))
            .collect::<HashSet<_>>();
        self.connection_selection.retain(&valid_ids);
    }

    /// 当前页面可见且可管理的连接 id（按分组展示顺序；最近区为子集，不重复计入）。
    pub(super) fn visible_manageable_connection_ids(&self, query: &str, cx: &App) -> Vec<i64> {
        self.home_groups(query, cx)
            .into_iter()
            .flat_map(|(_, _, connections)| connections.into_iter())
            .filter_map(|connection| connection.id)
            .filter(|id| self.can_move_connection(*id))
            .collect()
    }
}

pub(crate) fn connection_selection_checkbox(
    home: &Entity<HomePage>,
    props: ConnectionCheckProps,
) -> AnyElement {
    let home = home.clone();
    div()
        .id(props.element_id)
        .flex_shrink_0()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            Checkbox::new(SharedString::from(format!(
                "connection-check-{}",
                props.connection_id
            )))
            .xsmall()
            .checked(props.checked)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                home.update(cx, |home, cx| {
                    home.connection_selection.toggle(props.connection_id);
                    cx.notify();
                });
            }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{ConnectionSelection, ConnectionSelectionMode};
    use std::collections::HashSet;

    #[test]
    fn toggling_connections_adds_and_removes_them() {
        let mut selection = ConnectionSelection::default();

        selection.toggle(7);
        selection.toggle(11);
        assert!(selection.contains(7));
        assert!(selection.contains(11));
        assert_eq!(selection.len(), 2);

        selection.toggle(7);
        assert!(!selection.contains(7));
        assert_eq!(selection.ids(), vec![11]);
    }

    #[test]
    fn replacing_and_pruning_selection_keeps_only_valid_connections() {
        let mut selection = ConnectionSelection::default();
        selection.replace([9, 3, 9, 6]);
        selection.retain(&HashSet::from([3, 6]));

        assert_eq!(selection.ids(), vec![3, 6]);
        selection.clear();
        assert!(selection.is_empty());
    }

    #[test]
    fn shift_selection_uses_the_visible_connection_order() {
        let mut selection = ConnectionSelection::default();
        selection.replace([11]);

        selection.select(17, ConnectionSelectionMode::Range, &[7, 11, 13, 17, 19]);

        assert_eq!(selection.ids(), vec![11, 13, 17]);
        assert_eq!(selection.anchor_id, Some(11));
    }

    #[test]
    fn selecting_visible_connections_toggles_all_visible_items() {
        let mut selection = ConnectionSelection::default();
        selection.replace([3]);

        selection.select_visible(&[3, 6]);
        assert_eq!(selection.ids(), vec![3, 6]);

        selection.select_visible(&[3, 6]);
        assert!(selection.is_empty());
    }

    #[test]
    fn leaving_batch_mode_clears_selection_and_anchor() {
        let mut selection = ConnectionSelection::default();
        selection.set_active(true);
        selection.replace([3, 6]);
        assert!(selection.is_active());

        selection.set_active(false);

        assert!(!selection.is_active());
        assert!(selection.is_empty());
        assert_eq!(selection.anchor_id, None);
    }
}
