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
    /// 纯文本笔记（issue #429）。没有 Markdown 渲染，只在源码视图里编辑。
    Text,
}

impl DocumentFormat {
    /// 该格式落盘时使用的文件后缀。
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Markdown => ".md",
            Self::Text => ".txt",
        }
    }

    /// 是否为纯文本：不提供所见即所得视图。
    pub fn is_plain_text(self) -> bool {
        matches!(self, Self::Text)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MarkdownViewMode {
    #[default]
    Wysiwyg,
    Source,
}

impl<'de> Deserialize<'de> for MarkdownViewMode {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = <String as Deserialize>::deserialize(deserializer)?;
        match value.as_str() {
            "wysiwyg" => Ok(Self::Wysiwyg),
            "source" | "split" => Ok(Self::Source),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["wysiwyg", "source"],
            )),
        }
    }
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

    #[test]
    fn text_format_round_trips_and_keeps_markdown_as_default() {
        let json = serde_json::to_string(&DocumentFormat::Text).unwrap();
        assert_eq!(r#""text""#, json);
        assert_eq!(
            DocumentFormat::Text,
            serde_json::from_str::<DocumentFormat>(&json).unwrap()
        );
        assert_eq!(DocumentFormat::Markdown, DocumentFormat::default());
        assert!(DocumentFormat::Text.is_plain_text());
        assert!(!DocumentFormat::Markdown.is_plain_text());
    }

    #[test]
    fn last_created_text_format_persists() {
        let mut state = NotebookUiState::default();
        state.last_created_format = DocumentFormat::Text;
        let restored: NotebookUiState =
            serde_json::from_str(&serde_json::to_string(&state).unwrap()).unwrap();
        assert_eq!(DocumentFormat::Text, restored.last_created_format);
    }

    #[test]
    fn legacy_split_mode_migrates_to_source() {
        let state: NotebookUiState = serde_json::from_str(
            r#"{"selected_document":null,"expanded_directories":[],"markdown_view_modes":{"doc-1":"split"}}"#,
        )
        .unwrap();

        assert_eq!(
            MarkdownViewMode::Source,
            state.markdown_view_modes.get("doc-1").copied().unwrap()
        );
        assert_eq!(
            r#""source""#,
            serde_json::to_string(&MarkdownViewMode::Source).unwrap()
        );
    }
}
