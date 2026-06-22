rust_i18n::i18n!("locales", fallback = "en");

mod ime_guard;
pub mod keyboard;
mod modifiers;
pub mod pixels;
pub mod pointer;
mod shortcuts;
pub mod view;
pub mod remote_desktop_form;

pub use view::{init, refresh_keybindings, RemoteDesktopView, RemoteDesktopViewConfig};
