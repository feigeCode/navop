//! 终端侧边栏的扩展面板注入点。
//!
//! terminal_view 不依赖扩展运行时（extension-runtime / universal-plugins），
//! 扩展面板由宿主（navop_app）在启动时经 [`TerminalExtensionPanelHost`]
//! 注入：宿主负责清单过滤（anchor/when）、图标解析与面板视图的创建；
//! 终端侧栏只拿到“已过滤的面板规格”负责渲染与停靠。
//!
//! 参照 `broadcast_registry` 的全局注册表模式。

use std::path::PathBuf;
use std::sync::Arc;

use gpui::{AnyView, App, SharedString};
use one_core::sidebar_contribution::SidebarPlacement;

use crate::TerminalColors;

/// 扩展面板规格：宿主已按 anchor/when 过滤、按 order 排序后的条目。
#[derive(Clone, Debug)]
pub struct ExtensionPanelSpec {
    /// 面板全局唯一键，宿主用 `"{extension_id}:{panel_id}"` 形式。
    pub id: String,
    /// 工具栏与面板标题显示的标题。
    pub title: SharedString,
    /// 面板图标资产的绝对路径（可选）。
    pub icon_path: Option<PathBuf>,
    /// 排序权重（清单 `panels[].order`，越小越靠前）。
    pub order: i64,
    /// 默认停靠位置（清单 `panels[].placement`），只在面板首次出现时生效；
    /// 用户之后手动移动过的位置优先。
    pub placement: SidebarPlacement,
}

/// 终端扩展面板宿主：由 navop_app 实现。
pub trait TerminalExtensionPanelHost: Send + Sync {
    /// 当前可用的扩展面板（已按 when 过滤、按 order 排序）。
    fn panels(&self, cx: &App) -> Vec<ExtensionPanelSpec>;

    /// 打开（或创建）指定面板的视图。首次打开时创建，之后复用同一实体。
    ///
    /// `colors` 是终端自己的色板：面板要按它渲染，跟终端看起来是一体的。
    fn open_panel(&self, id: &str, colors: &TerminalColors, cx: &mut App) -> Option<AnyView>;
}

/// 全局注册表：持有宿主实现。
struct GlobalTerminalExtensionPanels(Arc<dyn TerminalExtensionPanelHost>);

impl gpui::Global for GlobalTerminalExtensionPanels {}

/// 注册终端扩展面板宿主（navop_app 启动时调用一次）。
pub fn init_terminal_extension_panels(host: Arc<dyn TerminalExtensionPanelHost>, cx: &mut App) {
    cx.set_global(GlobalTerminalExtensionPanels(host));
}

/// 读取当前注册的扩展面板规格（未注册返回空）。
pub fn terminal_extension_panel_specs(cx: &App) -> Vec<ExtensionPanelSpec> {
    match cx.try_global::<GlobalTerminalExtensionPanels>() {
        Some(global) => global.0.panels(cx),
        None => Vec::new(),
    }
}

/// 请求宿主打开指定扩展面板视图（未注册返回 None）。
pub fn open_terminal_extension_panel(
    id: &str,
    colors: &TerminalColors,
    cx: &mut App,
) -> Option<AnyView> {
    let host = cx
        .try_global::<GlobalTerminalExtensionPanels>()
        .map(|global| global.0.clone())?;
    host.open_panel(id, colors, cx)
}
