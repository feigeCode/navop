use crate::markdown_file_store::MarkdownFileStore;
use crate::markdown_persistence::MarkdownDocumentPersistence;
use anyhow::Result;
use cditor_app::{
    AiProvider, DocumentRendererProvider, Editor, EditorDocument, EditorEvent, EditorHandle,
    MarkdownApplyMode, MarkdownBundleOptions, MarkdownCompatibility, MarkdownDiagnostic,
    MarkdownExportMode, MarkdownImportResult, SyntaxHighlightProvider, ThemeProvider,
};
use gpui::AppContext;
use smol::channel::{Receiver, unbounded};
use std::sync::Arc;
use std::time::Duration;

const MARKDOWN_AUTOSAVE_INTERVAL: Duration = Duration::from_millis(700);

pub(crate) struct MarkdownProjection {
    pub handle: EditorHandle,
    pub compatibility: MarkdownCompatibility,
    pub diagnostics: Vec<MarkdownDiagnostic>,
    pub events: Receiver<EditorEvent>,
}

pub(crate) struct MarkdownProjectionConfig<'a> {
    pub document_id: &'a str,
    pub source: &'a str,
    pub store: MarkdownFileStore,
    pub ai_provider: Option<Arc<dyn AiProvider>>,
    pub ai_model_id: Option<&'a str>,
    pub syntax_highlight_provider: Arc<dyn SyntaxHighlightProvider>,
    pub document_renderer_provider: Option<Arc<dyn DocumentRendererProvider>>,
    pub theme_provider: Arc<dyn ThemeProvider>,
}

pub(crate) fn build_markdown_projection<C: AppContext>(
    config: MarkdownProjectionConfig<'_>,
    cx: &mut C,
) -> Result<MarkdownProjection> {
    let imported = EditorDocument::from_markdown_bundle_with_report(
        config.document_id,
        config.source,
        &config.store,
    )?;
    let readonly = !matches!(imported.compatibility, MarkdownCompatibility::Editable);
    let media_base_path = config.store.media_base_path()?;
    let (event_sender, events) = unbounded();
    let mut builder = Editor::builder()
        .document_id(config.document_id)
        .initial_document(imported.document)
        .persistence(MarkdownDocumentPersistence::new(config.store))
        .media_base_path(media_base_path)
        .markdown_native_blocks_only(true)
        .autosave(MARKDOWN_AUTOSAVE_INTERVAL)
        .readonly(readonly)
        .on_event(move |event| {
            let _ = event_sender.try_send(event);
        });
    builder = match config.ai_provider {
        Some(provider) => builder.ai_provider_arc(provider),
        None => builder.without_ai(),
    };
    builder = builder.syntax_highlight_provider_arc(config.syntax_highlight_provider);
    builder = builder.theme_provider_arc(config.theme_provider);
    if let Some(provider) = config.document_renderer_provider {
        builder = builder.document_renderer_provider_arc(provider);
    }
    let handle = builder.build(cx)?;
    if let Some(model_id) = config.ai_model_id {
        let _ = handle.select_ai_model(model_id, cx);
    }
    Ok(MarkdownProjection {
        handle,
        compatibility: imported.compatibility,
        diagnostics: imported.diagnostics,
        events,
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

pub(crate) fn export_markdown_bundle<C: AppContext>(
    handle: &EditorHandle,
    store: &MarkdownFileStore,
    cx: &C,
) -> Result<String> {
    let document = handle.get_document(cx)?;
    export_markdown_bundle_from_document(&document, store)
}

pub(crate) fn export_markdown_bundle_from_document(
    document: &EditorDocument,
    store: &MarkdownFileStore,
) -> Result<String> {
    let exported = document.export_markdown_bundle(
        MarkdownExportMode::BestEffort,
        &MarkdownBundleOptions {
            asset_directory: store.asset_directory()?,
            ..MarkdownBundleOptions::default()
        },
    )?;
    store.write_assets(&exported.assets)?;
    Ok(exported.markdown)
}
