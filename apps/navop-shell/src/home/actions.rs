//! 侧栏与顶栏动作：打开连接、打开各内置页签。
//!
//! 新建 / 编辑 / 复制 / 删除统一走 [`app_kit`] 的共享入口，这里不重复实现。

use domain_api::{HomeContext as _, NotifyLevel, TabIntent};
use gpui::{App, Window};
use one_core::storage::StoredConnection;
use one_core::tab_container::TabOpenMode;

impl super::ShellHome {
    /// 打开连接页签；领域未接入时给出可见提示，避免点击无反应。
    pub(super) fn open_connection(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !app_kit::registry().open_connection(
            &connection,
            TabOpenMode::Activate,
            self,
            window,
            cx,
        ) {
            self.notify(
                NotifyLevel::Warning,
                "该连接类型尚未接入 navop-shell",
                window,
                cx,
            );
        }
    }

    /// 打开 SFTP 页签；只有 SSH / FTP 类连接支持。
    pub(super) fn open_sftp(
        &mut self,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut App,
    ) {
        if !app_kit::registry().open_sftp(&connection, self, window, cx) {
            self.notify(NotifyLevel::Warning, "该连接不支持 SFTP", window, cx);
        }
    }

    /// 打开本地终端页签。
    pub(super) fn open_local_terminal(&mut self, window: &mut Window, cx: &mut App) {
        let intent = TabIntent::Terminal { profile: None };
        if !app_kit::registry().open_tab(&intent, TabOpenMode::Activate, self, window, cx) {
            self.notify(NotifyLevel::Warning, "本地终端尚未接入", window, cx);
        }
    }

    /// 打开 Known Hosts 页签。
    pub(super) fn open_known_hosts(&mut self, window: &mut Window, cx: &mut App) {
        let intent = TabIntent::KnownHosts;
        if !app_kit::registry().open_tab(&intent, TabOpenMode::Activate, self, window, cx) {
            self.notify(NotifyLevel::Warning, "Known Hosts 尚未接入", window, cx);
        }
    }

    /// 打开会话日志页签（shell 专属）。
    pub(super) fn open_session_logs(&mut self, window: &mut Window, cx: &mut App) {
        let intent = TabIntent::SessionLogs;
        if !app_kit::registry().open_tab(&intent, TabOpenMode::Activate, self, window, cx) {
            self.notify(NotifyLevel::Warning, "会话日志尚未接入", window, cx);
        }
    }
}
