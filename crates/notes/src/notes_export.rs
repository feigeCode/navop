use crate::notes_notifications::notify_operation_error;
use crate::{MarkdownViewMode, NodeKind, NotesView, TreeRow};
use anyhow::Context as _;
use futures::AsyncReadExt;
use gpui::{App, AppContext, AsyncApp, Context, Hsla, PathPromptOptions, Rgba, Window};
use gpui_component::{ActiveTheme, WindowExt, notification::Notification};
use one_core::tab_container::{TabContentEvent, TabItem, TabOpenMode};
use rust_i18n::t;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock};

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
        if row.kind != NodeKind::Document {
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
        let source = match self.source_for_export(&row, cx) {
            Ok(source) => source,
            Err(error) => {
                notify_operation_error(window, cx, error);
                return;
            }
        };
        let request = extension_wasm::DocumentExportRequest {
            exporter: String::new(),
            format: format_name,
            title: row.display_name,
            source: source.source,
            assets: source.assets,
            theme: export_theme_from_app(cx),
        };
        self.prompt_and_export(catalog, request, cx.http_client(), window, cx);
    }

    fn prompt_and_export(
        &self,
        catalog: Arc<extension_runtime::ExtensionRuntimeCatalog>,
        request: extension_wasm::DocumentExportRequest,
        http_client: Arc<dyn gpui::http_client::HttpClient>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("Notes.select_export_directory").into()),
        });
        let window_handle = window.window_handle();
        cx.spawn(async move |_, cx: &mut AsyncApp| {
            let directory = match prompt.await {
                Ok(Ok(Some(paths))) => paths.into_iter().next(),
                Ok(Ok(None)) => None,
                Ok(Err(error)) => {
                    notify_async_export_error(cx, window_handle, error.to_string());
                    return;
                }
                Err(error) => {
                    notify_async_export_error(cx, window_handle, error.to_string());
                    return;
                }
            };
            let Some(directory) = directory else { return };
            notify_export_started(cx, window_handle);
            let result = cx
                .background_spawn(run_document_export(
                    catalog,
                    request,
                    directory,
                    http_client,
                ))
                .await;
            notify_export_finished(cx, window_handle, result);
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
        if let Some(session) = self
            .markdown_sessions
            .values()
            .find(|session| session.relative_path == row.relative_path)
        {
            let source = match session.state.mode {
                MarkdownViewMode::Source => session.source_editor.read(cx).value().to_string(),
                MarkdownViewMode::Wysiwyg => session.preview.read(cx).source().to_owned(),
            };
            let path = session.store.path()?;
            return Ok(ExportSource::new(source, path.parent()));
        }

        let descriptor = self.storage()?.descriptor(&row.relative_path)?;
        let source = fs::read_to_string(&descriptor.absolute_path)
            .with_context(|| format!("read Markdown {}", descriptor.absolute_path.display()))?;
        Ok(ExportSource::new(source, descriptor.absolute_path.parent()))
    }
}

struct ExportSource {
    source: String,
    assets: Vec<extension_wasm::DocumentExportAsset>,
}

impl ExportSource {
    fn new(source: String, base: Option<&Path>) -> Self {
        let assets = collect_export_assets(&source, base);
        Self { source, assets }
    }
}

async fn run_document_export(
    catalog: Arc<extension_runtime::ExtensionRuntimeCatalog>,
    mut request: extension_wasm::DocumentExportRequest,
    directory: PathBuf,
    http_client: Arc<dyn gpui::http_client::HttpClient>,
) -> anyhow::Result<PathBuf> {
    request
        .assets
        .extend(collect_remote_export_assets(&request.source, http_client).await);
    prepare_export_image_assets(&request.format, &mut request.assets)?;
    let title = request.title.clone();
    let artifact = catalog
        .export_document(request)
        .await
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .ok_or_else(|| anyhow::anyhow!("no document exporter supports this format"))?;
    let path = next_export_path(&directory, &title, &artifact.extension)?;
    fs::write(&path, artifact.bytes).with_context(|| format!("write export {}", path.display()))?;
    Ok(path)
}

