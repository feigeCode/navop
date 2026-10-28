rust_i18n::i18n!("locales", fallback = "en");

pub mod host_key_dialog;
pub mod ssh_form_mfa;
pub mod ssh_form_window;

pub use ssh_form_window::{SshFormPostSaveAction, SshFormWindow, SshFormWindowConfig};
