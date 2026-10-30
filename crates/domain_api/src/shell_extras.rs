use std::sync::Arc;

use gpui::{App, SharedString, Window};

use crate::{HomeContext, PortForwardingRuntime, RemoteProtocol};
use one_core::storage::StoredConnection;
use one_core::tab_container::TabItem;

/// WSL 发行版信息（Windows 专用；其它平台为空列表）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslDistributionInfo {
    pub name: SharedString,
    pub is_default: bool,
}

/// 侧栏在终端主题下的配色（由 shell 领域按当前终端主题计算）。
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SidebarColors {
    pub background: gpui::Hsla,
    pub foreground: gpui::Hsla,
    pub muted: gpui::Hsla,
    pub muted_foreground: gpui::Hsla,
    pub border: gpui::Hsla,
    pub accent: gpui::Hsla,
}

/// shell 领域可选能力的只读视图：产品公共壳用它渲染菜单，不直接依赖 `terminal`。
///
/// 目前只覆盖 WSL 发行版列表；后续可继续添加端口转发会话状态等只读快照。
pub trait ShellExtras: Send + Sync {
    fn wsl_distributions(&self, _cx: &App) -> Option<Vec<WslDistributionInfo>> {
        None
    }

    fn refresh_wsl_distributions(&self, _cx: &mut App) {}

    /// 当前终端主题下的侧栏配色；返回 `None` 表示沿用应用主题。
    fn sidebar_colors(&self, _cx: &App) -> Option<SidebarColors> {
        None
    }

    /// 把临时连接保存为正式连接（打开 SSH 表单）；返回 `false` 表示不支持。
    fn save_temporary_connection(
        &self,
        _connection: StoredConnection,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        false
    }

    /// 打开终端会话日志文件；返回 `false` 表示不支持。
    fn open_session_log(
        &self,
        _path: std::path::PathBuf,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        false
    }

    /// 在指定目录打开终端：`connection` 为 `None` 时开本地终端，否则开该连接的 SSH 终端。
    fn open_terminal_in_dir(
        &self,
        _home: &mut dyn HomeContext,
        _working_dir: Option<String>,
        _connection: Option<StoredConnection>,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        false
    }

    /// 复制一个领域标签页（例如终端标签）；返回 `false` 表示不支持该标签。
    fn duplicate_terminal_tab(
        &self,
        _tab: &TabItem,
        _home: &mut dyn HomeContext,
        _window: &mut Window,
        _cx: &mut App,
    ) -> bool {
        false
    }

    /// 以内置视图全屏打开远程桌面；返回 `false` 表示不支持。
    fn open_remote_desktop_fullscreen(
        &self,
        _connection: &StoredConnection,
        _protocol: RemoteProtocol,
        _title: String,
        _cx: &mut App,
    ) -> bool {
        false
    }

    /// 调用系统远程桌面客户端（Windows 的 mstsc）；其它平台返回错误说明。
    fn launch_remote_desktop_external(&self, _connection: &StoredConnection) -> Result<(), String> {
        Err("当前产品不支持外部远程桌面客户端".to_string())
    }

    /// 端口转发运行时；`None` 表示当前产品不提供端口转发。
    fn port_forwarding_runtime(&self) -> Option<Arc<tokio::sync::Mutex<PortForwardingRuntime>>> {
        None
    }
}
