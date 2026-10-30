use std::sync::Arc;

use gpui::{App, SharedString};

use crate::{PortForwardingRuntime, RemoteProtocol};
use one_core::storage::StoredConnection;

/// WSL 发行版信息（Windows 专用；其它平台为空列表）。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WslDistributionInfo {
    pub name: SharedString,
    pub is_default: bool,
}

/// shell 领域可选能力的只读视图：产品公共壳用它渲染菜单，不直接依赖 `terminal`。
///
/// 目前只覆盖 WSL 发行版列表；后续可继续添加端口转发会话状态等只读快照。
pub trait ShellExtras: Send + Sync {
    fn wsl_distributions(&self, _cx: &App) -> Option<Vec<WslDistributionInfo>> {
        None
    }

    fn refresh_wsl_distributions(&self, _cx: &mut App) {}

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
