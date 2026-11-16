//! Application-owned lifecycle for universal resource plugins and their GPUI
//! views.
//!
//! 公共资源服务、原生连接 tab、连接表单不依赖 UI 平面，始终编译；
//! 脚本页面的 UI 平面是 nsh（`nsh_host`，`nsh-host` feature）。

// 扩展连接表单的国际化文案归本 crate 所有。
rust_i18n::i18n!("locales", fallback = "en");

mod connection_icon;
pub mod extension_connection_form;
mod extension_resource;
pub mod extension_tools;
#[cfg(feature = "nsh-host")]
mod host_error;
#[cfg(feature = "nsh-host")]
mod nsh_host;
mod universal_plugins;

pub use connection_icon::{
    extension_connection_icon_or_default, resolve_extension_connection_icon,
};
pub use extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
#[cfg(feature = "nsh-host")]
pub use nsh_host::{
    ConnectionPageOpen, DevOps, NativeFactory, NativeViewContext, NshHost, NshPanel,
    init as init_nsh_host, register_native_component, set_dev_ops,
};
#[cfg(feature = "nsh-db")]
pub use nsh_host::set_db_state as set_nsh_db_state;
#[cfg(feature = "nsh-db")]
pub use nsh_host::set_tree_refresh_hook as set_nsh_db_tree_refresh;
/// 把 navop 的能力面（db / terminal / app / ui）注册成 nsh 的 nativeModule。
///
/// 宿主在创建任何 `AppView` 之前调用一次：它同时装 `nsh/base`，因为
/// `set_ui_sets` 是替换语义。
#[cfg(feature = "nsh-host")]
pub use nsh_host::install_native_api;
#[cfg(all(feature = "nsh-host", feature = "nsh-terminal"))]
pub use nsh_host::nsh_terminal_theme;

/// 界面语言变化后刷新已打开的脚本页面。
///
/// 名字沿用 gpui-shell 时期的调用点（设置页切语言那一处）；实现换成了
/// nsh 的 `set_locale`（页面重渲染取词条），不再重跑整个脚本。
#[cfg(feature = "nsh-host")]
pub fn refresh_shell_apps(cx: &mut gpui::App) {
    nsh_host::refresh_views(cx);
}

/// 不含脚本页面的构建里没有东西可刷新。
#[cfg(not(feature = "nsh-host"))]
pub fn refresh_shell_apps(_cx: &mut gpui::App) {}

pub use universal_plugins::{
    GlobalUniversalPluginService, UniversalPluginService, init, init_for_tests, spawn_shutdown,
};
