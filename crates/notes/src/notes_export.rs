use crate::notes_notifications::notify_operation_error;
use crate::{DocumentFormat, MarkdownViewMode, NotesView, TreeRow};
use cditor_app::{
    DocumentRenderRequest, DocumentRenderTheme, DocumentRendererProvider, EditorDocument,
    MarkdownExportMode,
};
use gpui::{App, AppContext, AsyncApp, Context, Hsla, PathPromptOptions, Rgba, Window};
use gpui_component::{ActiveTheme, WindowExt, notification::Notification};
use one_core::tab_container::{TabContentEvent, TabItem, TabOpenMode};
use rust_i18n::t;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NotesExportFormat {
    Html,
    Pdf,
    Word,
}

impl NotesExportFormat {
    pub(crate) const ALL: [Self; 3] = [Self::Html, Self::Pdf, Self::Word];

    fn protocol_name(self) -> &'static str {
        match self {
            Self::Html => "html",
            Self::Pdf => "pdf",
            Self::Word => "docx",
        }
    }

    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Html => "HTML",
            Self::Pdf => "PDF",
            Self::Word => "Word (.docx)",
        }
    }

    fn marketplace_query(self) -> &'static str {
        match self {
            Self::Html => "Notes HTML Exporter",
            Self::Pdf => "Notes PDF Exporter",
            Self::Word => "Notes Word Exporter",
        }
    }

    fn marketplace_tab_id(self) -> &'static str {
        match self {
            Self::Html => "extensions-notes-html-exporter",
            Self::Pdf => "extensions-notes-pdf-exporter",
            Self::Word => "extensions-notes-word-exporter",
        }
    }
}

