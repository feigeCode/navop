use super::*;

impl HomePage {
    /// 打开端口转发管理标签页。
    ///
    /// 标签页由终端领域包注册的 `ConnectionOpener` 构建，公共主页只做分发；
    /// 当前产品没有终端领域时会给出明确提示，而不是静默失败。
    pub(crate) fn open_port_forwarding_tab(
        &mut self,
        connection: StoredConnection,
        mode: TabOpenMode,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if app_kit::registry().open_connection(&connection, mode, self, window, cx) {
            return;
        }
        let message = t!(
            "Home.port_forwarding_unsupported",
            name = connection.name.clone()
        );
        window.push_notification(message.to_string(), cx);
    }
}
