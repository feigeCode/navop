mod action;
mod catalog;
pub mod database_driver_install;
pub mod extension;
mod extension_action_handler;
pub mod extension_db_gateway;
pub mod extension_downloader;
mod extension_view_host;
mod global;
mod registration;
mod types;

pub use catalog::ExtensionRuntimeCatalog;
pub use extension::init;
pub use extension_view_host::MainExtensionViewHost;
pub use global::{GlobalExtensionRuntimeCatalog, refresh_global_runtime_catalog};

#[cfg(test)]
mod database_driver_install_tests;
#[cfg(test)]
mod extension_downloader_archive_tests;
#[cfg(test)]
mod extension_downloader_network_tests;
#[cfg(test)]
mod extension_downloader_policy_tests;
#[cfg(test)]
mod extension_downloader_tests;
#[cfg(test)]
mod extension_runtime_contract_tests;
#[cfg(test)]
mod extension_runtime_wasm_contract_tests;
