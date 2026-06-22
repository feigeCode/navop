rust_i18n::i18n!("locales", fallback = "en");

mod form_window;
mod input_values;
mod persistence;
mod selects;
mod view;

pub use form_window::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
