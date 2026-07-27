use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookMetadata {
    pub id: Uuid,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct NotebookUiState {
    pub selected_document: Option<PathBuf>,
    pub expanded_directories: BTreeSet<PathBuf>,
    #[serde(default)]
    pub markdown_view_modes: BTreeMap<String, MarkdownViewMode>,
    #[serde(default)]
    pub markdown_save_mode: MarkdownSaveMode,
    #[serde(default)]
    pub last_created_format: DocumentFormat,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum NodeKind {
    Directory,
    Document,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentFormat {
    #[default]
    #[serde(alias = "rich_text")]
    Markdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownViewMode {
    #[default]
    Wysiwyg,
    Source,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownSaveMode {
    #[default]
    Automatic,
    Manual,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileNode {
    pub relative_path: PathBuf,
    pub display_name: String,
    pub kind: NodeKind,
    pub format: Option<DocumentFormat>,
    pub children: Vec<FileNode>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DocumentDescriptor {
    pub document_id: String,
    pub format: DocumentFormat,
    pub relative_path: PathBuf,
    pub absolute_path: PathBuf,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DeleteSummary {
    pub directories: usize,
    pub documents: usize,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_ui_state_uses_safe_markdown_defaults() {
        let state: NotebookUiState =
            serde_json::from_str(r#"{"selected_document":null,"expanded_directories":[]}"#)
                .unwrap();
        assert!(state.markdown_view_modes.is_empty());
        assert_eq!(DocumentFormat::Markdown, state.last_created_format);
        assert_eq!(MarkdownViewMode::Wysiwyg, MarkdownViewMode::default());
        assert_eq!(MarkdownSaveMode::Automatic, state.markdown_save_mode);
    }

    #[test]
    fn markdown_save_mode_round_trips_manual_choice() {
        let mut state = NotebookUiState::default();
        state.markdown_save_mode = MarkdownSaveMode::Manual;

        let json = serde_json::to_string(&state).unwrap();
        let restored: NotebookUiState = serde_json::from_str(&json).unwrap();

        assert_eq!(MarkdownSaveMode::Manual, restored.markdown_save_mode);
        assert!(json.contains(r#""markdown_save_mode":"manual""#));
    }

    #[test]
    fn legacy_rich_text_format_migrates_to_markdown() {
        let state: NotebookUiState = serde_json::from_str(
            r#"{"selected_document":null,"expanded_directories":[],"last_created_format":"rich_text"}"#,
        )
        .unwrap();

        assert_eq!(DocumentFormat::Markdown, state.last_created_format);
        assert_eq!(
            r#""markdown""#,
            serde_json::to_string(&state.last_created_format).unwrap()
        );
    }
}
