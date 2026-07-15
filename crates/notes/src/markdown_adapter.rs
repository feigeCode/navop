use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_persistence::MarkdownDocumentPersistence;
use anyhow::Result;
use cditor_app::{
    Editor, EditorDocument, EditorHandle, MarkdownApplyMode, MarkdownCompatibility,
    MarkdownDiagnostic, MarkdownExportMode, MarkdownImportResult,
};
use gpui::AppContext;
use std::time::Duration;

const MARKDOWN_AUTOSAVE_INTERVAL: Duration = Duration::from_millis(700);

pub(crate) struct MarkdownProjection {
    pub handle: EditorHandle,
    pub compatibility: MarkdownCompatibility,
    pub diagnostics: Vec<MarkdownDiagnostic>,
}

pub(crate) fn build_markdown_projection<C: AppContext>(
    document_id: &str,
    source: &str,
    store: MarkdownFileStore,
    cx: &mut C,
) -> Result<MarkdownProjection> {
    let imported = EditorDocument::from_markdown_with_report(document_id, source)?;
    let readonly = !matches!(imported.compatibility, MarkdownCompatibility::Editable);
    let handle = Editor::builder()
        .document_id(document_id)
        .initial_document(imported.document)
        .persistence(MarkdownDocumentPersistence::new(store))
        .autosave(MARKDOWN_AUTOSAVE_INTERVAL)
        .readonly(readonly)
        .build(cx)?;
    Ok(MarkdownProjection {
        handle,
        compatibility: imported.compatibility,
        diagnostics: imported.diagnostics,
    })
}

pub(crate) fn apply_markdown_source<C: AppContext>(
    handle: &EditorHandle,
    source: &str,
    normalization_accepted: bool,
    cx: &mut C,
) -> Result<MarkdownImportResult> {
    let imported = handle.apply_markdown(source, MarkdownApplyMode::ReadOnlyPreview, cx)?;
    let readonly = match imported.compatibility {
        MarkdownCompatibility::Editable => false,
        MarkdownCompatibility::EditableWithNormalization(_) => !normalization_accepted,
        MarkdownCompatibility::SourceOnly(_) => true,
    };
    handle.set_readonly(readonly, cx)?;
    Ok(imported)
}

pub(crate) fn export_markdown_strict<C: AppContext>(
    handle: &EditorHandle,
    cx: &C,
) -> Result<String> {
    Ok(handle
        .export_markdown(MarkdownExportMode::Strict, cx)?
        .markdown)
}
