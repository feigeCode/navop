use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::menus::{MenuCommandRef, MenuContrib};

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct ContributesManifest {
    #[serde(default)]
    pub languages: Vec<LanguageContrib>,
    #[serde(default, rename = "connectionImporters")]
    pub connection_importers: Vec<ConnectionImporterContrib>,
    #[serde(default)]
    pub drivers: Vec<Value>,
    #[serde(default)]
    pub connections: Vec<Value>,
    #[serde(default)]
    pub commands: Vec<CommandContrib>,
    #[serde(default)]
    pub menus: BTreeMap<String, Vec<MenuContrib>>,
    #[serde(default)]
    pub toolbars: BTreeMap<String, Vec<ToolbarContrib>>,
    #[serde(default)]
    pub keybindings: Vec<KeybindingContrib>,
    #[serde(default, rename = "htmlPreviewTransforms")]
    pub html_preview_transforms: Vec<HtmlPreviewTransformContrib>,
    #[serde(default, rename = "documentRenderers")]
    pub document_renderers: Vec<DocumentRendererContrib>,
    #[serde(default, rename = "remoteFileEditors")]
    pub remote_file_editors: Vec<RemoteFileEditorContrib>,
    #[serde(default)]
    pub views: Vec<Value>,
    #[serde(default)]
    pub tasks: Vec<Value>,
    #[serde(default)]
    pub data_types: Vec<Value>,
    #[serde(default)]
    pub sidebar: Vec<Value>,
    #[serde(default)]
    pub tabs: Vec<Value>,
    #[serde(default)]
    pub forms: Vec<Value>,
    #[serde(default)]
    pub transforms: Vec<Value>,
    #[serde(default)]
    pub completions: Vec<Value>,
    #[serde(default)]
    pub themes: Vec<Value>,
    #[serde(default)]
    pub icons: Vec<Value>,
}

impl ContributesManifest {
    pub fn total_count(&self) -> usize {
        self.languages.len()
            + self.connection_importers.len()
            + self.drivers.len()
            + self.connections.len()
            + self.commands.len()
            + self.menus.len()
            + self.toolbars.len()
            + self.keybindings.len()
            + self.html_preview_transforms.len()
            + self.document_renderers.len()
            + self.remote_file_editors.len()
            + self.views.len()
            + self.tasks.len()
            + self.data_types.len()
            + self.sidebar.len()
            + self.tabs.len()
            + self.forms.len()
            + self.transforms.len()
            + self.completions.len()
            + self.themes.len()
            + self.icons.len()
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteFileEditorContrib {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default, rename = "fileMasks")]
    pub file_masks: Vec<String>,
    #[serde(default)]
    pub priority: i32,
    pub command: RemoteFileEditorCommandContrib,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct RemoteFileEditorCommandContrib {
    #[serde(default, rename = "launchMode")]
    pub launch_mode: RemoteFileEditorLaunchMode,
    #[serde(default, rename = "programCandidates")]
    pub program_candidates: Vec<String>,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileEditorLaunchMode {
    #[default]
    Direct,
    MacosOpen,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct HtmlPreviewTransformContrib {
    pub id: String,
    #[serde(default, rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(default = "default_html_transform_function")]
    pub function: String,
    #[serde(default)]
    pub languages: Vec<String>,
    #[serde(default)]
    pub assets: String,
}

fn default_html_transform_function() -> String {
    "transform-html".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct DocumentRendererContrib {
    pub id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default, rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(default = "default_document_render_function")]
    pub function: String,
    #[serde(default, rename = "blockKinds")]
    pub block_kinds: Vec<String>,
    #[serde(default, rename = "outputMediaTypes")]
    pub output_media_types: Vec<String>,
    #[serde(default)]
    pub priority: i32,
}

fn default_document_render_function() -> String {
    "render-document".to_string()
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ConnectionImporterContrib {
    pub id: String,
    #[serde(default, rename = "runtimeId")]
    pub runtime_id: String,
    #[serde(rename = "displayName")]
    pub display_name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default, rename = "outputKinds")]
    pub output_kinds: Vec<String>,
    #[serde(default)]
    pub platforms: Vec<String>,
    #[serde(default, rename = "manualFilePick")]
    pub manual_file_pick: ManualFilePickContrib,
    #[serde(default, rename = "candidateFiles")]
    pub candidate_files: Vec<CandidateFileContrib>,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ManualFilePickContrib {
    #[serde(default)]
    pub prompt: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CandidateFileContrib {
    pub id: String,
    #[serde(default)]
    pub platform: String,
    pub path: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct LanguageContrib {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub path: String,
    #[serde(default)]
    pub file_extensions: Vec<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ToolbarContrib {
    pub command: MenuCommandRef,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
    #[serde(default)]
    pub text_when: Option<String>,
    #[serde(default)]
    pub icon_only: bool,
    #[serde(default)]
    pub priority: Option<i32>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct KeybindingContrib {
    pub command: String,
    pub key: String,
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default)]
    pub linux: Option<String>,
    #[serde(default)]
    pub windows: Option<String>,
    #[serde(default)]
    pub when: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandContrib {
    pub id: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub category: String,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub enablement_when: Option<String>,
    #[serde(default)]
    pub handler: CommandHandlerContrib,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct CommandHandlerContrib {
    #[serde(default = "default_command_handler_kind")]
    pub kind: String,
    #[serde(default)]
    pub runtime_id: String,
    #[serde(default = "default_command_function")]
    pub function: Option<String>,
}

impl Default for CommandHandlerContrib {
    fn default() -> Self {
        Self {
            kind: default_command_handler_kind(),
            runtime_id: String::new(),
            function: default_command_function(),
        }
    }
}

fn default_command_handler_kind() -> String {
    "builtin".to_string()
}

fn default_command_function() -> Option<String> {
    Some("invoke".to_string())
}
