rust_i18n::i18n!("locales", fallback = "en");

pub mod common;
pub mod compare;
pub mod connection_form_window;
pub mod database_objects_tab;
pub mod database_tab;
mod database_toolbar;
mod database_users_list;
mod database_users_tab;
mod database_users_toolbar;
pub mod database_view_plugin;
pub mod db_object_selector;
mod db_tree_event;
pub mod db_tree_view;
mod driver_i18n;
pub mod er_diagram;
pub mod extension_menu;
#[cfg(test)]
mod extension_menu_contract_tests;
pub mod extension_selector;
pub mod extension_widget;
#[cfg(test)]
mod extension_widget_tests;
pub mod extension_widget_view;
mod extension_widget_view_controls;
mod import_export;
pub mod search_shortcut;
pub mod settings;
mod sidebar;
pub mod sql_editor;
#[cfg(test)]
mod sql_editor_completion_tests;
pub mod sql_editor_view;
pub(crate) mod sql_inline_completion;
pub mod sql_result_tab;
mod table_data;
pub mod table_data_tab;
pub mod table_designer_tab;

pub use ai_chat_view::{AskAiButton, emit_ask_ai_event, init_ask_ai_notifier};
pub use common::DatabaseFormEvent;
pub(crate) use driver_i18n::t_driver;
