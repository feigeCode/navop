rust_i18n::i18n!("locales", fallback = "en");

mod ai_model_catalog;
mod ai_provider;
mod document_index;
mod document_persistence;
mod document_rendering;
mod file_manager;
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
mod notes_conversion;
mod notes_export;
mod notes_notifications;
mod notes_render;
mod notes_setup;
mod notes_setup_render;
mod notes_view;
mod path_policy;
mod shortcuts;
mod storage;
mod storage_conversion;
mod storage_location;
mod storage_support;
mod syntax_highlighting;
mod theme_provider;
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
pub use shortcuts::NotesShortcutDescriptor;
pub use storage::NotesStorage;
pub use tree_state::{TreeRow, TreeState};

/// Installs the Cditor keymap required by embedded Notes editors.
pub fn init(cx: &mut gpui::App) {
    shortcuts::init(cx);
}

/// Rebinds host-configured Cditor commands after shortcut settings change.
pub fn refresh_keybindings(cx: &mut gpui::App) {
    shortcuts::refresh(cx);
}

/// Returns the stable Cditor command catalog with Navop defaults.
pub fn shortcut_descriptors() -> Vec<NotesShortcutDescriptor> {
    shortcuts::descriptors()
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
        let _ = cditor_app::init_for_external_keymap;
        let _ = std::mem::size_of::<EditorHandle>();
        let _ = MarkdownApplyMode::ReadOnlyPreview;
        let _ = MarkdownCompatibility::Editable;
        let _ = MarkdownExportMode::Strict;
        let _ = |cx: &mut gpui::App, bindings: Vec<cditor_app::CditorKeyBinding>| {
            cditor_app::bind_command_keys(cx, bindings)
        };
        assert_persistence::<CompileOnlyPersistence>();
    }
}
