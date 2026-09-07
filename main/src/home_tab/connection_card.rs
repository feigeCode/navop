use super::*;

impl HomePage {
    pub(super) fn render_connection_card(
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
        // 同一连接会同时出现在最近区与普通分组，元素 ID 按展示区命名空间区分（redesign §6.2）。
        let card_id = if recent {
            "conn-card-recent"
        } else {
            "conn-card"
        };
        let card_id = SharedString::from(format!("{card_id}-{}", conn_id.unwrap_or(0)));
        let hover_bg = cx.theme().list_hover;
        let home_for_menu = cx.entity();

        let card = v_flex()
            .justify_center()
            .id(card_id.clone())
            .w_full()
            // 最近区更紧凑，靠留白与去重信息实现（redesign §6.1）。
            .h(gpui::rems(if recent { 3.75 } else { 4.75 }))
            .rounded(px(11.0))
            .bg(cx.theme().background)
            .px_3()
            .when(recent, |this| this.py_2())
            .when(!recent, |this| this.py_2p5())
            .border_1()
            .relative()
            .overflow_hidden()
            .group("")
            // 选中态：持续的主题选中背景+边框，不依赖悬停（redesign §5.5）。
            .when(is_selected, |this| {
                this.border_color(cx.theme().list_active_border)
                    .bg(cx.theme().list_active)
            })
            // 非选中 hover 只给轻中性背景，不给完整品牌蓝边；选中后 hover 不覆盖选中组合。
            .when(!is_selected, |this| {
                this.border_color(cx.theme().border)
                    .hover(move |style| style.bg(hover_bg))
            })
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
                this.child(div().absolute().top_2().left_2().child(
                    connection_selection::connection_selection_checkbox(
                        &cx.entity(),
                        connection_selection::ConnectionCheckProps {
                            element_id: SharedString::from(format!("{card_id}-check")),
                            connection_id: conn_id.unwrap_or(0),
                            checked: is_selected,
                        },
                    ),
                ))
            })
            // 批量模式下左上角由勾选框占用，不再叠加在线状态点。
            .when(is_active && !batch_mode, |this| {
                this.child(
                    div()
                        .absolute()
                        .top(px(6.0))
                        .left(px(6.0))
                        .w(px(10.0))
                        .h(px(10.0))
                        .rounded_full()
                        .bg(cx.theme().success),
                )
            })
            .child(self.render_connection_card_content(&card_id, &conn, team_badge, cx))
            // actions 在 content 之后添加：GPUI 按添加顺序绘制，悬浮按钮必须
            // 位于团队徽标之上，否则徽标会截获 hover 并盖住按钮。
            .child(self.render_connection_card_actions(&card_id, &conn, can_edit, cx));
        match conn_id {
            Some(id) => card
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
            None => card.into_any_element(),
        }
    }
}
