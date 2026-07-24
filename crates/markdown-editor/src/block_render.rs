use futures::future::BoxFuture;
use gpui::Hsla;
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MarkdownBlockRenderKind {
    Math,
    Mermaid,
}

impl std::hash::Hash for MarkdownBlockRenderKind {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (*self as u8).hash(state);
    }
}

#[derive(Clone)]
pub struct MarkdownBlockRenderRequest {
    pub kind: MarkdownBlockRenderKind,
    pub source: String,
    pub background: Hsla,
    pub foreground: Hsla,
    pub border: Hsla,
    pub muted: Hsla,
    pub accent: Hsla,
    pub available_width: f32,
    pub scale_factor: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MarkdownBlockRenderArtifact {
    pub media_type: String,
    pub bytes: Vec<u8>,
    pub intrinsic_width: Option<f32>,
    pub intrinsic_height: Option<f32>,
}

pub type MarkdownBlockRenderProvider = Arc<
    dyn Fn(
            MarkdownBlockRenderRequest,
        ) -> BoxFuture<'static, Result<Option<MarkdownBlockRenderArtifact>, String>>
        + Send
        + Sync,
>;
