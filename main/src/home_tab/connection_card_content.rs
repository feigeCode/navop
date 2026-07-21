use super::*;

impl HomePage {
    pub(super) fn render_connection_card_content(
        &self,
        conn: &StoredConnection,
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
                    .h(px(40.0))
                    .w(px(40.0))
                    .rounded(px(6.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(self.connection_icon(conn, px(34.0))),
            )
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .overflow_hidden()
                    .child(
                        div()
                            .id(SharedString::from(format!(
                                "conn-name-{}",
                                conn.id.unwrap_or(0)
                            )))
                            .w_full()
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
                    .when_some(connection_info, |this, info| {
                        let tooltip_text: SharedString = info.clone().into();
                        this.child(
                            div()
                                .id(SharedString::from(format!(
                                    "conn-info-{}",
                                    conn.id.unwrap_or(0)
                                )))
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
