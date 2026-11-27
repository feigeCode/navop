//! Navop's nsh UI.
//!
//! This crate is the Rust UI twin of the existing GPUI shell. It deliberately
//! depends on domain/data crates, never on `navop_app`'s GPUI views: pages use
//! `nsh-core` primitives and `nsh-base` behaviors, while the application host
//! passes thread-safe services through nsh's [`EngineContext`].
//!
//! The migration is staged. The first surface contains the shell, connection
//! inventory, notes, JSON formatter, Known Hosts and system information. More
//! domains are added behind the same route shell instead of growing a second
//! navigation architecture.

mod extension_host;
mod extension_manager_host;
mod pages;
mod port_forwarding_host;
mod pty;
mod remote_desktop_host;
mod route;
mod services;
mod shell;
mod terminal_host;
mod ui;

use nsh_core::prelude::*;

pub use extension_host::ExtensionAppComponent;
pub use extension_manager_host::ExtensionManagerHost;
pub use port_forwarding_host::PortForwardingHost;
pub use remote_desktop_host::RemoteDesktopHost;
pub use route::Route;
pub use services::{NavopExtensionInfo, NavopNshServices};
pub use terminal_host::TerminalHost;

/// The app's UI set (`navop/native`).
#[native_module("navop/native")]
fn native(registry: &mut Registry) {
    registry.register::<NavopNshApp>();
}

/// Navop's nsh root component.
#[component]
pub fn NavopNshApp() -> impl IntoElement {
    ui::initialize();
    view()
        .size_full()
        .flex_row()
        .bg(move || nsh_base::theme().background)
        .child(shell::sidebar())
        .child(shell::content())
}
