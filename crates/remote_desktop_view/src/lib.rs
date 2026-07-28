rust_i18n::i18n!("locales", fallback = "en");

mod ime_guard;
pub mod keyboard;
mod modifiers;
pub mod pixels;
pub mod pointer;
pub mod remote_desktop_form;
mod shortcuts;
pub mod view;

pub use view::{RemoteDesktopView, RemoteDesktopViewConfig, init, refresh_keybindings};
