use gpui::{App, KeyBinding, actions};

pub(crate) const MARKDOWN_CONTEXT: &str = "NotesMarkdown";

actions!(notes_markdown, [SaveMarkdown]);

pub(crate) fn init(cx: &mut App) {
    cx.bind_keys([KeyBinding::new(
        "secondary-s",
        SaveMarkdown,
        Some(MARKDOWN_CONTEXT),
    )]);
}
