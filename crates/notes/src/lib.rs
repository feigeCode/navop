rust_i18n::i18n!("locales", fallback = "en");

mod document_persistence;
mod model;
mod notes_actions;
mod notes_render;
mod notes_view;
mod path_policy;
mod storage;
mod tree_state;

#[cfg(test)]
mod storage_tests;

pub use document_persistence::FileDocumentPersistence;
pub use model::{
    DeleteSummary, DocumentDescriptor, FileNode, NodeKind, NotebookMetadata, NotebookUiState,
};
pub use notes_view::NotesView;
pub use path_policy::validate_node_name;
pub use storage::NotesStorage;
pub use tree_state::{TreeRow, TreeState};

#[cfg(test)]
mod tests {
    use cditor_app::{
        Editor, EditorDocument, EditorHandle, EditorPersistence, EditorPersistenceError,
        EditorSaveRequest,
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
        let _ = std::mem::size_of::<EditorHandle>();
        assert_persistence::<CompileOnlyPersistence>();
    }
}
