pub mod contributes;
pub mod menus;
pub mod parser;
pub mod schema;
pub mod security;
mod security_rules;
pub mod versioning;

pub use contributes::CommandContrib;
#[cfg(test)]
pub use contributes::{CommandHandlerContrib, ContributesManifest};
#[cfg(test)]
pub use menus::{MenuCommandRef, MenuContrib};
pub use parser::{ManifestError, load_and_check, load_from_dir};
#[cfg(test)]
pub use schema::{ApiVersions, Engines, RuntimeSection, WasmRuntime};
pub use schema::{Manifest, WasmRuntimeKind};
pub use security::build_permission_review;
pub use versioning::{HostApiVersions, current_host_version};

#[cfg(test)]
mod parser_tests;