fn notify_async_export_error(
    cx: &mut AsyncApp,
    window_handle: gpui::AnyWindowHandle,
    error: String,
) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.push_notification(
            Notification::error(t!("Notes.operation_failed", error = error).to_string())
                .autohide(false),
            cx,
        );
    });
}

fn notify_export_started(cx: &mut AsyncApp, window_handle: gpui::AnyWindowHandle) {
    let _ = cx.update_window(window_handle, |_, window, cx| {
        window.push_notification(
            Notification::info(t!("Notes.export_started").to_string()).autohide(true),
            cx,
        );
    });
}

fn notify_export_finished(
    cx: &mut AsyncApp,
    window_handle: gpui::AnyWindowHandle,
    result: anyhow::Result<PathBuf>,
) {
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
}

fn export_theme_from_app(cx: &App) -> extension_wasm::DocumentExportTheme {
    export_theme(
        cx.theme().background,
        cx.theme().foreground,
        cx.theme().muted_foreground,
        cx.theme().border,
        cx.theme().primary,
        cx.theme().danger,
    )
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

fn collect_export_assets(
    source: &str,
    base: Option<&Path>,
) -> Vec<extension_wasm::DocumentExportAsset> {
    let Some(base) = base else { return Vec::new() };
    let mut assets = Vec::new();
    let mut seen = HashSet::new();
    for target in export_asset_paths(source) {
        let path = export_asset_target(target);
        if path.is_empty() || is_external_export_asset(path) || !seen.insert(path.to_owned()) {
            continue;
        }
        let file = base.join(path);
        let Ok(bytes) = fs::read(&file) else { continue };
        assets.push(extension_wasm::DocumentExportAsset {
            path: path.to_owned(),
            media_type: export_asset_media_type(&file).to_owned(),
            bytes,
        });
    }
    assets
}

fn is_external_export_asset(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.contains("://") || lower.starts_with("data:") || lower.starts_with("asset:")
}

fn export_asset_media_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default()
        .to_ascii_lowercase()
        .as_str()
    {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => "application/octet-stream",
    }
}

async fn collect_remote_export_assets(
    source: &str,
    http_client: Arc<dyn gpui::http_client::HttpClient>,
) -> Vec<extension_wasm::DocumentExportAsset> {
    let mut assets = Vec::new();
    let mut seen = HashSet::new();
    for target in export_asset_paths(source) {
        let path = export_asset_target(target);
        let lower = path.to_ascii_lowercase();
        if !(lower.starts_with("https://") || lower.starts_with("http://"))
            || !seen.insert(path.to_owned())
        {
            continue;
        }
        let Ok(response) = http_client
            .get(path, gpui::http_client::AsyncBody::default(), true)
            .await
        else {
            continue;
        };
        if !response.status().is_success() {
            continue;
        }
        let mut body = response.into_body();
        let mut bytes = Vec::new();
        if body.read_to_end(&mut bytes).await.is_err() || bytes.is_empty() {
            continue;
        }
        assets.push(extension_wasm::DocumentExportAsset {
            path: path.to_owned(),
            media_type: export_image_media_type(path, &bytes).to_owned(),
            bytes,
        });
    }
    assets
}

fn export_image_media_type(path: &str, bytes: &[u8]) -> &'static str {
    let trimmed = bytes
        .iter()
        .copied()
        .skip_while(u8::is_ascii_whitespace)
        .take(64)
        .collect::<Vec<_>>();
    if trimmed.starts_with(b"<svg") || trimmed.starts_with(b"<?xml") {
        return "image/svg+xml";
    }
    if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return "image/png";
    }
    if bytes.starts_with(b"\xff\xd8\xff") {
        return "image/jpeg";
    }
    if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        return "image/gif";
    }
    if bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP") {
        return "image/webp";
    }
    let path = path.split(['?', '#']).next().unwrap_or(path);
    export_asset_media_type(Path::new(path))
}

static EXPORT_SVG_FONT_DB: LazyLock<Arc<resvg::usvg::fontdb::Database>> = LazyLock::new(|| {
    let mut font_db = resvg::usvg::fontdb::Database::new();
    font_db.load_system_fonts();
    Arc::new(font_db)
});

