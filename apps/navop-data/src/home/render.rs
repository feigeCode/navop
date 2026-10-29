//! data 主页渲染：顶栏、侧栏与内容区。

use super::DataHome;
use gpui::{AnyElement, App, Context, IntoElement, ParentElement, Render, Styled, Window, div, px};
use gpui_component::Sizable as _;
use gpui_component::button::Button;

impl DataHome {
    /// 内容区：有连接或已打开页签时显示页签容器，否则给首次使用引导。
    fn render_content(&self, cx: &App) -> AnyElement {
        if !self.connections.is_empty() || !self.tab_container.read(cx).tabs().is_empty() {
            return div()
                .size_full()
                .child(self.tab_container.clone())
                .into_any_element();
        }
        div()
            .size_full()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_2()
            .child(
                div()
                    .text_lg()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child("欢迎使用 Navop Data"),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(gpui::rgba(0x666666ff))
                    .child("还没有连接。创建第一个连接即可开始 Database / Redis / MongoDB。"),
            )
            .child(
                Button::new("welcome-new-connection")
                    .label("新建连接")
                    .on_click(|_, window, cx| app_kit::open_new_connection(window, cx)),
            )
            .into_any_element()
    }
}

impl Render for DataHome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
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
                                |_, _, cx| {
                                    let enabled = mcp_host::settings_enabled(cx);
                                    let _ = mcp_host::set_enabled(cx, !enabled);
                                },
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
                    .child(self.render_sidebar(cx))
                    .child(div().flex_1().min_w_0().child(self.render_content(cx))),
            )
    }
}
