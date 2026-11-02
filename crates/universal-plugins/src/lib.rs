//! Application-owned lifecycle for universal resource plugins and their GPUI
//! views.
//!
//! 公共资源服务、原生连接 tab、连接表单不依赖 Shell feature,始终编译。
//! 只有 Shell 实现(shell_plugin_host / shell_plugin_tab / dev host ops)
//! 保持在 `shell-plugins` feature 之后。

// 扩展连接表单的国际化文案归本 crate 所有。
rust_i18n::i18n!("locales", fallback = "en");

mod connection_icon;
mod extension_connection_form;
mod extension_connection_tab;
mod extension_resource;
mod host_error;
#[cfg(feature = "nsh-host")]
mod nsh_host;
#[cfg(feature = "shell-plugins")]
mod shell_page_host;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_host;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_tab;
mod universal_plugins;

pub use connection_icon::{
    extension_connection_icon_or_default, resolve_extension_connection_icon,
};
pub use extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
pub use extension_connection_tab::ExtensionConnectionTab;
#[cfg(feature = "shell-plugins")]
pub use shell_plugin_host::{
    ConnectionShellOpen, DevHostOps, GlobalDevHostOps, ShellPluginHost, gpui_shell_reexport,
    refresh_shell_views, set_dev_host_ops,
};

#[cfg(feature = "nsh-host")]
pub use nsh_host::{NshHost, init as init_nsh_host};

/// 无 `shell-plugins` 构建里不存在脚本视图，没有东西可刷新。
///
/// 保留同名函数，是为了让调用点（设置页切语言那处）不必为这个 feature 再写
/// 一遍 `cfg` —— 那是把"哪个构建有 shell"这件事复制到调用方去。
#[cfg(not(feature = "shell-plugins"))]
pub fn refresh_shell_views(_cx: &mut gpui::App) {}

pub use universal_plugins::{
    GlobalUniversalPluginService, UniversalPluginService, init, spawn_shutdown,
};
