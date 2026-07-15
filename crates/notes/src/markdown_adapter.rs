use anyhow::Result;
use cditor_app::{Editor, EditorHandle};
use gpui::AppContext;

pub(crate) fn build_markdown_preview<C: AppContext>(
    document_id: &str,
    source: &str,
    cx: &mut C,
) -> Result<EditorHandle> {
    Ok(Editor::builder()
        .document_id(document_id)
        .initial_markdown(source)
        .readonly(true)
        .build(cx)?)
}

pub(crate) fn refresh_markdown_preview<C: AppContext>(
    handle: &EditorHandle,
    source: &str,
    cx: &mut C,
) -> Result<()> {
    handle.set_markdown(source, cx)?;
    handle.set_readonly(true, cx)?;
    Ok(())
}
