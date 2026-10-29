use super::DataHome;
use gpui::{Context, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px};
use gpui_component::{Sizable as _, button::Button};
use one_core::storage::{ConnectionType, StoredConnection};

impl DataHome {
    fn connection_row(
        &self,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = connection.id.unwrap_or_default();
        let open_connection = connection.clone();
        div()
            .id(("data-connection-row", id as usize))
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .p_2()
            .rounded_md()
            .hover(|style| style.bg(gpui::rgba(0x00000008)))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .min_w_0()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(connection.name.clone()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(gpui::rgba(0x888888ff))
                            .child(connection_type_label(connection.connection_type)),
                    ),
            )
            .child(
                Button::new(("open-data-connection", id as usize))
                    .small()
                    .label("打开")
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.open_connection(open_connection.clone(), window, cx);
                    })),
            )
    }
}

impl Render for DataHome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .id("data-sidebar")
            .w(px(260.0))
            .h_full()
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .border_r_1()
            .border_color(gpui::rgba(0x00000020))
            .overflow_y_scroll()
            .child(
                div()
                    .mt_1()
                    .text_xs()
                    .text_color(gpui::rgba(0x666666ff))
                    .child(format!("连接（{}）", self.connections.len())),
            );

        for connection in self.connections.clone() {
            sidebar = sidebar.child(self.connection_row(connection, cx));
        }

        let mcp_running = mcp_host::is_running(cx);
        let mcp_enabled = mcp_host::settings_enabled(cx);
        let mcp_label = if mcp_running {
            "MCP 已连接"
        } else if mcp_enabled {
            "MCP 启动中"
        } else {
            "MCP 关闭"
        };

        div()
            .size_full()
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(44.0))
                    .flex()
                    .items_center()
                    .justify_between()
                    .px_3()
                    .border_b_1()
                    .border_color(gpui::rgba(0x00000020))
                    .child(
                        div()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Navop Data"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Button::new("mcp-toggle").small().label(mcp_label).on_click(
                                cx.listener(|_, _, _window, cx| {
                                    let enabled = mcp_host::settings_enabled(cx);
                                    let _ = mcp_host::set_enabled(cx, !enabled);
                                }),
                            ))
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(gpui::rgba(0x666666ff))
                                    .child("Database / Redis / MongoDB"),
                            ),
                    ),
            )
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .flex()
                    .child(sidebar)
                    .child(div().flex_1().min_w_0().child(self.tab_container.clone())),
            )
    }
}

fn connection_type_label(kind: ConnectionType) -> &'static str {
    match kind {
        ConnectionType::Database => "Database",
        ConnectionType::Redis => "Redis",
        ConnectionType::MongoDB => "MongoDB",
        _ => "Unknown",
    }
}
