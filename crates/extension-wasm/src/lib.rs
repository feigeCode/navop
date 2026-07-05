pub mod bindings;
mod component;
mod component_host;
mod config;
mod connection_import;
pub mod connection_import_bindings;
mod error;
mod host_conversions;
pub mod html_preview_bindings;
mod html_preview_transform;

pub use component::{ComponentHostState, ComponentRuntime};
pub use config::WasmRuntimeConfig;
pub use connection_import::{ConnectionImportComponentRuntime, ConnectionImportHostState};
pub use error::{WasmError, WasmResult};
pub use html_preview_transform::HtmlPreviewTransformRuntime;

#[cfg(test)]
#[path = "component_coverage_tests.rs"]
mod component_coverage_tests;
#[cfg(test)]
#[path = "component_db_host_tests.rs"]
mod component_db_host_tests;
#[cfg(test)]
#[path = "component_import_shape_tests.rs"]
mod component_import_shape_tests;
#[cfg(test)]
#[path = "component_runtime_tests.rs"]
mod component_runtime_tests;
#[cfg(test)]
#[path = "component_ui_import_tests.rs"]
mod component_ui_import_tests;
#[cfg(test)]
#[path = "connection_import_tests.rs"]
mod connection_import_tests;
#[cfg(test)]
#[path = "html_preview_transform_tests.rs"]
mod html_preview_transform_tests;
#[cfg(test)]
mod test_support;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
