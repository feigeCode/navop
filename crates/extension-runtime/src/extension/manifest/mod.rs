pub mod contributes;
pub mod menus;
pub mod parser;
pub mod schema;
pub mod security;
mod security_rules;
mod shell_validation;
pub mod versioning;

pub use contributes::{
    CommandContrib, DocumentExporterContrib, DocumentRendererContrib, RemoteFileEditorLaunchMode,
    ResourceConnectionContrib, ResourceConnectionFieldType, ResourceConnectionForm,
    ResourceConnectionFormField, ResourceConnectionFormTab, ResourceConnectionSelectOption,
    ResourceConnectionVisibilityRule, ResourceWorkbenchAction, ResourceWorkbenchBinding,
    ResourceWorkbenchBindingSource, ResourceWorkbenchBottomRegion, ResourceWorkbenchBottomSource,
    ResourceWorkbenchCenterRegion, ResourceWorkbenchCenterSource, ResourceWorkbenchCollection,
    ResourceWorkbenchColumn, ResourceWorkbenchColumnStyle, ResourceWorkbenchContrib, ResourceWorkbenchEffect,
    ResourceWorkbenchInput, ResourceWorkbenchInputEditor, ResourceWorkbenchInputOption, ResourceWorkbenchLayout,
    ResourceWorkbenchLeftRegion, ResourceWorkbenchNavEntry, ResourceWorkbenchNavSource,
    ResourceWorkbenchOpen, ResourceWorkbenchOperation, ResourceWorkbenchOperationMode,
    ResourceWorkbenchPage, ResourceWorkbenchPagination, ResourceWorkbenchPaginationKind, ResourceWorkbenchRenderer,
    ResourceWorkbenchRendererKind, ResourceWorkbenchRowAction, ResourceWorkbenchShellSource,
    ResourceWorkbenchSideRegion, ResourceWorkbenchSideSource, ResourceWorkbenchStatusFormat,
    ResourceWorkbenchStatusItem,
    ResourceWorkbenchTab, ResourceWorkbenchTabGroup, ResourceWorkbenchTemplate,
    ResourceWorkbenchTerminal, ResourceWorkbenchTreeChildren, ResourceWorkbenchTreeRoot,
    ResourceWorkbenchValueType, ShellHostModule, ShellSurface, ShellViewContrib,
};
#[cfg(test)]
pub use contributes::{CommandHandlerContrib, ContributesManifest, HtmlPreviewTransformContrib};
#[cfg(test)]
pub use menus::{MenuCommandRef, MenuContrib};
pub(crate) use parser::required_spawn_permission;
pub use parser::{ManifestError, load_and_check, load_from_dir};
#[cfg(test)]
pub use schema::{
    ApiVersions, Engines, IpcEntry, IpcRuntime, IpcTransport, RuntimeSection, WasmRuntime,
};
pub use schema::{Manifest, WasmRuntimeKind};
pub use security::build_permission_review;
pub(crate) use shell_validation::validate_shell_views;
pub use versioning::{HostApiVersions, current_host_version, set_current_host_version};

#[cfg(test)]
mod parser_tests;
