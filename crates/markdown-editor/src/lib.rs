mod actions;
mod editor;
mod projection;
mod theme;

pub use actions::*;
pub use editor::{MarkdownEditor, MarkdownEditorError, MarkdownEditorEvent};
pub use projection::{
    MarkdownProjection, ProjectionEdit, ProjectionSegment, ProjectionStyle, ProjectionStyleSpan,
};
pub use theme::MarkdownEditorTheme;

pub fn init(cx: &mut gpui::App) {
    actions::init(cx);
}
