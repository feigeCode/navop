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
        let is_selected = selected_id == conn.id;
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
        let hover_border = cx.theme().list_active_border;
        let hover_bg = cx.theme().muted.opacity(0.5);
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
            .when(!is_selected, |this| this.border_color(cx.theme().border))
            .cursor_pointer()
            // 悬停只给轻背景与边框变化，不使用阴影抬升（redesign §5.5）。
            .hover(move |style| style.bg(hover_bg).border_color(hover_border))
            .on_double_click(cx.listener(move |this, _, window, cx| {
                this.open_connection_from_quick(&open_connection, window, cx);
                cx.notify()
            }))
            .on_click(cx.listener(move |this, _, _, cx| {
                this.selected_connection_id = conn_id;
                cx.notify();
            }))
            .when(is_active, |this| {
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
            .child(self.render_connection_card_actions(&card_id, &conn, can_edit, cx))
            .child(self.render_connection_card_content(&card_id, &conn, team_badge, cx));
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