fn prepare_export_image_assets(
    format: &str,
    assets: &mut [extension_wasm::DocumentExportAsset],
) -> anyhow::Result<()> {
    if !matches!(format.to_ascii_lowercase().as_str(), "pdf" | "docx") {
        return Ok(());
    }
    for asset in assets {
        let png = match asset.media_type.as_str() {
            "image/svg+xml" => rasterize_export_svg(&asset.bytes)
                .and_then(|pixmap| pixmap.encode_png().map_err(anyhow::Error::from))
                .with_context(|| format!("rasterize image asset {}", asset.path))?,
            "image/png" | "image/jpeg" | "image/gif" | "image/webp" => {
                normalize_export_raster_image(&asset.bytes)
                    .with_context(|| format!("normalize image asset {}", asset.path))?
            }
            _ => continue,
        };
        asset.bytes = png;
        asset.media_type = "image/png".to_owned();
    }
    Ok(())
}

fn normalize_export_raster_image(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let image = image::load_from_memory(bytes).context("decode image")?;
    let image = if image.width() > 400 || image.height() > 300 {
        image.thumbnail(400, 300)
    } else {
        image
    };
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .context("encode PNG")?;
    Ok(output.into_inner())
}

fn rasterize_export_svg(svg: &[u8]) -> anyhow::Result<resvg::tiny_skia::Pixmap> {
    let mut options = resvg::usvg::Options::default();
    options.fontdb = Arc::clone(&EXPORT_SVG_FONT_DB);
    let tree = resvg::usvg::Tree::from_data(svg, &options).context("parse SVG")?;
    let size = tree.size();
    let scale = (400.0 / size.width()).min(300.0 / size.height()).min(1.0);
    let width = (size.width() * scale).round().max(1.0) as u32;
    let height = (size.height() * scale).round().max(1.0) as u32;
    let mut pixmap =
        resvg::tiny_skia::Pixmap::new(width, height).context("SVG raster size is invalid")?;
    resvg::render(
        &tree,
        resvg::tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

fn export_asset_paths(source: &str) -> Vec<&str> {
    let mut paths = markdown_export_asset_paths(source);
    paths.extend(html_export_asset_paths(source));
    paths
}

fn markdown_export_asset_paths(source: &str) -> Vec<&str> {
    let mut paths = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = source[cursor..].find("![") {
        let image_start = cursor + relative;
        let label_start = image_start + 2;
        let Some(label_end) = source[label_start..].find("](") else {
            cursor = label_start;
            continue;
        };
        let target_start = label_start + label_end + 2;
        let Some(target_end) = markdown_target_end(&source[target_start..]) else {
            cursor = target_start;
            continue;
        };
        paths.push(source[target_start..target_start + target_end].trim());
        cursor = target_start + target_end + 1;
    }
    paths
}

fn markdown_target_end(target: &str) -> Option<usize> {
    let mut escaped = false;
    let mut nested = 0usize;
    let mut in_angle = false;
    for (index, character) in target.char_indices() {
        if escaped {
            escaped = false;
            continue;
        }
        match character {
            '\\' => escaped = true,
            '<' if target[..index].trim().is_empty() => in_angle = true,
            '>' if in_angle => in_angle = false,
            '(' if !in_angle => nested = nested.saturating_add(1),
            ')' if !in_angle && nested == 0 => return Some(index),
            ')' if !in_angle => nested -= 1,
            _ => {}
        }
    }
    None
}

fn html_export_asset_paths(source: &str) -> Vec<&str> {
    let lower = source.to_ascii_lowercase();
    let mut paths = Vec::new();
    let mut cursor = 0;
    while let Some(relative) = lower[cursor..].find("<img") {
        let start = cursor + relative;
        cursor = start + 4;
        if !is_html_img_tag(&lower, start) {
            continue;
        }
        let Some(end) = source[start..].find('>').map(|offset| start + offset) else {
            break;
        };
        if let Some(path) = html_export_attribute(&source[start..=end], "src") {
            paths.push(path);
        }
        cursor = end + 1;
    }
    paths
}

fn is_html_img_tag(source: &str, start: usize) -> bool {
    source
        .as_bytes()
        .get(start + 4)
        .is_none_or(|byte| byte.is_ascii_whitespace() || matches!(byte, b'/' | b'>'))
}

fn html_export_attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let mut cursor = 4;
    while cursor < bytes.len() {
        while bytes
            .get(cursor)
            .is_some_and(|byte| byte.is_ascii_whitespace() || *byte == b'/')
        {
            cursor += 1;
        }
        let attribute_start = cursor;
        while bytes
            .get(cursor)
            .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'=' | b'>' | b'/'))
        {
            cursor += 1;
        }
        let attribute = tag.get(attribute_start..cursor)?;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        if bytes.get(cursor) != Some(&b'=') {
            cursor = cursor.saturating_add(1);
            continue;
        }
        cursor += 1;
        while bytes.get(cursor).is_some_and(u8::is_ascii_whitespace) {
            cursor += 1;
        }
        let value = html_attribute_value(tag, &mut cursor)?;
        if attribute.eq_ignore_ascii_case(name) {
            return Some(value);
        }
    }
    None
}

