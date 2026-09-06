use super::*;

impl HomePage {
    pub(super) fn render_connection_card_content(
        &self,
        card_id: &str,
        conn: &StoredConnection,
        team_badge: Option<ConnectionTeamBadge>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let display_name = connection_display_name(conn);
        let name_tooltip: SharedString = display_name.clone().into();
        let connection_info = card_connection_info(conn);

        h_flex()
            .items_center()
            .gap_2()
            .w_full()
            .child(
                div()
                    .size(gpui::rems(2.5))
                    .rounded(cx.theme().radius)
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.connection_icon(conn, ConnectionVisualSize::Card)),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .overflow_hidden()
                    .child(
                        // 名称与团队标签同一行基线对齐；名称截断优先，标签有最大宽度（redesign §5.2）。
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .id(SharedString::from(format!("{card_id}-name")))
                                    .flex_1()
                                    .text_sm()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .text_color(cx.theme().foreground)
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .whitespace_nowrap()
                                    .min_w_0()
                                    .tooltip(move |window, cx| {
                                        Tooltip::new(name_tooltip.clone()).build(window, cx)
                                    })
                                    .child(display_name),
                            )
                            .when_some(team_badge, |this, badge| {
                                this.child(render_team_badge(card_id, conn, badge, cx))
                            }),
                    )
                    .when_some(connection_info, |this, info| {
                        let tooltip_text: SharedString = info.clone().into();
                        this.child(
                            div()
                                .id(SharedString::from(format!("{card_id}-info")))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .max_w_full()
                                .tooltip(move |window, cx| {
                                    Tooltip::new(tooltip_text.clone()).build(window, cx)
                                })
                                .child(info),
                        )
                    }),
            )
            .into_any_element()
    }
}
