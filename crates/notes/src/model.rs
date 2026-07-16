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
    RichText,
    Markdown,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownViewMode {
    #[default]
    Wysiwyg,
    Source,
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
        assert_eq!(DocumentFormat::RichText, state.last_created_format);
        assert_eq!(MarkdownViewMode::Wysiwyg, MarkdownViewMode::default());
    }
}
