use super::*;

impl HomePage {
    pub(super) fn render_connection_list_item(
        &self,
        conn: StoredConnection,
        selected_id: Option<i64>,
        _index: usize,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let conn_id = conn.id;
        let open_connection = conn.clone();
        let is_selected = selected_id == conn.id;
        let is_active = conn
            .id
            .is_some_and(|id| cx.global::<ActiveConnections>().is_active(id));
        let can_edit = can_edit_connection_with_cached_teams(
            conn.team_id.as_deref(),
            &self.team_options,
            self.current_user.is_some(),
        );
        let team_badge = connection_team_badge(conn.team_id.as_deref(), &self.team_options);
        let actions = self.render_connection_list_actions(&conn, can_edit, cx);

        h_flex()
            .id(SharedString::from(format!(
                "conn-list-item-{}",
                conn.id.unwrap_or(0)
            )))
            .w_full()
            .h(px(64.0))
            .rounded(px(6.0))
            .bg(cx.theme().background)
            .px_3()
            .border_1()
            .items_center()
            .gap_3()
            .relative()
            .group("")
            .when(is_selected, |this| {
                this.border_color(cx.theme().list_active_border)
                    .border_l_3()
            })
            .when(!is_selected, |this| this.border_color(cx.theme().border))
            .cursor_pointer()
            .hover(|style| style.bg(cx.theme().muted))
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
                        .flex_shrink_0()
                        .w(px(8.0))
                        .h(px(8.0))
                        .rounded_full()
                        .bg(cx.theme().success),
                )
            })
            .child(self.connection_icon(&conn, px(24.0)).flex_shrink_0())
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
                                    .child(conn.name.clone()),
                            )
                            .when_some(team_badge, |this, badge| {
                                this.child(render_list_team_badge(&conn, badge, cx))
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
            )
            .into_any_element()
    }
}

fn render_list_team_badge(
    conn: &StoredConnection,
    badge: ConnectionTeamBadge,
    cx: &App,
) -> AnyElement {
    let tooltip_text: SharedString = badge.tooltip.into();
    let background = if badge.active {
        cx.theme().primary
    } else {
        cx.theme().muted
    };
    let foreground = if badge.active {
        cx.theme().primary_foreground
    } else {
        cx.theme().muted_foreground
    };
    div()
        .id(SharedString::from(format!(
            "conn-list-team-{}",
            conn.id.unwrap_or(0)
        )))
        .max_w(px(112.0))
        .px_1p5()
        .py_0p5()
        .rounded(px(4.0))
        .bg(background)
        .text_color(foreground)
        .text_xs()
        .overflow_hidden()
        .text_ellipsis()
        .whitespace_nowrap()
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .child(badge.name)
        .into_any_element()
}
