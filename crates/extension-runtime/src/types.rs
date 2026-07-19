#[cfg(feature = "wasm-components")]
use std::path::PathBuf;
#[cfg(not(feature = "wasm-components"))]
use std::path::PathBuf;

use db_view::extension_menu::DbTreeExtensionMenuItem;
use one_core::{
    command_registry::{CommandDescriptor, CommandHandler, CommandRegistryError},
    contributions::{ContributionProvenance, SlotItem},
};
use serde_json::Value;

use crate::extension::manifest::{CommandContrib, RemoteFileEditorLaunchMode, WasmRuntimeKind};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRemoteFileEditorContribution {
    pub extension_id: String,
    pub id: String,
    pub editor_key: String,
    pub display_name: String,
    pub platforms: Vec<String>,
    pub file_masks: Vec<String>,
    pub priority: i32,
    pub command: RegisteredRemoteFileEditorCommand,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredRemoteFileEditorCommand {
    pub launch_mode: RemoteFileEditorLaunchMode,
    pub program_candidates: Vec<String>,
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredKeybindingContribution {
    pub extension_id: String,
    pub command: String,
    pub key: String,
    pub mac: Option<String>,
    pub linux: Option<String>,
    pub windows: Option<String>,
    pub when_clause: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct RegisteredDbTreeMenuContribution {
    pub position: String,
    pub item: DbTreeExtensionMenuItem,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredHtmlPreviewTransform {
    pub extension_id: String,
    pub id: String,
    pub runtime_id: String,
    pub function: String,
    pub languages: Vec<String>,
    pub assets_root: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredDocumentRenderer {
    pub extension_id: String,
    pub id: String,
    pub display_name: String,
    pub runtime_id: String,
    pub function: String,
    pub block_kinds: Vec<String>,
    pub output_media_types: Vec<String>,
    pub priority: i32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegisteredDocumentExporter {
    pub extension_id: String,
    pub id: String,
    pub display_name: String,
    pub runtime_id: String,
    pub function: String,
    pub formats: Vec<String>,
    pub output_media_types: Vec<String>,
    pub priority: i32,
}

#[derive(Debug, Clone)]
pub struct WasmRuntimeBinding {
    #[cfg(feature = "wasm-components")]
    pub extension_id: String,
    #[cfg(feature = "wasm-components")]
    pub runtime_key: String,
    pub kind: WasmRuntimeKind,
    #[cfg(feature = "wasm-components")]
    pub module_path: PathBuf,
    #[cfg(feature = "wasm-components")]
    pub config: extension_wasm::WasmRuntimeConfig,
    pub permissions: Vec<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ExtensionRuntimeError {
    #[error("duplicate wasm runtime id: {id}")]
    DuplicateRuntime { id: String },
    #[error("unknown runtime_id `{runtime_id}` for command `{command_id}`")]
    UnknownRuntime {
        command_id: String,
        runtime_id: String,
    },
    #[error(transparent)]
    CommandRegistry(#[from] CommandRegistryError),
    #[error("read composite extension root failed: {0}")]
    ReadCompositeRoot(std::io::Error),
    #[error("wasm command not found: {command_id}")]
    CommandNotFound { command_id: String },
    #[error("wasm runtime binding not found for command `{command_id}`: {runtime_id}")]
    RuntimeBindingNotFound {
        command_id: String,
        runtime_id: String,
    },
    #[error("command `{command_id}` is not a component wasm command")]
    UnsupportedCommand { command_id: String },
    #[error("invalid remote file editor `{editor_id}`: {reason}")]
    InvalidRemoteFileEditor { editor_id: String, reason: String },
}

pub(super) fn runtime_key(extension_id: &str, runtime_id: &str) -> String {
    format!("{extension_id}::{runtime_id}")
}

pub(super) fn command_descriptor(
    extension_id: &str,
    command: &CommandContrib,
    runtime_id: String,
    function: String,
) -> CommandDescriptor {
    let mut descriptor = CommandDescriptor::wasm(
        command.id.clone(),
        command.title.clone(),
        CommandHandler::wasm(runtime_id, function),
    )
    .with_extension(extension_id.to_string());
    if !command.category.is_empty() {
        descriptor = descriptor.with_category(command.category.clone());
    }
    if let Some(icon) = &command.icon {
        descriptor = descriptor.with_icon(icon.clone());
    }
    if let Some(when) = &command.enablement_when {
        descriptor = descriptor.with_enablement(when.clone());
    }
    descriptor
}

pub(super) fn slot_item_from_menu(
    extension_id: &str,
    command_id: String,
    label: Option<String>,
    group: Option<String>,
    when: Option<String>,
    args: Value,
) -> SlotItem {
    SlotItem {
        command: command_id,
        label,
        icon: None,
        group,
        when,
        args,
        provenance: ContributionProvenance {
            extension_id: extension_id.to_string(),
        },
    }
}