fn html_attribute_value<'a>(tag: &'a str, cursor: &mut usize) -> Option<&'a str> {
    let bytes = tag.as_bytes();
    let quote = bytes.get(*cursor).copied()?;
    if matches!(quote, b'\'' | b'"') {
        *cursor += 1;
        let start = *cursor;
        while bytes.get(*cursor).is_some_and(|byte| *byte != quote) {
            *cursor += 1;
        }
        let value = tag.get(start..*cursor)?;
        *cursor = cursor.saturating_add(1);
        return Some(value);
    }
    let start = *cursor;
    while bytes
        .get(*cursor)
        .is_some_and(|byte| !byte.is_ascii_whitespace() && !matches!(byte, b'>' | b'/'))
    {
        *cursor += 1;
    }
    tag.get(start..*cursor)
}

fn export_asset_target(target: &str) -> &str {
    let target = target.trim();
    if let Some(target) = target.strip_prefix('<') {
        target
            .split_once('>')
            .map(|(path, _)| path)
            .unwrap_or(target)
    } else {
        target.split_whitespace().next().unwrap_or(target)
    }
}

fn next_export_path(directory: &Path, title: &str, extension: &str) -> anyhow::Result<PathBuf> {
    let extension = extension.trim_start_matches('.');
    if extension.is_empty() || !extension.bytes().all(|byte| byte.is_ascii_alphanumeric()) {
        anyhow::bail!("extension exporter returned an invalid file extension");
    }
    let mut stem = title.trim().to_owned();
    stem.retain(|character| {
        !character.is_control() && !matches!(character, '/' | '\\' | ':' | '\0')
    });
    if stem.is_empty() {
        stem = "note".to_owned();
    }
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
    anyhow::bail!("too many existing exports for {stem}");
}

#[cfg(test)]
mod tests {
    use super::{
        ExportSource, NotesExportFormat, collect_export_assets, export_image_media_type,
        next_export_path, prepare_export_image_assets, rasterize_export_svg,
    };

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
    fn export_source_preserves_markdown_bytes_exactly() {
        let source = "\u{feff}# 标题\r\n\r\n结尾无换行".to_owned();
        let export = ExportSource::new(source.clone(), None);
        assert_eq!(source, export.source);
    }

