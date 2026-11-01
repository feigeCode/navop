rust_i18n::i18n!("locales", fallback = "en");

#[cfg(all(feature = "wasm-components", feature = "database"))]
mod action;
mod catalog;
pub mod connection_import_provider;
#[cfg(feature = "database")]
pub mod database_driver_install;
#[cfg(any(feature = "database", feature = "remote-desktop"))]
mod database_driver_install_progress;
pub mod extension;
#[cfg(feature = "database")]
mod extension_action_handler;
#[cfg(feature = "database")]
pub mod extension_db_gateway;
#[cfg(feature = "database")]
mod extension_db_gateway_conversion;
pub mod extension_downloader;
mod extension_package_layout;
mod extension_view_host;
pub mod global;
#[cfg(any(feature = "database", feature = "remote-desktop"))]
mod install_flow;
pub mod language_extensions;
mod mirror;
mod registration;
#[cfg(feature = "remote-desktop")]
pub mod remote_desktop_provider_install;
mod types;

pub use catalog::ExtensionRuntimeCatalog;
pub use extension::{init, manifest::set_current_host_version};
pub use extension_view_host::MainExtensionViewHost;
#[cfg(feature = "wasm-components")]
pub use extension_wasm::{DocumentRenderArtifact, DocumentRenderRequest, DocumentRenderTheme};
pub use global::{GlobalExtensionRuntimeCatalog, global_catalog, refresh_global_runtime_catalog};
pub use mirror::{extension_mirror, set_extension_mirror};
pub use types::{
    RegisteredDocumentExporter, RegisteredIpcRuntimeBinding, RegisteredRemoteFileEditorCommand,
    RegisteredRemoteFileEditorContribution, RegisteredResourceConnectionContribution,
    RegisteredResourceWorkbenchContribution, RegisteredShellViewContribution,
    WorkbenchOperationCatalog, WorkbenchOperationEntry, WorkbenchOperationParam,
};

#[cfg(all(test, feature = "wasm-components"))]
mod connection_import_provider_tests;
#[cfg(all(test, feature = "database"))]
mod database_driver_install_tests;
#[cfg(all(test, feature = "database"))]
mod extension_downloader_archive_tests;
#[cfg(all(test, feature = "database"))]
mod extension_downloader_network_tests;
#[cfg(test)]
mod extension_downloader_policy_tests;
#[cfg(all(test, feature = "database"))]
mod extension_downloader_tests;
#[cfg(all(test, feature = "database"))]
mod extension_runtime_contract_tests;
#[cfg(all(test, feature = "wasm-components", feature = "database"))]
mod extension_runtime_wasm_contract_tests;