impl NotesView {
    pub(crate) fn export_document(
        &mut self,
        row: TreeRow,
        format: NotesExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if row.kind != crate::NodeKind::Document {
            return;
        }
        let format_name = format.protocol_name().to_owned();
        let Some(catalog) = cx
            .try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()
            .and_then(|global| global.get())
        else {
            self.open_document_exporter_marketplace(format, window, cx);
            return;
        };
        if catalog.document_exporter_for_format(&format_name).is_none() {
            self.open_document_exporter_marketplace(format, window, cx);
            return;
        }
        let export_source = match self.source_for_export(&row, cx) {
            Ok(source) => source,
            Err(error) => {
                notify_operation_error(window, cx, error);
                return;
            }
        };
        let title = row.display_name.clone();
        let theme = export_theme(
            cx.theme().background,
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().border,
            cx.theme().primary,
            cx.theme().danger,
        );
        window.push_notification(
            Notification::info(
                "将按预览效果导出：数学公式、Mermaid 等渲染为图片，HTML 按页面效果输出，普通代码块保留源码。"
                    .to_owned(),
            )
            .autohide(true),
            cx,
        );
        let document_renderer = self.document_renderer_provider.clone();
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("Notes.select_export_directory").into()),
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let selected = match prompt.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        window.push_notification(
                            Notification::error(
                                t!("Notes.operation_failed", error = error.to_string()).to_string(),
                            )
                            .autohide(false),
                            cx,
                        );
                    });
                    return;
                }
                Err(error) => {
                    let _ = cx.update_window(window_handle, |_, window, cx| {
                        window.push_notification(
                            Notification::error(
                                t!("Notes.operation_failed", error = error.to_string()).to_string(),
                            )
                            .autohide(false),
                            cx,
                        );
                    });
                    return;
                }
            };
            let Some(directory) = selected else { return };
            let _ = cx.update_window(window_handle, |_, window, cx| {
                window.push_notification(
                    Notification::info(t!("Notes.export_started").to_string()).autohide(true),
                    cx,
                );
            });
            let result = cx
                .background_spawn(async move {
                    let mut assets = export_source.assets;
                    for pending in export_source.pending_renders {
                        let provider = document_renderer.as_ref().ok_or_else(|| {
                            anyhow::anyhow!(
                                "块 {} 当前为预览态，但文档渲染扩展不可用；请切换为源码态后重试",
                                pending.block_id
                            )
                        })?;
                        let rendered = provider
                            .render(DocumentRenderRequest {
                                renderer: pending.renderer.to_owned(),
                                source: pending.source,
                                available_width: 720.0,
                                scale_factor: 1.0,
                                theme: DocumentRenderTheme {
                                    dark: theme.dark,
                                    background: theme.background,
                                    foreground: theme.foreground,
                                    border: theme.border,
                                    muted: theme.muted,
                                    accent: theme.accent,
                                    danger: theme.danger,
                                    font_family:
                                        "Inter, ui-sans-serif, system-ui, -apple-system, sans-serif"
                                            .to_owned(),
                                },
                            })
                            .await
                            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
                        assets.push(extension_wasm::DocumentExportAsset {
                            path: pending.path,
                            media_type: rendered.media_type,
                            bytes: rendered.bytes,
                        });
                    }
                    let artifact = catalog
                        .export_document(extension_wasm::DocumentExportRequest {
                            exporter: String::new(),
                            format: format_name,
                            title: title.clone(),
                            source: export_source.source,
                            assets,
                            theme,
                        })
                        .await
                        .map_err(|error| anyhow::anyhow!(error.to_string()))?
                        .ok_or_else(|| {
                            anyhow::anyhow!("no document exporter supports this format")
                        })?;
                    let path = next_export_path(&directory, &title, &artifact.extension)?;
                    fs::write(&path, &artifact.bytes).map_err(|error| {
                        anyhow::anyhow!("write export {}: {error}", path.display())
                    })?;
                    Ok::<_, anyhow::Error>(path)
                })
                .await;
            let _ = cx.update_window(window_handle, |_, window, cx| match result {
                Ok(path) => window.push_notification(
                    Notification::success(
                        t!("Notes.exported", path = path.display().to_string()).to_string(),
                    ),
                    cx,
                ),
                Err(error) => window.push_notification(
                    Notification::error(
                        t!("Notes.operation_failed", error = error.to_string()).to_string(),
                    )
                    .autohide(false),
                    cx,
                ),
            });
        })
        .detach();
    }

    fn open_document_exporter_marketplace(
        &mut self,
        format: NotesExportFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let host = Arc::new(extension_runtime::MainExtensionViewHost);
        let extensions = cx.new(|cx| {
            extension_view::ExtensionManagerView::new_marketplace_search(
                host,
                format.marketplace_query(),
                window,
                cx,
            )
        });
        cx.emit(TabContentEvent::OpenTab {
            tab: TabItem::new(format.marketplace_tab_id(), "home", extensions),
            mode: TabOpenMode::Activate,
        });
        window.push_notification(
            Notification::info(t!("Notes.export_extension_required").to_string()).autohide(true),
            cx,
        );
    }

    fn source_for_export(&self, row: &TreeRow, cx: &App) -> anyhow::Result<ExportSource> {
        let descriptor = self.storage()?.descriptor(&row.relative_path)?;
        let source_path = descriptor.absolute_path.clone();
        let source = match descriptor.format {
            DocumentFormat::Markdown => {
                if let Some((_, session)) = self
                    .markdown_sessions
                    .iter()
                    .find(|(_, session)| session.relative_path == row.relative_path)
                {
                    match session.state.mode {
                        MarkdownViewMode::Source => {
                            let source = session.source_editor.read(cx).value().to_string();
                            let document = EditorDocument::from_markdown("notes-export", &source)?;
                            let (document, pending_renders) = document_for_preview(&document);
                            Ok(PreparedExportSource {
                                source:
                                    crate::markdown_adapter::export_markdown_bundle_from_document(
                                        &document,
                                        &session.store,
                                    )?,
                                pending_renders,
                            })
                        }
                        MarkdownViewMode::Wysiwyg => {
                            let document = session.preview.get_document(cx)?;
                            let document = without_comment_blocks(&document);
                            let (document, pending_renders) = document_for_preview(&document);
                            Ok(PreparedExportSource {
                                source:
                                    crate::markdown_adapter::export_markdown_bundle_from_document(
                                        &document,
                                        &session.store,
                                    )?,
                                pending_renders,
                            })
                        }
                    }
                } else {
                    Ok(PreparedExportSource {
                        source: fs::read_to_string(&source_path)?,
                        pending_renders: Vec::new(),
                    })
                }
            }
            DocumentFormat::RichText => {
                if let Some(cached) = self
                    .editors
                    .values()
                    .find(|cached| cached.relative_path == row.relative_path)
                {
                    let document = cached.handle.get_document(cx)?;
                    export_document_preview(&without_comment_blocks(&document))
                } else {
                    let document = EditorDocument::from_json(&fs::read_to_string(&source_path)?)?;
                    export_document_preview(&without_comment_blocks(&document))
                }
            }
        }?;
        let source_text = strip_whiteboard_metadata_comments(&source.source);
        let assets = collect_export_assets(&source_text, source_path.parent());
        Ok(ExportSource {
            source: source_text,
            assets,
            pending_renders: source.pending_renders,
        })
    }
}

