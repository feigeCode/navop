use gpui::{App, SharedString};

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
}
