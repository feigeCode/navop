//! shell 侧栏：动作工具栏、搜索框、连接列表与空列表引导。

use super::ShellHome;
use app_kit::connection_actions::{
    delete_connection, duplicate_connection, edit_connection, open_new_connection,
};
use app_kit::settings::open_settings;
use gpui::{Context, IntoElement, ParentElement, SharedString, Styled, div, prelude::*, px};
use gpui_component::Sizable as _;
use gpui_component::button::Button;
use gpui_component::input::Input;
use gpui_component::menu::{ContextMenuExt as _, PopupMenuItem};
use one_core::storage::{ConnectionType, StoredConnection};

impl ShellHome {
    /// 侧栏整体：工具栏 + 搜索 + 计数 + 可滚动列表（含空态）。
    pub(super) fn render_sidebar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.search_input.read(cx).value().trim().to_lowercase();
        let visible: Vec<StoredConnection> = self
            .connections
            .iter()
            .filter(|connection| super::matches_query(&connection.name, &query))
            .cloned()
            .collect();

        let mut list = div()
            .id("shell-connection-list")
            .flex_1()
            .min_h_0()
            .flex()
            .flex_col()
            .gap_1()
            .overflow_y_scroll();
        if visible.is_empty() {
            list = list.child(self.render_empty_state(&query));
        }
        for connection in visible {
            list = list.child(self.connection_row(connection, cx));
        }

        div()
            .id("shell-sidebar")
            .w(px(280.0))
            .h_full()
            .flex()
            .flex_col()
            .gap_2()
            .p_2()
            .border_r_1()
            .border_color(gpui::rgba(0x00000020))
            .child(self.render_toolbar(cx))
            .child(div().w_full().child(Input::new(&self.search_input)))
            .child(
                div()
                    .text_xs()
                    .text_color(gpui::rgba(0x666666ff))
                    .child(format!("连接（{}）", self.connections.len())),
            )
            .child(list)
    }

    /// 「新建连接 / 设置」与内置页签入口。
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .flex()
            .flex_col()
            .gap_1()
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new("new-connection")
                            .small()
                            .label("新建连接")
                            .on_click(|_, window, cx| open_new_connection(window, cx)),
                    )
                    .child(
                        Button::new("shell-settings")
                            .small()
                            .label("设置")
                            .on_click(|_, window, cx| open_settings(window, cx)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
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
                    )
                    .child(
                        Button::new("session-logs")
                            .small()
                            .label("会话日志")
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_session_logs(window, cx);
                            })),
                    ),
            )
    }

    /// 列表为空时的引导：无查询词给出「新建连接」，有查询词提示无匹配。
    fn render_empty_state(&self, query: &str) -> impl IntoElement {
        let mut hint =
            div()
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .rounded_md()
                .bg(gpui::rgba(0x00000008))
                .child(div().text_sm().child(if query.is_empty() {
                    "还没有连接"
                } else {
                    "没有匹配的连接"
                }))
                .child(div().text_xs().text_color(gpui::rgba(0x666666ff)).child(
                    if query.is_empty() {
                        "点击「新建连接」添加第一个 SSH / SFTP / 终端连接"
                    } else {
                        "换个关键字试试，或清空搜索框"
                    },
                ));
        if query.is_empty() {
            hint = hint.child(
                div().mt_1().child(
                    Button::new("empty-new-connection")
                        .small()
                        .label("新建连接")
                        .on_click(|_, window, cx| open_new_connection(window, cx)),
                ),
            );
        }
        hint
    }

    /// 单个连接行：名称/类型/工作区 + 打开类按钮 + 右键编辑复制删除菜单。
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
        let can_remote = matches!(
            connection.connection_type,
            ConnectionType::Rdp | ConnectionType::Vnc
        );
        let workspace = connection
            .workspace_id
            .and_then(|workspace_id| self.workspace_name(workspace_id));

        let open_connection = connection.clone();
        let open_sftp = connection.clone();
        let open_remote = connection.clone();
        let menu_connection = connection.clone();

        div()
            .id(SharedString::from(format!("connection-row-{id}")))
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
                    .gap_1()
                    .child(div().text_sm().child(connection.name.clone()))
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .gap_1()
                            .text_xs()
                            .text_color(gpui::rgba(0x666666ff))
                            .when_some(workspace, |line, name| line.child(name))
                            .child(connection.connection_type.to_string()),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_wrap()
                    .gap_1()
                    .child(
                        Button::new(SharedString::from(format!("open-{id}")))
                            .small()
                            .label("打开")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_connection(open_connection.clone(), window, cx);
                            })),
                    )
                    .when(can_sftp, |actions| {
                        actions.child(
                            Button::new(SharedString::from(format!("sftp-{id}")))
                                .small()
                                .label("SFTP")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_sftp(open_sftp.clone(), window, cx);
                                })),
                        )
                    })
                    .when(can_remote, |actions| {
                        actions.child(
                            Button::new(SharedString::from(format!("remote-{id}")))
                                .small()
                                .label("远程桌面")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.open_connection(open_remote.clone(), window, cx);
                                })),
                        )
                    }),
            )
            .context_menu(move |menu, _window, _cx| {
                menu.item(menu_item("编辑", menu_connection.clone(), edit_connection))
                    .item(menu_item(
                        "复制",
                        menu_connection.clone(),
                        duplicate_connection,
                    ))
                    .separator()
                    .item(menu_item(
                        "删除",
                        menu_connection.clone(),
                        delete_connection,
                    ))
            })
    }

    /// 查找工作区名称，用于行内展示归属。
    fn workspace_name(&self, workspace_id: i64) -> Option<String> {
        self.workspaces
            .iter()
            .find(|workspace| workspace.id == Some(workspace_id))
            .map(|workspace| workspace.name.clone())
    }
}

/// 构造一个「点击后把该连接交给 app_kit 动作」的菜单项。
fn menu_item(
    label: &'static str,
    connection: StoredConnection,
    action: fn(StoredConnection, &mut gpui::Window, &mut gpui::App),
) -> PopupMenuItem {
    PopupMenuItem::new(label).on_click(move |_, window, cx| {
        action(connection.clone(), window, cx);
    })
}
