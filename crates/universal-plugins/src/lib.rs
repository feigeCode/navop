//! Application-owned lifecycle for universal resource plugins and their GPUI
//! views (shell views and headless connection tabs).
//!
//! This crate is gated behind the `shell-plugins` feature: without it (the
//! default) the modules below are not compiled and the crate is empty.

#[cfg(feature = "shell-plugins")]
mod extension_connection_form;
#[cfg(feature = "shell-plugins")]
mod extension_connection_tab;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_host;
#[cfg(feature = "shell-plugins")]
mod shell_plugin_tab;
#[cfg(feature = "shell-plugins")]
mod universal_plugins;

#[cfg(feature = "shell-plugins")]
pub use extension_connection_form::{ExtensionConnectionForm, ExtensionConnectionFormConfig};
#[cfg(feature = "shell-plugins")]
pub use extension_connection_tab::ExtensionConnectionTab;
#[cfg(feature = "shell-plugins")]
pub use shell_plugin_host::{
    ConnectionShellOpen, DevHostOps, GlobalDevHostOps, ShellPluginHost, gpui_shell_reexport,
    set_dev_host_ops,
};
#[cfg(feature = "shell-plugins")]
pub use universal_plugins::{
    GlobalUniversalPluginService, UniversalPluginService, init, spawn_shutdown,
};
