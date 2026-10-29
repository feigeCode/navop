//! data 侧栏：动作工具栏、搜索框、连接列表与空列表引导。

use super::DataHome;
use super::dialogs::open_mcp_tools_dialog;
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

impl DataHome {
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
            .id("data-connection-list")
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
            .id("data-sidebar")
            .w(px(260.0))
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

    /// 「新建连接 / 设置 / MCP 工具」工具栏。
    fn render_toolbar(&self, cx: &mut Context<Self>) -> impl IntoElement {
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
                Button::new("data-settings")
                    .small()
                    .label("设置")
                    .on_click(|_, window, cx| open_settings(window, cx)),
            )
            .child(
                Button::new("mcp-tools")
                    .small()
                    .label("MCP 工具")
                    .on_click(cx.listener(|_, _, window, cx| {
                        open_mcp_tools_dialog(window, cx);
                    })),
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
                        "点击「新建连接」添加第一个 Database / Redis / MongoDB 连接"
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

    /// 单个连接行：名称/类型 + 打开按钮 + 右键编辑复制删除菜单。
    fn connection_row(
        &self,
        connection: StoredConnection,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let id = connection.id.unwrap_or_default();
        let open_connection = connection.clone();
        let menu_connection = connection.clone();
        div()
            .id(SharedString::from(format!("data-connection-row-{id}")))
            .flex()
            .flex_col()
            .gap_1()
            .p_2()
            .rounded_md()
            .hover(|style| style.bg(gpui::rgba(0x00000008)))
            .child(
                div()
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap_2()
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
                        Button::new(SharedString::from(format!("open-data-{id}")))
                            .small()
                            .label("打开")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.open_connection(open_connection.clone(), window, cx);
                            })),
                    ),
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

/// 连接类型的中文/英文短标签。
fn connection_type_label(kind: ConnectionType) -> &'static str {
    match kind {
        ConnectionType::Database => "Database",
        ConnectionType::Redis => "Redis",
        ConnectionType::MongoDB => "MongoDB",
        _ => "Unknown",
    }
}
