use super::*;

impl HomePage {
    pub(super) fn render_connection_list_item(
        &self,
        conn: StoredConnection,
        selected_id: Option<i64>,
        _index: usize,
        recent: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conn_id = conn.id;
        let open_connection = conn.clone();
        // 最近区是纯快捷入口：不参与批量模式（无勾选框、点击不进选择），
        // 同一连接在下方分组中的实例才承载批量交互。
        let batch_mode = self.batch_mode_active() && !recent;
        let can_manage = conn_id.is_some_and(|id| self.can_move_connection(id));
        // 批量模式下选中态来自批量选择集（参考常驻侧栏连接树）。
        let is_selected = if batch_mode {
            conn_id.is_some_and(|id| self.connection_selection.contains(id))
        } else {
            selected_id == conn.id
        };
        let is_active = conn
            .id
            .is_some_and(|id| cx.global::<ActiveConnections>().is_active(id));
        let can_edit = self
            .team_permissions
            .can_edit_connection(conn.team_id.as_deref());
        let team_badge = if cfg!(feature = "screenshot-safe") {
            None
        } else {
            connection_team_badge(conn.team_id.as_deref(), self.team_permissions.teams())
        };
        let actions = self.render_connection_list_actions(&conn, can_edit, cx);
        let display_name = connection_display_name(&conn);
        let home_for_menu = cx.entity();
        let hover_bg = cx.theme().list_hover;

        // 同一连接会同时出现在最近区与普通分组，元素 ID 按展示区命名空间区分（redesign §6.2）。
        let row_id = if recent {
            "conn-list-item-recent"
        } else {
            "conn-list-item"
        };
        let row_id = SharedString::from(format!("{row_id}-{}", conn_id.unwrap_or(0)));

        let row = h_flex()
            .id(row_id.clone())
            .w_full()
            .px_3p5()
            .py_2p5()
            .rounded(px(10.0))
            .border_1()
            .border_color(cx.theme().border)
            .items_center()
            .gap_3()
            .relative()
            .group("")
            // 选中态与卡片一致：主题选中背景；hover 不覆盖选中（refinement §7.2）。
            .when(is_selected, |this| this.bg(cx.theme().list_active))
            .when(!is_selected, |this| this.hover(|style| style.bg(hover_bg)))
            .cursor_pointer()
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_connection_from_quick(&open_connection, window, cx);
                cx.notify()
            }))
            .on_click(cx.listener(move |this, event: &gpui::ClickEvent, _, cx| {
                if batch_mode && can_manage {
                    if let Some(id) = conn_id {
                        let mode = if event.modifiers().shift {
                            connection_selection::ConnectionSelectionMode::Range
                        } else if event.modifiers().secondary() {
                            connection_selection::ConnectionSelectionMode::Toggle
                        } else {
                            connection_selection::ConnectionSelectionMode::Replace
                        };
                        let query = this.search_query.read(cx).to_lowercase();
                        let visible_ids = this.visible_manageable_connection_ids(&query, cx);
                        this.select_connection_in_batch(
                            connection_selection::ConnectionSelectionRequest {
                                connection_id: id,
                                mode,
                                manageable: true,
                            },
                            &visible_ids,
                            cx,
                        );
                    }
                }
                this.selected_connection_id = conn_id;
                cx.notify();
            }))
            .when(batch_mode && can_manage, |this| {
                this.child(connection_selection::connection_selection_checkbox(
                    &cx.entity(),
                    connection_selection::ConnectionCheckProps {
                        element_id: SharedString::from(format!("{row_id}-check")),
                        connection_id: conn_id.unwrap_or(0),
                        checked: is_selected,
                    },
                ))
            })
            .when(is_active, |this| {
                this.child(
                    div()
                        .flex_shrink_0()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .bg(cx.theme().success),
                )
            })
            .child(
                div()
                    .size(gpui::rems(2.25))
                    .rounded(px(8.0))
                    .border_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().muted)
                    .flex()
                    .flex_shrink_0()
                    .items_center()
                    .justify_center()
                    .child(self.connection_icon(&conn, ConnectionVisualSize::List)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .flex_1()
                                    .min_w_0()
                                    .child(display_name),
                            )
                            .when_some(team_badge, |this, badge| {
                                this.child(render_team_badge(&row_id, &conn, badge, cx))
                            })
                            .child(actions),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .w_full()
                            .min_w_0()
                            .child(self.connection_info_text(&conn)),
                    ),
            );
        match conn_id {
            Some(id) => row
                .context_menu(move |menu, window, cx| {
                    crate::persistent_connection_sidebar::build_connection_context_menu(
                        menu,
                        &home_for_menu,
                        id,
                        window,
                        cx,
                    )
                })
                .into_any_element(),
            None => row.into_any_element(),
        }
    }
}