struct ExportSource {
    source: String,
    assets: Vec<extension_wasm::DocumentExportAsset>,
    pending_renders: Vec<PendingDocumentRender>,
}

struct PreparedExportSource {
    source: String,
    pending_renders: Vec<PendingDocumentRender>,
}

struct PendingDocumentRender {
    block_id: u64,
    renderer: &'static str,
    source: String,
    path: String,
}

fn collect_export_assets(
    source: &str,
    base: Option<&Path>,
) -> Vec<extension_wasm::DocumentExportAsset> {
    let Some(base) = base else { return Vec::new() };
    let mut assets = Vec::new();
    let mut seen = HashSet::new();
    for target in export_asset_paths(source) {
        let path = if let Some(target) = target.strip_prefix('<') {
            target
                .split_once('>')
                .map(|(path, _)| path)
                .unwrap_or(target)
        } else {
            target.split_whitespace().next().unwrap_or(target)
        };
        if path.is_empty()
            || path.contains("://")
            || path.starts_with("data:")
            || path.starts_with("asset:")
        {
            continue;
        }
        if !seen.insert(path.to_owned()) {
            continue;
        }
        let file = base.join(path);
        let Ok(bytes) = fs::read(&file) else { continue };
        let media_type = match file
            .extension()
            .and_then(|extension| extension.to_str())
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str()
        {
            "png" => "image/png",
            "jpg" | "jpeg" => "image/jpeg",
            "gif" => "image/gif",
            "svg" => "image/svg+xml",
            _ => "application/octet-stream",
        };
        assets.push(extension_wasm::DocumentExportAsset {
            path: path.to_owned(),
            media_type: media_type.to_owned(),
            bytes,
        });
    }
    assets
}

fn export_asset_paths(source: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut remaining = source;
    while let Some(start) = remaining.find("![") {
        remaining = &remaining[start + 2..];
        let Some(label_end) = remaining.find("](") else {
            break;
        };
        let target_start = label_end + 2;
        let Some(target_end) = remaining[target_start..].find(')') else {
            break;
        };
        paths.push(remaining[target_start..target_start + target_end].trim());
        remaining = &remaining[target_start + target_end + 1..];
    }
    let lower = source.to_ascii_lowercase();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("<img") {
        let start = cursor + relative;
        let Some(end) = source[start..].find('>').map(|offset| start + offset) else {
            break;
        };
        let tag = &source[start..=end];
        if let Some(path) = html_export_attribute(tag, "src") {
            paths.push(path);
        }
        cursor = end + 1;
    }
    paths
}

fn html_export_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let marker = format!("{name}=");
    let start = lower.find(&marker)? + marker.len();
    let quote = tag.as_bytes().get(start).copied()?;
    if quote == b'\'' || quote == b'"' {
        let value_start = start + 1;
        let end = tag[value_start..].find(quote as char)? + value_start;
        return Some(&tag[value_start..end]);
    }
    let end = tag[start..]
        .find(|character: char| character.is_whitespace() || character == '>')
        .map(|offset| start + offset)
        .unwrap_or(tag.len());
    Some(&tag[start..end])
}

fn export_rich_text_document(document: &EditorDocument) -> anyhow::Result<String> {
    Ok(without_comment_blocks(document)
        .export_markdown(MarkdownExportMode::BestEffort)?
        .markdown)
}

