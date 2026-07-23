rust_i18n::i18n!("locales", fallback = "en");

#[cfg(feature = "wasm-components")]
mod action;
mod catalog;
pub mod connection_import_provider;
pub mod database_driver_install;
mod database_driver_install_progress;
pub mod extension;
mod extension_action_handler;
pub mod extension_db_gateway;
pub mod extension_downloader;
mod extension_package_layout;
mod extension_view_host;
mod global;
pub mod mcp_helper_install;
mod registration;
pub mod remote_desktop_provider_install;
mod types;

pub use catalog::ExtensionRuntimeCatalog;
pub use extension::{init, manifest::set_current_host_version};
pub use extension_view_host::MainExtensionViewHost;
#[cfg(feature = "wasm-components")]
pub use extension_wasm::{DocumentRenderArtifact, DocumentRenderRequest, DocumentRenderTheme};
pub use global::{GlobalExtensionRuntimeCatalog, refresh_global_runtime_catalog};
pub use types::{
    RegisteredDocumentExporter, RegisteredRemoteFileEditorCommand,
    RegisteredRemoteFileEditorContribution,
};

#[cfg(all(test, feature = "wasm-components"))]
mod connection_import_provider_tests;
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
#[cfg(all(test, feature = "wasm-components"))]
mod extension_runtime_wasm_contract_tests;
