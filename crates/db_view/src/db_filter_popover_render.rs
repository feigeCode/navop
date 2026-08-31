use gpui::{
    AnyElement, Bounds, Context, Entity, Focusable, InteractiveElement, IntoElement, ParentElement,
    Pixels, Render, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, ElementExt, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    list::{List, ListState},
    v_flex,
};
use rust_i18n::t;

use crate::{
    db_filter_list::DatabaseListDelegate,
    db_filter_popover::{DbFilterPopover, LIST_MAX_HEIGHT, PANEL_MAX_HEIGHT, PANEL_WIDTH},
    db_tree_view::DbTreeView,
    window_positioned_popover::WindowPositionedPopover,
};

impl DbFilterPopover {
    fn render_panel(
        tree_view: &Entity<DbTreeView>,
        connection_id: &str,
        list_state: &Entity<ListState<DatabaseListDelegate>>,
        cx: &mut gpui::App,
    ) -> AnyElement {
        let conn_id = connection_id.to_string();
        let is_all_selected = tree_view.read(cx).is_all_selected(&conn_id);
        let focus_handle = list_state.focus_handle(cx);

        v_flex()
            .id("db-filter-popover")
            .track_focus(&focus_handle)
            .child(
                v_flex()
                    .w(PANEL_WIDTH)
                    .max_h(PANEL_MAX_HEIGHT)
                    .gap_2()
                    .p_2()
                    .child(Self::render_header(tree_view, &conn_id, is_all_selected))
                    .child(div().border_t_1().border_color(cx.theme().border))
                    .child(
                        List::new(list_state)
                            .w_full()
                            .max_h(LIST_MAX_HEIGHT)
                            .p(gpui::px(8.))
                            .flex_1()
                            .border_1()
                            .border_color(cx.theme().border)
                            .rounded(cx.theme().radius),
                    ),
            )
            .into_any_element()
    }

    fn render_header(
        tree_view: &Entity<DbTreeView>,
        connection_id: &str,
        is_all_selected: bool,
    ) -> gpui::Stateful<gpui::Div> {
        let view_select = tree_view.clone();
        let conn_select = connection_id.to_string();
        let view_clear = tree_view.clone();
        let conn_clear = connection_id.to_string();

        h_flex()
            .id("db-filter-header")
            .w_full()
            .items_center()
            .justify_between()
            .px_1()
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Checkbox::new("select-all")
                            .checked(is_all_selected)
                            .on_click(move |_, _, cx| {
                                view_select.update(cx, |this, cx| {
                                    if this.is_all_selected(&conn_select) {
                                        this.deselect_all_databases(&conn_select, cx);
                                    } else {
                                        this.select_all_databases(&conn_select, cx);
                                    }
                                });
                            }),
                    )
                    .child(div().text_sm().child(t!("Common.select_all").to_string())),
            )
            .child(
                Button::new("clear-filter")
                    .ghost()
                    .small()
                    .label(t!("Common.clear_filter"))
                    .on_click(move |_, _, cx| {
                        view_clear.update(cx, |this, cx| {
                            this.deselect_all_databases(&conn_clear, cx);
                        });
                    }),
            )
    }
}

impl Render for DbFilterPopover {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open_connection = self.open_connection.clone();
        let list_state = open_connection
            .as_ref()
            .and_then(|connection_id| self.list_states.get(connection_id).cloned());
        let is_open = open_connection.is_some() && list_state.is_some();
        let entity = cx.entity();
        let origin_entity = entity.clone();
        let tree_view = self.tree_view.clone();
        let (focus_handle, content) = match (open_connection, list_state) {
            (Some(connection_id), Some(list_state)) => (
                list_state.focus_handle(cx),
                Self::render_panel(&tree_view, &connection_id, &list_state, cx),
            ),
            _ => (cx.focus_handle(), div().into_any_element()),
        };

        div()
            .absolute()
            .inset_0()
            .on_prepaint(move |bounds: Bounds<Pixels>, _, cx| {
                origin_entity.update(cx, |this, cx| {
                    this.set_host_origin(bounds.origin, cx);
                });
            })
            .child(
                WindowPositionedPopover::new("db-filter-popover-host", self.anchor, focus_handle)
                    .host_origin(self.host_origin)
                    .open(is_open)
                    .content(content)
                    .on_open_change(move |open, window, cx| {
                        if !open {
                            entity.update(cx, |this, cx| this.dismiss(window, cx));
                        }
                    }),
            )
            .into_any_element()
    }
}
