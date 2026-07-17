use cditor_app::{
    DocumentRenderArtifact, DocumentRenderError, DocumentRenderFuture, DocumentRenderRequest,
    DocumentRendererProvider,
};
use extension_runtime::GlobalExtensionRuntimeCatalog;

pub(crate) struct NavopDocumentRendererProvider {
    catalog: GlobalExtensionRuntimeCatalog,
}

impl NavopDocumentRendererProvider {
    pub(crate) fn new(catalog: GlobalExtensionRuntimeCatalog) -> Self {
        Self { catalog }
    }
}

impl DocumentRendererProvider for NavopDocumentRendererProvider {
    fn id(&self) -> &str {
        "navop.extensions.document-renderers"
    }
    fn revision(&self) -> u64 {
        self.catalog.revision()
    }
    fn supports(&self, renderer: &str) -> bool {
        self.catalog
            .get()
            .and_then(|catalog| catalog.document_renderer_for_kind(renderer).map(|_| ()))
            .is_some()
    }
    fn render(&self, request: DocumentRenderRequest) -> DocumentRenderFuture {
        let catalog = self.catalog.get();
        Box::pin(async move {
            let catalog = catalog.ok_or_else(|| DocumentRenderError::new("扩展运行时尚未加载"))?;
            let output = catalog
                .render_document(extension_wasm::DocumentRenderRequest {
                    renderer: request.renderer,
                    source: request.source,
                    available_width: request.available_width,
                    scale_factor: request.scale_factor,
                    theme: extension_wasm::DocumentRenderTheme {
                        dark: request.theme.dark,
                        background: request.theme.background,
                        foreground: request.theme.foreground,
                        border: request.theme.border,
                        muted: request.theme.muted,
                        accent: request.theme.accent,
                        danger: request.theme.danger,
                        font_family: request.theme.font_family,
                    },
                })
                .await
                .map_err(|error| DocumentRenderError::new(error.to_string()))?
                .ok_or_else(|| DocumentRenderError::new("没有扩展支持该文档渲染器"))?;
            Ok(DocumentRenderArtifact {
                media_type: output.media_type,
                bytes: output.bytes,
                intrinsic_width: output.intrinsic_width,
                intrinsic_height: output.intrinsic_height,
            })
        })
    }
}
