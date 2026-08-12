use std::collections::HashSet;

use gpui::{
    AnyElement, Entity, InteractiveElement, IntoElement, ParentElement, SharedString, Styled, div,
};
use gpui_component::{Sizable, checkbox::Checkbox};

use super::PersistentConnectionSidebar;

#[derive(Default)]
pub(super) struct ConnectionSelection {
    ids: HashSet<i64>,
}

impl ConnectionSelection {
    pub(super) fn contains(&self, connection_id: i64) -> bool {
        self.ids.contains(&connection_id)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.ids.is_empty()
    }

    pub(super) fn len(&self) -> usize {
        self.ids.len()
    }

    pub(super) fn toggle(&mut self, connection_id: i64) {
        if !self.ids.remove(&connection_id) {
            self.ids.insert(connection_id);
        }
    }

    pub(super) fn replace(&mut self, connection_ids: impl IntoIterator<Item = i64>) {
        self.ids = connection_ids.into_iter().collect();
    }

    pub(super) fn clear(&mut self) {
        self.ids.clear();
    }

    pub(super) fn ids(&self) -> Vec<i64> {
        let mut ids = self.ids.iter().copied().collect::<Vec<_>>();
        ids.sort_unstable();
        ids
    }

    pub(super) fn retain(&mut self, valid_ids: &HashSet<i64>) {
        self.ids.retain(|id| valid_ids.contains(id));
    }
}

impl PersistentConnectionSidebar {
    pub(super) fn select_connection_from_row(
        &mut self,
        connection_id: i64,
        additive: bool,
        manageable: bool,
        cx: &mut gpui::Context<Self>,
    ) {
        if manageable && additive {
            self.connection_selection.toggle(connection_id);
        } else if manageable {
            self.connection_selection.replace([connection_id]);
        } else {
            self.connection_selection.clear();
        }
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
}
