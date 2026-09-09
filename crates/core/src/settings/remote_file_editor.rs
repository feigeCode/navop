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
    #[serde(default = "default_max_file_size_mib")]
    pub max_file_size_mib: u32,
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
            max_file_size_mib: default_max_file_size_mib(),
            open_mode: RemoteFileOpenMode::BuiltIn,
            default_external_editor: None,
            auto_upload_external_changes: true,
            check_remote_modified_before_upload: true,
            overrides: Vec::new(),
        }
    }
}

impl RemoteFileEditorUserSettings {
    pub const MIN_FILE_SIZE_MIB: u32 = 1;
    pub const MAX_FILE_SIZE_MIB: u32 = 1024;

    pub fn max_file_size_bytes(&self) -> usize {
        self.max_file_size_mib
            .clamp(Self::MIN_FILE_SIZE_MIB, Self::MAX_FILE_SIZE_MIB) as usize
            * 1024
            * 1024
    }
}

fn default_max_file_size_mib() -> u32 {
    10
}

fn default_auto_upload_external_changes() -> bool {
    true
}

fn default_conflict_check() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_file_size_defaults_for_existing_settings() {
        let settings: RemoteFileEditorUserSettings = serde_json::from_str("{}").unwrap();
        assert_eq!(10 * 1024 * 1024, settings.max_file_size_bytes());
        assert_eq!(RemoteFileEditorUserSettings::default(), settings);
    }

    #[test]
    fn remote_file_size_custom_setting_round_trips() {
        let settings: RemoteFileEditorUserSettings =
            serde_json::from_str(r#"{"max_file_size_mib":20,"open_mode":"external"}"#).unwrap();
        assert_eq!(20 * 1024 * 1024, settings.max_file_size_bytes());
        let restored = serde_json::from_str::<RemoteFileEditorUserSettings>(
            &serde_json::to_string(&settings).unwrap(),
        )
        .unwrap();
        assert_eq!(settings, restored);
    }

    #[test]
    fn remote_file_size_bounds_prevent_zero_and_overflow() {
        for (value, expected) in [(0, 1), (u32::MAX, 1024)] {
            let settings = RemoteFileEditorUserSettings {
                max_file_size_mib: value,
                ..Default::default()
            };
            assert_eq!(expected * 1024 * 1024, settings.max_file_size_bytes());
        }
    }
}