    #[test]
    fn export_path_is_unique_and_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ab.html"), b"old").unwrap();
        let path = next_export_path(dir.path(), "a/b", "html").unwrap();
        assert_eq!("ab (2).html", path.file_name().unwrap().to_string_lossy());
        assert_eq!(
            "note.pdf",
            next_export_path(dir.path(), "/:\\", ".pdf")
                .unwrap()
                .file_name()
                .unwrap()
                .to_string_lossy()
        );
    }

    #[test]
    fn exporter_extension_must_be_a_simple_file_extension() {
        let dir = tempfile::tempdir().unwrap();
        for extension in ["", ".", "../pdf", "tar.gz", "pd/f", "文档"] {
            assert!(
                next_export_path(dir.path(), "note", extension).is_err(),
                "extension={extension:?}"
            );
        }
    }

    #[test]
    fn pdf_and_word_images_are_normalized_by_the_host() {
        let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="160" height="40"><rect width="160" height="40" fill="#2563eb"/></svg>"##;
        let pixmap = rasterize_export_svg(svg).unwrap();
        assert!(pixmap.pixels().iter().any(|pixel| pixel.alpha() > 0));

        let mut large_png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::new_rgb8(800, 600)
            .write_to(&mut large_png, image::ImageFormat::Png)
            .unwrap();
        let assets = vec![
            extension_wasm::DocumentExportAsset {
                path: "diagram.svg".to_owned(),
                media_type: "image/svg+xml".to_owned(),
                bytes: svg.to_vec(),
            },
            extension_wasm::DocumentExportAsset {
                path: "screenshot.png".to_owned(),
                media_type: "image/png".to_owned(),
                bytes: large_png.into_inner(),
            },
        ];

        for format in ["pdf", "docx"] {
            let mut normalized_assets = assets.clone();
            prepare_export_image_assets(format, &mut normalized_assets).unwrap();
            for asset in &normalized_assets {
                assert_eq!(asset.media_type, "image/png");
                assert!(asset.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
            }
            let normalized = image::load_from_memory(&normalized_assets[1].bytes).unwrap();
            assert_eq!((normalized.width(), normalized.height()), (400, 300));
        }
    }

    #[test]
    fn remote_image_media_type_uses_content_before_url_extension() {
        assert_eq!(
            export_image_media_type(
                "https://img.example/badge?style=flat",
                b"\n <svg xmlns=\"http://www.w3.org/2000/svg\"></svg>",
            ),
            "image/svg+xml"
        );
        assert_eq!(
            export_image_media_type("https://example.com/wrong.png", b"\xff\xd8\xffjpeg"),
            "image/jpeg"
        );
        assert_eq!(
            export_image_media_type("https://example.com/photo.webp?size=2", b"unknown"),
            "image/webp"
        );
    }

    #[test]
    fn local_markdown_images_are_attached_once() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("diagram.png"), b"png-data").unwrap();
        let assets = collect_export_assets(
            "![Diagram](diagram.png)\n![Again](diagram.png \"Title\")",
            Some(dir.path()),
        );
        assert_eq!(1, assets.len());
        assert_eq!("diagram.png", assets[0].path);
        assert_eq!("image/png", assets[0].media_type);
        assert_eq!(b"png-data", assets[0].bytes.as_slice());
    }

    #[test]
    fn angle_wrapped_image_paths_can_contain_spaces_and_titles() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("image with spaces.png"), b"image").unwrap();
        let assets = collect_export_assets(
            "![Diagram](<image with spaces.png> \"A title\")",
            Some(dir.path()),
        );
        assert_eq!(1, assets.len());
        assert_eq!("image with spaces.png", assets[0].path);
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
            "<figure><img ALT='HTML' src = \"html.png\"></figure>"
        );
        let paths = collect_export_assets(source, Some(dir.path()))
            .into_iter()
            .map(|asset| asset.path)
            .collect::<Vec<_>>();
        assert_eq!(paths, vec!["left.png", "right.png", "html.png"]);
    }

    #[test]
    fn remote_data_and_asset_images_are_not_read_as_local_files() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["https:", "DATA:image", "asset:preview"] {
            let _ = std::fs::write(dir.path().join(name), b"must-not-export");
        }
        let assets = collect_export_assets(
            concat!(
                "![Remote](https://example.com/a.png)\n",
                "![Data](DATA:image/png;base64,AAAA)\n",
                "<img src='asset:preview'>"
            ),
            Some(dir.path()),
        );
        assert!(assets.is_empty());
    }

    #[test]
    fn html_parser_does_not_treat_data_src_or_image_tags_as_img_src() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("real.png"), b"real").unwrap();
        std::fs::write(dir.path().join("lazy.png"), b"lazy").unwrap();
        let assets = collect_export_assets(
            "<image src='real.png'><img data-src='lazy.png' src='real.png'>",
            Some(dir.path()),
        );
        assert_eq!(1, assets.len());
        assert_eq!("real.png", assets[0].path);
    }
}
