rust_i18n::i18n!("locales", fallback = "en");

pub mod keyboard;
mod modifiers;
mod native_cursor;
pub mod pixels;
pub mod pointer;
pub mod remote_desktop_form;
mod shortcuts;
pub mod view;
mod windows_native_shutdown;

pub use view::{RemoteDesktopView, RemoteDesktopViewConfig, init, refresh_keybindings};
pub use windows_native_shutdown::{WindowsNativeRdpShutdownReport, shutdown_windows_native_rdp};
