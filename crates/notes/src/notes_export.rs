use crate::notes_notifications::{notify_error_message, notify_operation_error};
use crate::{DocumentFormat, MarkdownViewMode, NotesView, TreeRow};
use cditor_app::{EditorDocument, MarkdownExportMode};
use gpui::{App, AppContext, AsyncApp, Context, Hsla, PathPromptOptions, Rgba, Window};
use gpui_component::{ActiveTheme, WindowExt, notification::Notification};
use rust_i18n::t;
use std::fs;
use std::path::{Path, PathBuf};

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
        let source = match self.source_for_export(&row, cx) {
            Ok(source) => source,
            Err(error) => {
                notify_operation_error(window, cx, error);
                return;
            }
        };
        let title = row.display_name.clone();
        let Some(global) = cx
            .try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()
            .cloned()
        else {
            notify_error_message(window, cx, t!("Notes.export_unavailable").to_string());
            return;
        };
        let theme = export_theme(
            cx.theme().background,
            cx.theme().foreground,
            cx.theme().muted_foreground,
            cx.theme().border,
            cx.theme().primary,
            cx.theme().danger,
        );
        let prompt = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(t!("Notes.select_export_directory").into()),
        });
        let window_handle = window.window_handle();
        let format_name = format.protocol_name().to_owned();
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
            let Some(catalog) = global.get() else {
                let _ = cx.update_window(window_handle, |_, window, cx| {
                    window.push_notification(
                        Notification::error(t!("Notes.export_unavailable").to_string())
                            .autohide(false),
                        cx,
                    );
                });
                return;
            };
            let result = cx
                .background_spawn(async move {
                    let artifact = catalog
                        .export_document(extension_wasm::DocumentExportRequest {
                            exporter: String::new(),
                            format: format_name,
                            title: title.clone(),
                            source,
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

    fn source_for_export(&self, row: &TreeRow, cx: &App) -> anyhow::Result<String> {
        let descriptor = self.storage()?.descriptor(&row.relative_path)?;
        match descriptor.format {
            DocumentFormat::Markdown => {
                if let Some((_, session)) = self
                    .markdown_sessions
                    .iter()
                    .find(|(_, session)| session.relative_path == row.relative_path)
                {
                    return match session.state.mode {
                        MarkdownViewMode::Source => {
                            Ok(session.source_editor.read(cx).value().to_string())
                        }
                        MarkdownViewMode::Wysiwyg => {
                            crate::markdown_adapter::export_markdown_bundle(
                                &session.preview,
                                &session.store,
                                cx,
                            )
                        }
                    };
                }
                Ok(fs::read_to_string(descriptor.absolute_path)?)
            }
            DocumentFormat::RichText => {
                if let Some(cached) = self
                    .editors
                    .values()
                    .find(|cached| cached.relative_path == row.relative_path)
                {
                    return Ok(cached
                        .handle
                        .export_markdown(MarkdownExportMode::BestEffort, cx)?
                        .markdown);
                }
                let document =
                    EditorDocument::from_json(&fs::read_to_string(descriptor.absolute_path)?)?;
                Ok(document
                    .export_markdown(MarkdownExportMode::BestEffort)?
                    .markdown)
            }
        }
    }
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
    use super::{NotesExportFormat, next_export_path};

    #[test]
    fn export_submenu_exposes_html_pdf_and_word() {
        assert_eq!(
            NotesExportFormat::ALL.map(NotesExportFormat::label),
            ["HTML", "PDF", "Word (.docx)"]
        );
    }

    #[test]
    fn export_path_is_unique_and_sanitized() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("ab.html"), b"old").unwrap();
        let path = next_export_path(dir.path(), "a/b", "html").unwrap();
        assert_eq!("ab (2).html", path.file_name().unwrap().to_string_lossy());
    }
}
