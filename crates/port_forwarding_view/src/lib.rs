rust_i18n::i18n!("locales", fallback = "en");

mod form_window;
mod input_values;
mod persistence;
mod selects;
mod tab;
mod tab_activity;
mod tab_close;
mod tab_config;
mod tab_render;
mod tab_state;
mod view;

#[cfg(test)]
mod tab_contract_tests;

pub use form_window::{PortForwardingFormWindow, PortForwardingFormWindowConfig};
pub use tab::PortForwardingTab;
pub use tab_config::PortForwardingTabConfig;
