rust_i18n::i18n!("locales", fallback = "en");

mod document_index;
mod file_manager;
mod markdown_conflict;
mod markdown_file_store;
mod markdown_mode;
mod markdown_outline;
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
    MarkdownViewMode, NodeKind, NotebookMetadata, NotebookUiState, NoteDocument,
};
pub use notes_view::{NotesView, NotesViewEvent};
pub use path_policy::validate_node_name;
pub use shortcuts::NotesShortcutDescriptor;
pub use storage::NotesStorage;
pub use theme_provider::MarkdownEditorTheme;
pub use tree_state::{TreeRow, TreeState};

pub fn init(cx: &mut gpui::App) {
    markdown_editor::init(cx);
    sync_markdown_editor_fonts(cx);
    // 设置面板改动字体后立刻反映到编辑器，无需重启。
    cx.observe_global::<one_core::settings::AppSettings>(|cx| sync_markdown_editor_fonts(cx))
        .detach();
    markdown_source::init(cx);
    shortcuts::init(cx);
}

/// 把 Navop 的字体设置同步到内嵌 Markdown 编辑器。
///
/// 正文跟通用字体走（与主题字体一致），代码字体单独取「Markdown 代码字体」；
/// 字体族未安装时由编辑器回退到系统字体，不会影响渲染。
pub fn sync_markdown_editor_fonts(cx: &mut gpui::App) {
    let settings = one_core::settings::AppSettings::current(cx);
    let preferences = markdown_editor::FontPreferences::new(
        settings.font_family.clone(),
        settings.markdown_code_font_family.clone(),
        settings.font_family.clone(),
    );
    let _ = markdown_editor::set_font_stacks(preferences, cx);
}

pub fn refresh_keybindings(cx: &mut gpui::App) {
    shortcuts::refresh(cx);
}

pub fn set_markdown_editor_locale(locale: &str, cx: &mut gpui::App) -> bool {
    markdown_editor::set_locale(locale, cx)
}

pub fn shortcut_descriptors() -> Vec<NotesShortcutDescriptor> {
    shortcuts::descriptors()
}
