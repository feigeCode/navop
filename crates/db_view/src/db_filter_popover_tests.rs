use super::*;
use std::{cell::RefCell, rc::Rc};

use gpui::{
    AppContext, IntoElement, ParentElement, Render, Styled, TestAppContext, VisualTestContext, div,
};
use gpui_component::{Root, list::ListDelegate};

struct FilterHost {
    tree: Entity<DbTreeView>,
    popover: Entity<DbFilterPopover>,
}

impl Render for FilterHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .child(self.tree.clone())
            .child(self.popover.clone())
    }
}

fn setup(cx: &mut TestAppContext) -> (Entity<FilterHost>, &mut VisualTestContext) {
    cx.update(gpui_component::init);
    let host_slot = Rc::new(RefCell::new(None));
    let host_slot_clone = host_slot.clone();
    let (_, cx) = cx.add_window_view(move |window, cx| {
        let connections = Vec::new();
        let tree = cx.new(|cx| DbTreeView::new(&connections, window, cx));
        let popover = cx.new(|cx| DbFilterPopover::new(tree.clone(), cx));
        tree.update(cx, |tree, _| {
            tree.bind_db_filter_popover(popover.downgrade());
        });
        let host = cx.new(|_| FilterHost { tree, popover });
        *host_slot_clone.borrow_mut() = Some(host.clone());
        Root::new(host, window, cx)
    });
    let host = host_slot.borrow().clone().expect("filter host captured");
    (host, cx)
}

#[gpui::test]
fn filter_search_keeps_independent_panel_open(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.run_until_parked();
    let popover = host.read_with(cx, |host, _| host.popover.clone());

    popover.update_in(cx, |popover, window, cx| {
        popover.set_anchor("1", Point::default());
        popover.toggle("1", window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    let list_state = popover
        .read_with(cx, |popover, _| popover.list_states.get("1").cloned())
        .expect("opening the filter creates its list state");
    cx.update(|window, cx| {
        list_state.update(cx, |state, cx| {
            state.delegate_mut().databases = vec![
                ("analytics".to_string(), "analytics".to_string()),
                ("primary".to_string(), "primary".to_string()),
            ];
            let _ = state.delegate_mut().perform_search("ana", window, cx);
        });
    });
    cx.run_until_parked();

    assert!(popover.read_with(cx, |popover, _| popover.is_open_for("1")));
    assert_eq!(
        1,
        list_state.read_with(cx, |state, _| state.delegate().filtered_databases.len())
    );
}

#[gpui::test]
fn switching_connections_keeps_the_panel_lifecycle_consistent(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let popover = host.read_with(cx, |host, _| host.popover.clone());

    popover.update_in(cx, |popover, window, cx| {
        popover.toggle("1", window, cx);
        popover.toggle("2", window, cx);
        assert!(popover.is_open_for("2"));
        popover.dismiss(window, cx);
        assert!(!popover.is_open_for("2"));
    });
}

#[gpui::test]
fn switching_connections_restores_the_focus_from_before_open(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    cx.run_until_parked();
    let popover = host.read_with(cx, |host, _| host.popover.clone());
    let previous_focus = cx.update(|window, cx| {
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        focus
    });

    popover.update_in(cx, |popover, window, cx| {
        popover.toggle("1", window, cx);
        popover.toggle("2", window, cx);
    });
    cx.update(|window, _| window.refresh());
    cx.run_until_parked();
    popover.update_in(cx, |popover, window, cx| popover.dismiss(window, cx));

    assert_eq!(
        Some(previous_focus),
        cx.update(|window, cx| window.focused(cx))
    );
}

#[gpui::test]
fn removing_open_connection_closes_and_unregisters_panel(cx: &mut TestAppContext) {
    let (host, cx) = setup(cx);
    let popover = host.read_with(cx, |host, _| host.popover.clone());

    popover.update_in(cx, |popover, window, cx| {
        popover.toggle("1", window, cx);
        popover.remove_connection("1", cx);
        assert!(!popover.is_open_for("1"));
        assert!(!popover.list_states.contains_key("1"));
    });
}

#[test]
fn database_filter_is_structurally_hosted_outside_tree_rows() {
    let tree = include_str!("db_tree_view.rs");
    let tab = include_str!("database_tab.rs");
    let panel_state = include_str!("db_filter_popover.rs");
    let panel = include_str!("db_filter_popover_render.rs");
    let adapter = include_str!("window_positioned_popover.rs");

    assert!(!tree.contains("db_filter_popover_open"));
    assert!(!tree.contains("db_filter_list_states"));
    assert!(!tree.contains("Popover::new(SharedString::from(format!(\"db-filter-"));
    assert!(tab.contains("tree.bind_db_filter_popover(db_filter_popover.downgrade())"));
    let render = tab
        .rsplit_once("impl Render for DatabaseTabView")
        .expect("database tab render implementation")
        .1;
    assert!(!render.contains("return div()"));
    assert_eq!(
        1,
        render
            .matches(".child(self.db_filter_popover.clone())")
            .count()
    );
    assert!(!panel_state.contains("GlobalState"));
    assert!(panel.contains("WindowPositionedPopover::new"));
    assert!(adapter.contains("Popover::new(self.id)"));
    assert!(!panel.contains(".position(anchor)"));
    assert!(!adapter.contains("GlobalState"));
}
