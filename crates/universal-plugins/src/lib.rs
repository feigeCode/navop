//! Application-owned lifecycle for universal resource plugins and their GPUI
//! views.
//!
//! 公共资源服务、原生连接 tab、连接表单不依赖 Shell feature,始终编译。
//! 只有 Shell 实现(shell_plugin_host / shell_plugin_tab / dev host ops)
//! 保持在 `shell-plugins` feature 之后。

// 扩展连接表单的国际化文案归本 crate 所有。
rust_i18n::i18n!("locales", fallback = "en");

mod extension_connection_form;
mod extension_connection_tab;
mod extension_resource;
#[cfg(feature = "shell-plugins")]
mod shell_page_host;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_host;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_tab;
mod universal_plugins;

pub use extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
pub use extension_connection_tab::ExtensionConnectionTab;
#[cfg(feature = "shell-plugins")]
pub use shell_plugin_host::{
    ConnectionShellOpen, DevHostOps, GlobalDevHostOps, ShellPluginHost, gpui_shell_reexport,
    set_dev_host_ops,
};
pub use universal_plugins::{
    GlobalUniversalPluginService, UniversalPluginService, init, spawn_shutdown,
};
