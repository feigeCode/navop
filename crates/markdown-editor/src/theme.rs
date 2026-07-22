use gpui::Hsla;
use gpui_component::highlighter::HighlightTheme;
use std::sync::Arc;

#[derive(Clone, PartialEq)]
pub struct MarkdownEditorTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub primary: Hsla,
    pub highlight_theme: Arc<HighlightTheme>,
}
