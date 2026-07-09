use gpui::App;

rust_i18n::i18n!("locales", fallback = "zh-CN");

pub mod agent;
pub mod cloud_sync;
pub mod command_registry;
pub mod config;
pub mod connection_notifier;
pub mod contributions;
pub mod crypto;
pub mod gpui_tokio;
pub mod key_storage;
pub mod keybindings;
pub mod layout;
pub mod license;
pub mod llm;
pub mod popup_window;
pub mod sidebar_contribution;
pub mod split_tab_container;
pub mod storage;
pub mod tab_actions;
pub mod tab_container;
pub mod tab_navigation;
pub mod tab_switcher;
pub mod text_diag;
// pub mod tab_persistence;
pub mod settings;
pub mod themes;
pub mod utils;
pub mod when_clause;

#[cfg(test)]
mod extension_core_contract_tests;
#[cfg(test)]
mod sidebar_contribution_tests;
#[cfg(test)]
mod tab_container_split_tests;
#[cfg(test)]
mod tab_content_contract_tests;

pub use crate::agent::{
    Agent, AgentContext, AgentDescriptor, AgentDispatcher, AgentEvent, AgentRegistry, AgentResult,
    SessionAffinity,
};
pub fn init(cx: &mut App) {
    gpui_tokio::init(cx);
    themes::init(cx);
    storage::init(cx);
    llm::init(cx);
    agent::init(cx);
    connection_notifier::init(cx);
    tab_container::init(cx);
}
