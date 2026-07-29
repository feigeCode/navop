use std::sync::Arc;

use crate::{
    BlockRenderArtifact, BlockRenderKind, BlockRenderProvider, BlockRenderRequest,
    EditorHostServices, EditorHostTheme,
};

use crate::{
    MarkdownBlockRenderArtifact, MarkdownBlockRenderKind, MarkdownBlockRenderProvider,
    MarkdownBlockRenderRequest, MarkdownEditorTheme,
};

pub fn markdown_editor_host_services(
    theme: MarkdownEditorTheme,
    block_provider: Option<MarkdownBlockRenderProvider>,
) -> EditorHostServices {
    let host_theme = EditorHostTheme {
        background: theme.background,
        foreground: theme.foreground,
        border: theme.border,
        muted: theme.muted_foreground,
        accent: theme.primary,
    };
    let code_highlighter = crate::wasm_highlight::code_highlight_service(theme.highlight_theme);
    let mut services = EditorHostServices::new(host_theme).with_code_highlighter(code_highlighter);
    if let Some(provider) = block_provider {
        services = services.with_block_renderer(adapt_block_provider(provider));
    }
    services
}

fn adapt_block_provider(provider: MarkdownBlockRenderProvider) -> BlockRenderProvider {
    Arc::new(move |request| {
        let provider = provider.clone();
        let request = markdown_request(request);
        Box::pin(async move {
            provider(request)
                .await
                .map(|artifact| artifact.map(host_artifact))
        })
    })
}

fn markdown_request(request: BlockRenderRequest) -> MarkdownBlockRenderRequest {
    MarkdownBlockRenderRequest {
        kind: match request.kind {
            BlockRenderKind::Math | BlockRenderKind::InlineMath => MarkdownBlockRenderKind::Math,
            BlockRenderKind::Mermaid => MarkdownBlockRenderKind::Mermaid,
        },
        source: request.source,
        background: request.background,
        foreground: request.foreground,
        border: request.border,
        muted: request.muted,
        accent: request.accent,
        available_width: request.available_width,
        scale_factor: request.scale_factor,
    }
}

fn host_artifact(artifact: MarkdownBlockRenderArtifact) -> BlockRenderArtifact {
    BlockRenderArtifact {
        media_type: artifact.media_type,
        bytes: artifact.bytes,
        intrinsic_width: artifact.intrinsic_width,
        intrinsic_height: artifact.intrinsic_height,
    }
}
