rust_i18n::i18n!("locales", fallback = "en");

mod document_index;
mod file_manager;
mod markdown_conflict;
mod markdown_file_store;
mod markdown_mode;
mod markdown_render;
mod markdown_renderer;
mod markdown_session;
mod markdown_source;
mod markdown_view;
mod markdown_watcher;
mod model;
mod notes_actions;
mod notes_close;
mod notes_export;
mod notes_notifications;
mod notes_render;
mod notes_setup;
mod notes_setup_render;
mod notes_view;
mod path_policy;
mod shortcuts;
mod storage;
mod storage_location;
mod storage_support;
mod theme_provider;
mod tree_state;

#[cfg(test)]
mod storage_tests;

pub use model::{
    DeleteSummary, DocumentDescriptor, DocumentFormat, FileNode, MarkdownSaveMode,
    MarkdownViewMode, NodeKind, NotebookMetadata, NotebookUiState,
};
pub use notes_view::{NotesView, NotesViewEvent};
pub use path_policy::validate_node_name;
pub use shortcuts::NotesShortcutDescriptor;
pub use storage::NotesStorage;
pub use theme_provider::MarkdownEditorTheme;
pub use tree_state::{TreeRow, TreeState};

pub fn init(cx: &mut gpui::App) {
    markdown_editor::init(cx);
    markdown_source::init(cx);
    shortcuts::init(cx);
}

pub fn refresh_keybindings(cx: &mut gpui::App) {
    shortcuts::refresh(cx);
}

pub fn shortcut_descriptors() -> Vec<NotesShortcutDescriptor> {
    shortcuts::descriptors()
}