fn export_document_preview(document: &EditorDocument) -> anyhow::Result<PreparedExportSource> {
    let (document, pending_renders) = document_for_preview(document);
    Ok(PreparedExportSource {
        source: export_rich_text_document(&document)?,
        pending_renders,
    })
}

fn document_for_preview(document: &EditorDocument) -> (EditorDocument, Vec<PendingDocumentRender>) {
    use cditor_app::core::rich_text::{
        BlockPayload, ImagePayload, RichBlockKind, kind_tag_for_rich_block_kind,
    };

    let mut document = document.clone();
    let mut pending = Vec::new();
    for block in &mut document.blocks {
        let renderer = match &block.payload.kind {
            RichBlockKind::Math => Some("math"),
            RichBlockKind::Mermaid => Some("mermaid"),
            RichBlockKind::Code { language }
                if language.as_deref().is_some_and(|language| {
                    matches!(
                        language.trim().to_ascii_lowercase().as_str(),
                        "math" | "latex" | "tex" | "katex"
                    )
                }) =>
            {
                Some("math")
            }
            _ => None,
        };
        let Some(renderer) = renderer else { continue };
        let source = block.payload.plain_text();
        let path = format!("navop-export/rendered-block-{}.svg", block.id);
        pending.push(PendingDocumentRender {
            block_id: block.id,
            renderer,
            source,
            path: path.clone(),
        });
        block.kind_tag = kind_tag_for_rich_block_kind(&RichBlockKind::Image);
        block.payload.kind = RichBlockKind::Image;
        block.payload.payload = BlockPayload::Image(ImagePayload {
            source: path,
            alt: if renderer == "math" {
                "数学公式".to_owned()
            } else {
                "Mermaid 图表".to_owned()
            },
            caption: String::new(),
            display_width_ratio_milli: None,
        });
    }
    (document, pending)
}

fn without_comment_blocks(document: &EditorDocument) -> EditorDocument {
    let mut excluded_ids: HashSet<u64> = document
        .blocks
        .iter()
        .filter(|block| {
            matches!(
                block.payload.kind,
                cditor_app::core::rich_text::RichBlockKind::Comment
            )
        })
        .map(|block| block.id)
        .collect();

    loop {
        let previous_len = excluded_ids.len();
        for block in &document.blocks {
            if block
                .parent_id
                .is_some_and(|parent_id| excluded_ids.contains(&parent_id))
            {
                excluded_ids.insert(block.id);
            }
        }
        if excluded_ids.len() == previous_len {
            break;
        }
    }

    let mut export_document = document.clone();
    export_document
        .blocks
        .retain(|block| !excluded_ids.contains(&block.id));
    export_document
}

fn strip_whiteboard_metadata_comments(source: &str) -> String {
    let mut filtered = String::with_capacity(source.len());
    for line in source.split_inclusive('\n') {
        if line.trim_start().starts_with("<!-- cditor:whiteboard ") && line.contains("-->") {
            continue;
        }
        filtered.push_str(line);
    }
    filtered
}

fn export_theme(
    background: Hsla,
    foreground: Hsla,
    muted: Hsla,
    border: Hsla,
    accent: Hsla,
    danger: Hsla,
) -> extension_wasm::DocumentExportTheme {
    let background = rgb24(background);
    extension_wasm::DocumentExportTheme {
        dark: ((background >> 16) & 0xff) + ((background >> 8) & 0xff) + (background & 0xff) < 384,
        background,
        foreground: rgb24(foreground),
        border: rgb24(border),
        muted: rgb24(muted),
        accent: rgb24(accent),
        danger: rgb24(danger),
        font_family: String::new(),
    }
}

fn rgb24(color: Hsla) -> u32 {
    let color = Rgba::from(color);
    let channel = |value: f32| (value.clamp(0.0, 1.0) * 255.0).round() as u32;
    (channel(color.r) << 16) | (channel(color.g) << 8) | channel(color.b)
}

