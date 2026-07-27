use gpui::{App, Hsla};
use gpui_component::{ActiveTheme, highlighter::HighlightTheme};
use std::sync::Arc;

#[derive(Clone)]
pub struct MarkdownEditorTheme {
    pub background: Hsla,
    pub foreground: Hsla,
    pub muted: Hsla,
    pub muted_foreground: Hsla,
    pub border: Hsla,
    pub primary: Hsla,
    pub primary_foreground: Hsla,
    pub danger: Hsla,
    pub warning: Hsla,
    pub highlight_theme: Arc<HighlightTheme>,
}

impl MarkdownEditorTheme {
    pub(crate) fn from_app(cx: &App) -> Self {
        Self {
            background: cx.theme().background,
            foreground: cx.theme().foreground,
            muted: cx.theme().muted,
            muted_foreground: cx.theme().muted_foreground,
            border: cx.theme().border,
            primary: cx.theme().primary,
            primary_foreground: cx.theme().primary_foreground,
            danger: cx.theme().danger,
            warning: cx.theme().warning,
            highlight_theme: cx.theme().highlight_theme.clone(),
        }
    }
}
