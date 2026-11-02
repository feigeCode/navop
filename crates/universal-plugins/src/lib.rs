//! Application-owned lifecycle for universal resource plugins and their GPUI
//! views.
//!
//! 公共资源服务、原生连接 tab、连接表单不依赖 UI 平面，始终编译；
//! 脚本页面的 UI 平面是 nsh（`nsh_host`，`nsh-host` feature）。

// 扩展连接表单的国际化文案归本 crate 所有。
rust_i18n::i18n!("locales", fallback = "en");

mod connection_icon;
mod extension_connection_form;
mod extension_connection_tab;
mod extension_resource;
mod host_error;
#[cfg(feature = "nsh-host")]
mod nsh_host;
mod universal_plugins;

pub use connection_icon::{
    extension_connection_icon_or_default, resolve_extension_connection_icon,
};
pub use extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
pub use extension_connection_tab::ExtensionConnectionTab;
#[cfg(feature = "nsh-host")]
pub use nsh_host::{ConnectionPageOpen, DevOps, NshHost, init as init_nsh_host, set_dev_ops};

/// 界面语言变化后刷新已打开的脚本页面。
///
/// 名字沿用 gpui-shell 时期的调用点（设置页切语言那一处）；实现换成了
/// nsh 的 `set_locale`（页面重渲染取词条），不再重跑整个脚本。
#[cfg(feature = "nsh-host")]
pub fn refresh_shell_views(cx: &mut gpui::App) {
    nsh_host::refresh_views(cx);
}

/// 不含脚本页面的构建里没有东西可刷新。
#[cfg(not(feature = "nsh-host"))]
pub fn refresh_shell_views(_cx: &mut gpui::App) {}

pub use universal_plugins::{
    GlobalUniversalPluginService, UniversalPluginService, init, spawn_shutdown,
};