fn next_export_path(directory: &Path, title: &str, extension: &str) -> anyhow::Result<PathBuf> {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty() || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        anyhow::bail!("extension exporter returned an invalid file extension")
    }
    let mut stem = title.trim().to_owned();
    if stem.is_empty() {
        stem = "note".to_owned();
    }
    stem.retain(|character| !matches!(character, '/' | '\\' | ':' | '\0'));
    let first = directory.join(format!("{stem}.{extension}"));
    if !first.exists() {
        return Ok(first);
    }
    for index in 2..=9999 {
        let candidate = directory.join(format!("{stem} ({index}).{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    anyhow::bail!("too many existing exports for {stem}")
}

#[cfg(test)]
mod tests {
    use super::{
        NotesExportFormat, collect_export_assets, document_for_preview, export_rich_text_document,
        next_export_path, strip_whiteboard_metadata_comments, without_comment_blocks,
    };
    use cditor_app::core::rich_text::{
        BlockAttrs, BlockPayload, BlockPayloadRecord, RichBlockKind, WhiteboardPayload,
        kind_tag_for_rich_block_kind,
    };
    use cditor_app::{EditorBlock, EditorDocument};

    #[test]
    fn export_submenu_exposes_html_pdf_and_word() {
        assert_eq!(
            NotesExportFormat::ALL.map(NotesExportFormat::label),
            ["HTML", "PDF", "Word (.docx)"]
        );
    }

    #[test]
    fn all_export_formats_route_to_the_document_exporter_marketplace_entry() {
        assert_eq!(
            "Notes HTML Exporter",
            NotesExportFormat::Html.marketplace_query()
        );
        assert_eq!(
            "Notes PDF Exporter",
            NotesExportFormat::Pdf.marketplace_query()
        );
        assert_eq!(
            "Notes Word Exporter",
            NotesExportFormat::Word.marketplace_query()
        );
    }

    #[test]
    fn export_snapshot_always_uses_preview_semantics() {
        let mut html_preview = editor_block(3, None, RichBlockKind::Html, "");
        html_preview.payload.payload = BlockPayload::Html {
            html: "<strong>preview</strong>".to_owned(),
            sanitized: false,
        };
        let mut html_source = editor_block(4, None, RichBlockKind::Html, "");
        html_source.payload.payload = BlockPayload::Html {
            html: "<em>source</em>".to_owned(),
            sanitized: false,
        };
        let document = editor_document(vec![
            editor_block(1, None, RichBlockKind::Math, "x^2"),
            editor_block(2, None, RichBlockKind::Mermaid, "flowchart TD\nA-->B"),
            html_preview,
            html_source,
        ]);
        let (prepared, pending) = document_for_preview(&document);
        let markdown = export_rich_text_document(&prepared).unwrap();

        assert_eq!(pending.len(), 2);
        assert_eq!(pending[0].renderer, "math");
        assert_eq!(pending[1].renderer, "mermaid");
        assert!(markdown.contains("![数学公式](<navop-export/rendered-block-1.svg>)"));
        assert!(markdown.contains("![Mermaid 图表](<navop-export/rendered-block-2.svg>)"));
        assert!(markdown.contains("<strong>preview</strong>"));
        assert!(markdown.contains("<em>source</em>"));
        assert!(!markdown.contains("```html"));
    }

    #[test]
    fn export_path_is_unique_and_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ab.html"), b"old").unwrap();
        let path = next_export_path(dir.path(), "a/b", "html").unwrap();
        assert_eq!("ab (2).html", path.file_name().unwrap().to_string_lossy());
    }

    #[test]
    fn rich_text_export_excludes_comment_blocks() {
        let document = editor_document(vec![
            editor_block(1, None, RichBlockKind::Paragraph, "Before"),
            editor_block(2, None, RichBlockKind::Comment, "Internal annotation"),
            editor_block(
                3,
                None,
                RichBlockKind::Paragraph,
                "A normal comment remains",
            ),
            editor_block(4, None, RichBlockKind::Paragraph, "After"),
        ]);

        let markdown = export_rich_text_document(&document).unwrap();

        assert_eq!("Before\n\nA normal comment remains\n\nAfter", markdown);
        assert!(!markdown.contains("Internal annotation"));
    }

    #[test]
    fn comment_descendants_are_excluded_from_export_snapshot() {
        let document = editor_document(vec![
            editor_block(1, None, RichBlockKind::Paragraph, "Before"),
            editor_block(2, None, RichBlockKind::Comment, "Annotation"),
            editor_block(3, Some(2), RichBlockKind::Paragraph, "Annotation detail"),
            editor_block(4, None, RichBlockKind::Paragraph, "After"),
        ]);

        let filtered = without_comment_blocks(&document);

        assert_eq!(
            vec![1, 4],
            filtered
                .blocks
                .iter()
                .map(|block| block.id)
                .collect::<Vec<_>>()
        );
        assert_eq!(4, document.blocks.len());
    }

    #[test]
    fn filtering_comments_preserves_whiteboard_blocks_unchanged() {
        let whiteboard = whiteboard_block(1, r#"{"elements":[]}"#);
        let document = editor_document(vec![
            whiteboard.clone(),
            editor_block(2, None, RichBlockKind::Comment, "Annotation"),
        ]);

        let filtered = without_comment_blocks(&document);

        assert_eq!(vec![whiteboard], filtered.blocks);
    }

    #[test]
    fn whiteboard_metadata_comment_is_removed_without_removing_preview() {
        let source = "Before\n<!-- cditor:whiteboard {\"block_id\":1} -->\n![Whiteboard](assets/whiteboard-1.svg)\nAfter";

        assert_eq!(
            "Before\n![Whiteboard](assets/whiteboard-1.svg)\nAfter",
            strip_whiteboard_metadata_comments(source)
        );
    }

    #[test]
    fn ordinary_html_comments_are_not_removed() {
        let source = "Before\n<!-- user note -->\nAfter";

        assert_eq!(source, strip_whiteboard_metadata_comments(source));
    }

    #[test]
    fn local_markdown_images_are_attached_to_export_requests() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("diagram.png"), b"png-data").unwrap();

        let assets = collect_export_assets(
            "![Diagram](diagram.png)\n![Again](diagram.png)",
            Some(dir.path()),
        );

        assert_eq!(1, assets.len());
        assert_eq!("diagram.png", assets[0].path);
        assert_eq!("image/png", assets[0].media_type);
        assert_eq!(b"png-data", assets[0].bytes.as_slice());
    }

    #[test]
    fn table_and_html_images_are_all_attached_to_export_requests() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["left.png", "right.png", "html.png"] {
            std::fs::write(dir.path().join(name), name.as_bytes()).unwrap();
        }
        let source = concat!(
            "| 左 | 右 |\n| --- | --- |\n",
            "| ![左](left.png) | ![右](right.png) |\n\n",
            "<figure><img src=\"html.png\" alt=\"HTML\"></figure>"
        );

        let assets = collect_export_assets(source, Some(dir.path()));
        let paths = assets
            .iter()
            .map(|asset| asset.path.as_str())
            .collect::<Vec<_>>();

        assert_eq!(paths, vec!["left.png", "right.png", "html.png"]);
    }

    fn editor_document(blocks: Vec<EditorBlock>) -> EditorDocument {
        EditorDocument {
            schema_version: EditorDocument::CURRENT_SCHEMA_VERSION,
            document_id: "export-test".to_owned(),
            structure_version: 1,
            blocks,
        }
    }

    fn editor_block(
        id: u64,
        parent_id: Option<u64>,
        kind: RichBlockKind,
        text: &str,
    ) -> EditorBlock {
        EditorBlock {
            id,
            parent_id,
            depth: u16::from(parent_id.is_some()),
            kind_tag: kind_tag_for_rich_block_kind(&kind),
            flags: 0,
            estimated_height: 32.0,
            payload: BlockPayloadRecord::rich_text(id, kind, text),
            attrs: BlockAttrs::default(),
            raw_fallback: None,
        }
    }

    fn whiteboard_block(id: u64, scene_json: &str) -> EditorBlock {
        EditorBlock {
            id,
            parent_id: None,
            depth: 0,
            kind_tag: kind_tag_for_rich_block_kind(&RichBlockKind::Whiteboard),
            flags: 0,
            estimated_height: 220.0,
            payload: BlockPayloadRecord {
                block_id: id,
                content_version: 1,
                kind: RichBlockKind::Whiteboard,
                payload: BlockPayload::Whiteboard(WhiteboardPayload {
                    scene_json: scene_json.to_owned(),
                }),
            },
            attrs: BlockAttrs::default(),
            raw_fallback: None,
        }
    }
}
