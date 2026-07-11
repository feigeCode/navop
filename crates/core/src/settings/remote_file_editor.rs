use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteFileOpenMode {
    #[default]
    BuiltIn,
    External,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileEditorOverride {
    pub editor_key: String,
    pub program: String,
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileEditorUserSettings {
    #[serde(default)]
    pub open_mode: RemoteFileOpenMode,
    #[serde(default)]
    pub default_external_editor: Option<String>,
    #[serde(default = "default_auto_upload_external_changes")]
    pub auto_upload_external_changes: bool,
    #[serde(default = "default_conflict_check")]
    pub check_remote_modified_before_upload: bool,
    #[serde(default)]
    pub overrides: Vec<RemoteFileEditorOverride>,
}

impl Default for RemoteFileEditorUserSettings {
    fn default() -> Self {
        Self {
            open_mode: RemoteFileOpenMode::BuiltIn,
            default_external_editor: None,
            auto_upload_external_changes: true,
            check_remote_modified_before_upload: true,
            overrides: Vec::new(),
        }
    }
}

fn default_auto_upload_external_changes() -> bool {
    true
}

fn default_conflict_check() -> bool {
    true
}
