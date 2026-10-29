use super::ShellHome;
use gpui::{App, Context, IntoElement, ParentElement, Render, Styled, Window, div, prelude::*, px};
use gpui_component::{Sizable as _, WindowExt as _, button::Button, checkbox::Checkbox};
use one_core::storage::{ConnectionType, StoredConnection};

impl ShellHome {
    fn connection_row(
        &self,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = connection.id.unwrap_or_default();
        let can_sftp = matches!(
            connection.connection_type,
            ConnectionType::SshSftp | ConnectionType::Ftp
        );
        let open_connection = connection.clone();
        let open_sftp = connection.clone();
        let open_remote = connection.clone();
        div()
            .id(("connection-row", id as usize))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .hover(|style| style.bg(gpui::rgba(0x00000010)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(div().text_sm().child(connection.name.clone()))
                    .child(
                        div()
                            .text_xs()
                            .text_color(gpui::rgba(0x666666ff))
                            .child(connection.connection_type.to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .gap_1()
                    .child(
                        Button::new(gpui::SharedString::from(format!("open-{id}")))
                            .small()
                            .label("打开")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_connection(open_connection.clone(), window, cx);
                            })),
                    )
                    .when(can_sftp, |row| {
                        row.child(
                            Button::new(gpui::SharedString::from(format!("sftp-{id}")))
                                .small()
                                .label("SFTP")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_sftp(open_sftp.clone(), window, cx);
                                })),
                        )
                    })
                    .when(
                        matches!(
                            connection.connection_type,
                            ConnectionType::Rdp | ConnectionType::Vnc
                        ),
                        |row| {
                            row.child(
                                Button::new(gpui::SharedString::from(format!("remote-{id}")))
                                    .small()
                                    .label("远程桌面")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.open_connection(open_remote.clone(), window, cx);
                                    })),
                            )
                        },
                    ),
            )
    }
}

impl Render for ShellHome {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut sidebar = div()
            .id("shell-sidebar")
            .w(px(280.0))
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
                    .flex()
                    .gap_1()
                    .child(
                        Button::new("new-local-terminal")
                            .small()
                            .label("本地终端")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_local_terminal(window, cx);
                            })),
                    )
                    .child(
                        Button::new("known-hosts")
                            .small()
                            .label("Known Hosts")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_known_hosts(window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .mt_2()
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
                            .child("Navop Shell"),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(Button::new("mcp-tools").small().label("MCP 工具").on_click(
                                cx.listener(|_, _, window, cx| {
                                    open_mcp_tools_dialog(window, cx);
                                }),
                            ))
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
                                    .child("SSH / SFTP / 终端 / 端口转发 / 远程桌面"),
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

fn open_mcp_tools_dialog(window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, |dialog, _window, cx| {
        dialog
            .title("MCP 工具集")
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_2()
                    .child(
                        Checkbox::new("mcp-toolset-terminal")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Terminal,
                            ))
                            .label("终端")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Terminal,
                                    *checked,
                                );
                            }),
                    )
                    .child(
                        Checkbox::new("mcp-toolset-sftp")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Sftp,
                            ))
                            .label("SFTP")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Sftp,
                                    *checked,
                                );
                            }),
                    )
                    .child(
                        Checkbox::new("mcp-toolset-connections")
                            .checked(mcp_host::current_toolset_enabled(
                                cx,
                                mcp_host::McpToolset::Connections,
                            ))
                            .label("连接管理")
                            .on_click(|checked, _, cx| {
                                let _ = mcp_host::set_toolset_enabled(
                                    cx,
                                    mcp_host::McpToolset::Connections,
                                    *checked,
                                );
                            }),
                    ),
            )
            .confirm()
    });
}
