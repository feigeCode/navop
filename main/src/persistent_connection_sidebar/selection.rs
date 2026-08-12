use std::collections::HashSet;

use gpui::{
    AnyElement, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
};
use gpui_component::{Sizable, checkbox::Checkbox};

use super::PersistentConnectionSidebar;

#[derive(Default)]
pub(super) struct ConnectionSelection {
    active: bool,
    ids: HashSet<i64>,
    anchor_id: Option<i64>,
}

#[derive(Clone, Copy)]
pub(super) enum ConnectionSelectionMode {
    Replace,
    Toggle,
    Range,
}

pub(super) struct ConnectionSelectionRequest {
    pub connection_id: i64,
    pub mode: ConnectionSelectionMode,
    pub manageable: bool,
}

impl ConnectionSelection {
    pub(super) fn is_active(&self) -> bool {
        self.active
    }

    pub(super) fn set_active(&mut self, active: bool) {
        self.active = active;
        if !active {
            self.clear();
        }
    }

    #[cfg(test)]
    fn anchor_id(&self) -> Option<i64> {
        self.anchor_id
    }

    pub(super) fn contains(&self, connection_id: i64) -> bool {
        self.ids.contains(&connection_id)
    }

    #[cfg(test)]
    fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.ids.len()
    }

    pub(super) fn toggle(&mut self, connection_id: i64) {
        if !self.ids.remove(&connection_id) {
            self.ids.insert(connection_id);
        }
        self.anchor_id = Some(connection_id);
    }

    pub(super) fn replace(&mut self, connection_ids: impl IntoIterator<Item = i64>) {
        let ids = connection_ids.into_iter().collect::<Vec<_>>();
        self.anchor_id = ids.first().copied();
        self.ids = ids.into_iter().collect();
    }

    pub(super) fn select_visible(&mut self, visible_ids: &[i64]) {
        if visible_ids.iter().all(|id| self.ids.contains(id)) {
            self.ids.retain(|id| !visible_ids.contains(id));
        } else {
            self.ids.extend(visible_ids.iter().copied());
        }
        self.anchor_id = visible_ids.first().copied();
    }

    pub(super) fn clear(&mut self) {
        self.ids.clear();
        self.anchor_id = None;
    }

    pub(super) fn ids(&self) -> Vec<i64> {
        let mut ids = self.ids.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub(super) fn retain(&mut self, valid_ids: &HashSet<i64>) {
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

impl PersistentConnectionSidebar {
    pub(super) fn set_batch_mode(&mut self, active: bool, cx: &mut gpui::Context<Self>) {
        if self.connection_selection.is_active() == active {
            return;
        }
        self.connection_selection.set_active(active);
        cx.notify();
    }

    pub(super) fn select_connection_from_row(
        &mut self,
        request: ConnectionSelectionRequest,
        cx: &mut gpui::Context<Self>,
    ) {
        if !self.connection_selection.is_active() || !request.manageable {
            return;
        }
        let visible_ids = self.manageable_visible_connection_ids(&self.tree_rows(cx), cx);
        self.connection_selection
            .select(request.connection_id, request.mode, &visible_ids);
        cx.notify();
    }
}

pub(super) fn connection_selection_checkbox(
    view: Entity<PersistentConnectionSidebar>,
    connection_id: i64,
    checked: bool,
) -> AnyElement {
    div()
        .id(SharedString::from(format!(
            "persistent-connection-check-wrap-{connection_id}"
        )))
        .flex_shrink_0()
        .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            Checkbox::new(SharedString::from(format!(
                "persistent-connection-check-{connection_id}"
            )))
            .xsmall()
            .checked(checked)
            .on_click(move |_, _, cx| {
                cx.stop_propagation();
                view.update(cx, |this, cx| {
                    this.connection_selection.toggle(connection_id);
                    cx.notify();
                });
            }),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::ConnectionSelection;
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

        selection.select(
            17,
            super::ConnectionSelectionMode::Range,
            &[7, 11, 13, 17, 19],
        );

        assert_eq!(selection.ids(), vec![11, 13, 17]);
        assert_eq!(selection.anchor_id(), Some(11));
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
        assert_eq!(selection.anchor_id(), None);
    }
}
