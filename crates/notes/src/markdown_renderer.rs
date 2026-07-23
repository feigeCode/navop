pub(crate) fn block_render_provider(
    cx: &gpui::App,
) -> Option<markdown_editor::MarkdownBlockRenderProvider> {
    let catalog = cx
        .try_global::<extension_runtime::GlobalExtensionRuntimeCatalog>()?
        .get()?;
    Some(std::sync::Arc::new(move |request| {
        let catalog = catalog.clone();
        Box::pin(async move {
            let artifact = catalog
                .render_document(extension_request(request))
                .await
                .map_err(|error| error.to_string())?;
            Ok(artifact.map(map_artifact))
        })
    }))
}

pub(crate) fn markdown_editor_theme(
    theme: crate::theme_provider::MarkdownEditorTheme,
) -> markdown_editor::MarkdownEditorTheme {
    markdown_editor::MarkdownEditorTheme {
        background: theme.background,
        foreground: theme.foreground,
        muted_foreground: theme.muted_foreground,
        border: theme.border,
        primary: theme.primary,
        highlight_theme: theme.highlight_theme,
    }
}

fn extension_request(
    request: markdown_editor::MarkdownBlockRenderRequest,
) -> extension_runtime::DocumentRenderRequest {
    let renderer = match request.kind {
        markdown_editor::MarkdownBlockRenderKind::Math => "math",
        markdown_editor::MarkdownBlockRenderKind::Mermaid => "mermaid",
    };
    extension_runtime::DocumentRenderRequest {
        renderer: renderer.to_owned(),
        source: request.source,
        theme: extension_runtime::DocumentRenderTheme {
            dark: request.background.l < 0.5,
            background: color_u32(request.background),
            foreground: color_u32(request.foreground),
            border: color_u32(request.border),
            muted: color_u32(request.muted),
            accent: color_u32(request.accent),
            danger: 0xeb5757,
            font_family: "system-ui, sans-serif".to_owned(),
        },
        available_width: request.available_width,
        scale_factor: request.scale_factor,
    }
}

fn map_artifact(
    artifact: extension_runtime::DocumentRenderArtifact,
) -> markdown_editor::MarkdownBlockRenderArtifact {
    markdown_editor::MarkdownBlockRenderArtifact {
        media_type: artifact.media_type,
        bytes: artifact.bytes,
        intrinsic_width: artifact.intrinsic_width,
        intrinsic_height: artifact.intrinsic_height,
    }
}

fn color_u32(color: gpui::Hsla) -> u32 {
    let rgb = color.to_rgb();
    ((rgb.r * 255.) as u32) << 16 | ((rgb.g * 255.) as u32) << 8 | (rgb.b * 255.) as u32
}
