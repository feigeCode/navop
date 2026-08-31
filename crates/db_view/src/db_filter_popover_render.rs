use gpui::{
    AnyElement, Context, Entity, Focusable, InteractiveElement, IntoElement, KeyDownEvent,
    MouseButton, ParentElement, Render, Styled, Window, anchored, deferred, div,
};
use gpui_component::{
    ActiveTheme, Sizable, StyledExt,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    list::{List, ListState},
    v_flex,
};
use rust_i18n::t;

use crate::{
    db_filter_list::DatabaseListDelegate,
    db_filter_popover::{
        DbFilterPopover, KEY_CONTEXT, LIST_MAX_HEIGHT, PANEL_MAX_HEIGHT, PANEL_WIDTH, WINDOW_MARGIN,
    },
    db_tree_view::DbTreeView,
};

impl DbFilterPopover {
    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if event.keystroke.key == "escape" {
            self.dismiss(window, cx);
            cx.stop_propagation();
        }
    }

    fn render_panel(
        &mut self,
        connection_id: &str,
        list_state: &Entity<ListState<DatabaseListDelegate>>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conn_id = connection_id.to_string();
        let is_all_selected = self.tree_view.read(cx).is_all_selected(&conn_id);
        let focus_handle = list_state.focus_handle(cx);

        v_flex()
            .id("db-filter-popover")
            .occlude()
            .tab_group()
            .track_focus(&focus_handle)
            .key_context(KEY_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .popover_style(cx)
            .p_3()
            .top_1()
            .child(
                v_flex()
                    .w(PANEL_WIDTH)
                    .max_h(PANEL_MAX_HEIGHT)
                    .gap_2()
                    .p_2()
                    .child(Self::render_header(
                        &self.tree_view,
                        &conn_id,
                        is_all_selected,
                    ))
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
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let open = self.open_connection.clone().and_then(|connection_id| {
            self.list_states
                .get(&connection_id)
                .cloned()
                .map(|list| (connection_id, list))
        });

        let Some((connection_id, list_state)) = open else {
            return div().into_any_element();
        };

        let panel = self.render_panel(&connection_id, &list_state, cx);
        let anchor = self.anchor;
        let entity = cx.entity();

        div()
            .child(
                deferred(
                    anchored().child(
                        div()
                            .id("db-filter-popover-backdrop")
                            .w(window.bounds().size.width)
                            .h(window.bounds().size.height)
                            .on_mouse_down(MouseButton::Left, {
                                let entity = entity.clone();
                                move |_, window, cx| {
                                    entity.update(cx, |this, cx| this.dismiss(window, cx));
                                }
                            })
                            .on_mouse_down(MouseButton::Right, {
                                let entity = entity.clone();
                                move |_, window, cx| {
                                    entity.update(cx, |this, cx| this.dismiss(window, cx));
                                }
                            })
                            .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
                            .child(
                                anchored()
                                    .position(anchor)
                                    .snap_to_window_with_margin(WINDOW_MARGIN)
                                    .child(panel),
                            ),
                    ),
                )
                .with_priority(1),
            )
            .into_any_element()
    }
}
