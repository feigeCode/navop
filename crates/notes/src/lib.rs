rust_i18n::i18n!("locales", fallback = "en");

mod document_index;
mod document_persistence;
mod markdown_adapter;
mod markdown_file_store;
mod markdown_persistence;
mod markdown_render;
mod markdown_session;
mod markdown_source;
mod markdown_view;
mod model;
mod notes_actions;
mod notes_close;
mod notes_render;
mod notes_view;
mod path_policy;
mod storage;
mod storage_support;
mod tree_state;

#[cfg(test)]
mod storage_tests;

pub use document_persistence::FileDocumentPersistence;
pub use model::{
    DeleteSummary, DocumentDescriptor, DocumentFormat, FileNode, MarkdownViewMode, NodeKind,
    NotebookMetadata, NotebookUiState,
};
pub use notes_view::NotesView;
pub use path_policy::validate_node_name;
pub use storage::NotesStorage;
pub use tree_state::{TreeRow, TreeState};

/// Installs the Cditor keymap required by embedded Notes editors.
pub fn init(cx: &mut gpui::App) {
    cditor_app::init(cx);
}

#[cfg(test)]
mod tests {
    use cditor_app::{
        Editor, EditorDocument, EditorHandle, EditorPersistence, EditorPersistenceError,
        EditorSaveRequest, MarkdownApplyMode, MarkdownCompatibility, MarkdownExportMode,
    };

    struct CompileOnlyPersistence;

    impl EditorPersistence for CompileOnlyPersistence {
        fn load(
            &self,
            _document_id: &str,
        ) -> Result<Option<EditorDocument>, EditorPersistenceError> {
            Ok(None)
        }

        fn save(&self, _request: EditorSaveRequest) -> Result<(), EditorPersistenceError> {
            Ok(())
        }
    }

    #[test]
    fn cditor_public_integration_api_is_available() {
        fn assert_persistence<T: EditorPersistence>() {}

        let _ = Editor::builder;
        let _ = EditorDocument::from_json;
        let imported = EditorDocument::from_markdown_with_report("doc-1", "Body").unwrap();
        let _ = imported
            .document
            .export_markdown(MarkdownExportMode::Strict)
            .unwrap();
        let _ = cditor_app::init;
        let _ = std::mem::size_of::<EditorHandle>();
        let _ = MarkdownApplyMode::ReadOnlyPreview;
        let _ = MarkdownCompatibility::Editable;
        let _ = MarkdownExportMode::Strict;
        assert_persistence::<CompileOnlyPersistence>();
    }
}
