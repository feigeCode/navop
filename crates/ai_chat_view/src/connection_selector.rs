//! 连接选择器（简化版 v2 - 支持选择和切换）

use gpui::{
    AnyElement, App, ColorExt, Context, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};
use one_core::storage::{ConnectionType, StoredConnection};
use rust_i18n::t;

/// 连接选择器事件
#[derive(Clone, Debug)]
pub enum ConnectionSelectorEvent {
    /// 选择了新连接
    SelectionChanged { connection_id: i64 },
}

/// 连接选择器（当前为简化版：只读显示 + 事件系统）
pub struct ConnectionSelector {
    focus_handle: FocusHandle,
    connection: Option<StoredConnection>,
    readonly: bool,
}

impl EventEmitter<ConnectionSelectorEvent> for ConnectionSelector {}
impl Focusable for ConnectionSelector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ConnectionSelector {
    /// 创建新的连接选择器（只读模式）
    pub fn new(connection: Option<StoredConnection>, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        Self {
            focus_handle,
            connection,
            readonly: false,
        }
    }

    /// 创建只读模式的选择器（侧边栏）
    pub fn readonly(connection: Option<StoredConnection>, cx: &mut Context<Self>) -> Self {
        let mut selector = Self::new(connection, cx);
        selector.readonly = true;
        selector
    }

    /// 更新当前连接
    pub fn set_connection(&mut self, connection: StoredConnection, cx: &mut Context<Self>) {
        self.connection = Some(connection);
        cx.notify();
    }

    /// 获取当前连接
    pub fn connection(&self) -> Option<&StoredConnection> {
        self.connection.as_ref()
    }

    /// 渲染为只读徽章或按钮
    fn render_content(&self, cx: &App) -> AnyElement {
        if let Some(conn) = &self.connection {
            let icon = connection_type_icon(&conn.connection_type);
            let name = conn.name.clone();

            if self.readonly {
                // 只读徽章
                h_flex()
                    .gap_2()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().muted.opacity(0.3))
                    .child(icon.small().text_color(cx.theme().foreground))
                    .child(
                        v_flex()
                            .gap_0p5()
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("AgentUi.current_connection").to_string()),
                            )
                            .child(
                                div()
                                    .text_sm()
                                    .font_weight(gpui::FontWeight::MEDIUM)
                                    .text_color(cx.theme().foreground)
                                    .child(name),
                            ),
                    )
                    .into_any_element()
            } else {
                // 可点击按钮（未来扩展）
                Button::new("connection-btn")
                    .label(name)
                    .icon(icon)
                    .ghost()
                    .small()
                    .into_any_element()
            }
        } else {
            div()
                .child(t!("AgentUi.no_connection").to_string())
                .into_any_element()
        }
    }
}

impl Render for ConnectionSelector {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_content(cx)
    }
}

/// 获取连接类型的图标
fn connection_type_icon(conn_type: &ConnectionType) -> Icon {
    match conn_type {
        ConnectionType::Rdp => IconName::Rdp.color(),
        ConnectionType::Vnc => IconName::Vnc.color(),
        ConnectionType::Database => IconName::Database.mono(),
        ConnectionType::Redis => IconName::Database.mono(),
        ConnectionType::MongoDB => IconName::Database.mono(),
        ConnectionType::Mqtt => Icon::default().path(one_core::storage::NAVOP_MQTT_LINE_ICON),
        ConnectionType::SshSftp => IconName::Terminal.mono(),
        ConnectionType::Serial => IconName::SquareTerminal.mono(),
        ConnectionType::Telnet => IconName::SquareTerminal.mono(),
        ConnectionType::PortForwarding => IconName::Network.mono(),
        ConnectionType::Extension => IconName::ExtensionsLine.mono(),
        ConnectionType::All => IconName::GalleryVerticalEnd.mono(),
    }
}
