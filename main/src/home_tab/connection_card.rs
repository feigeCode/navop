use super::*;

impl HomePage {
    pub(super) fn render_connection_card(
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
        let team_badge = if cfg!(feature = "screenshot-safe") {
            None
        } else {
            connection_team_badge(conn.team_id.as_deref(), &self.team_options)
        };

        v_flex()
            .justify_center()
            .id(SharedString::from(format!(
                "conn-card-{}",
                conn.id.unwrap_or(0)
            )))
            .w_full()
            .h(px(76.0))
            .rounded(px(6.0))
            .bg(cx.theme().background)
            .px_3()
            .py_2()
            .border_1()
            .relative()
            .overflow_hidden()
            .group("")
            .when(is_selected, |this| {
                this.border_color(cx.theme().list_active_border)
                    .shadow_md()
                    .border_l_3()
            })
            .when(!is_selected, |this| this.border_color(cx.theme().border))
            .cursor_pointer()
            .hover(|style| {
                style
                    .shadow_sm()
                    .border_color(cx.theme().list_active_border)
            })
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
                        .bg(cx.theme().success)
                        .shadow_lg(),
                )
            })
            .when_some(team_badge, |this, badge| {
                this.child(render_card_team_badge(&conn, badge, cx))
            })
            .child(self.render_connection_card_actions(&conn, can_edit, cx))
            .child(self.render_connection_card_content(&conn, cx))
            .into_any_element()
    }
}

fn render_card_team_badge(
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
            "conn-team-{}",
            conn.id.unwrap_or(0)
        )))
        .absolute()
        .top_2()
        .right_2()
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
        .group_hover("", |style| style.opacity(0.0))
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .child(badge.name)
        .into_any_element()
}
