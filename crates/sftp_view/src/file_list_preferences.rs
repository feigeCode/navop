use anyhow::{Context as _, Result};
use one_core::storage::get_config_dir;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

const FILE_LIST_PREFERENCES_FILE: &str = "sftp-file-list.json";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FileListPreferenceScope {
    Left,
    Right,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct PanePreferences {
    hidden_columns: BTreeSet<String>,
}

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default)]
struct FileListPreferences {
    left: PanePreferences,
    right: PanePreferences,
}

impl FileListPreferences {
    fn pane(&self, scope: FileListPreferenceScope) -> &PanePreferences {
        match scope {
            FileListPreferenceScope::Left => &self.left,
            FileListPreferenceScope::Right => &self.right,
        }
    }

    fn pane_mut(&mut self, scope: FileListPreferenceScope) -> &mut PanePreferences {
        match scope {
            FileListPreferenceScope::Left => &mut self.left,
            FileListPreferenceScope::Right => &mut self.right,
        }
    }
}

pub(crate) fn load_hidden_columns(scope: FileListPreferenceScope) -> BTreeSet<String> {
    load_preferences()
        .map(|preferences| preferences.pane(scope).hidden_columns.clone())
        .unwrap_or_else(|error| {
            tracing::warn!("Failed to load SFTP file-list preferences: {error:#}");
            BTreeSet::new()
        })
}

pub(crate) fn save_hidden_columns(
    scope: FileListPreferenceScope,
    hidden_columns: &BTreeSet<String>,
) -> Result<()> {
    let mut preferences = load_preferences().unwrap_or_default();
    preferences.pane_mut(scope).hidden_columns = hidden_columns.clone();

    let path = preferences_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create SFTP file-list preference directory {}",
                parent.display()
            )
        })?;
    }
    let json = serde_json::to_string_pretty(&preferences)
        .context("failed to serialize SFTP file-list preferences")?;
    std::fs::write(&path, json).with_context(|| {
        format!(
            "failed to write SFTP file-list preferences {}",
            path.display()
        )
    })
}

fn load_preferences() -> Result<FileListPreferences> {
    let path = preferences_path()?;
    if !path.exists() {
        return Ok(FileListPreferences::default());
    }

    let json = std::fs::read_to_string(&path).with_context(|| {
        format!(
            "failed to read SFTP file-list preferences {}",
            path.display()
        )
    })?;
    serde_json::from_str(&json).with_context(|| {
        format!(
            "failed to parse SFTP file-list preferences {}",
            path.display()
        )
    })
}

fn preferences_path() -> Result<std::path::PathBuf> {
    Ok(get_config_dir()?.join(FILE_LIST_PREFERENCES_FILE))
}

#[cfg(test)]
mod tests {
    use super::{FileListPreferenceScope, FileListPreferences};

    #[test]
    fn missing_panes_and_unknown_fields_use_safe_defaults() {
        let preferences: FileListPreferences = serde_json::from_str(
            r#"{
                "left": {
                    "hidden_columns": ["kind"],
                    "future_option": true
                }
            }"#,
        )
        .expect("preferences should deserialize");

        assert!(
            preferences
                .pane(FileListPreferenceScope::Left)
                .hidden_columns
                .contains("kind")
        );
        assert!(
            preferences
                .pane(FileListPreferenceScope::Right)
                .hidden_columns
                .is_empty()
        );
    }
}
