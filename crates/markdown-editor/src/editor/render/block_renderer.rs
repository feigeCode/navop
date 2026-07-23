use super::MarkdownEditor;
use crate::{MarkdownBlockRenderKind, MarkdownBlockRenderRequest};
use gpui::{
    AppContext, Context, Image, ImageFormat, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Styled, StyledImage, img, px,
};
use markdown_source::{SourceBlock, SourceBlockKind, SourceNodeId};
use std::ops::Range;
use std::sync::Arc;

const MAX_CONCURRENT_BLOCK_RENDERS: usize = 4;

impl MarkdownEditor {
    pub(super) fn request_inline_math_renders(
        &mut self,
        range: Range<usize>,
        cx: &mut Context<Self>,
    ) {
        if self.block_render_provider.is_none() {
            return;
        }
        let sources = self
            .history
            .document()
            .blocks
            .get(range)
            .unwrap_or_default()
            .iter()
            .flat_map(|block| block.inline_nodes.iter())
            .filter_map(|node| match node.kind {
                markdown_source::SourceInlineKind::InlineMath { .. } => node.content_range.as_ref(),
                _ => None,
            })
            .map(|range| self.history.document().source[range.clone()].to_owned())
            .filter(|source| {
                !self.inline_math_artifacts.contains_key(source)
                    && !self.pending_inline_math_renders.contains(source)
                    && !self.failed_inline_math_renders.contains(source)
            })
            .take(MAX_CONCURRENT_BLOCK_RENDERS)
            .collect::<Vec<_>>();
        for source in sources {
            self.spawn_inline_math_render(source, cx);
        }
    }

    fn spawn_inline_math_render(&mut self, source: String, cx: &mut Context<Self>) {
        let Some(provider) = self.block_render_provider.clone() else {
            return;
        };
        let request = self.render_request(MarkdownBlockRenderKind::Math, source.clone());
        let generation = self.block_render_generation;
        self.pending_inline_math_renders.insert(source.clone());
        let weak = cx.entity().downgrade();
        let task = cx.background_spawn(async move { provider(request).await });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = weak.update(cx, |editor, cx| {
                editor.finish_inline_math_render(source, generation, result);
                editor.refresh_projection_highlights(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_inline_math_render(
        &mut self,
        source: String,
        generation: u64,
        result: Result<Option<crate::MarkdownBlockRenderArtifact>, String>,
    ) {
        if self.block_render_generation != generation {
            return;
        }
        self.pending_inline_math_renders.remove(&source);
        match result {
            Ok(Some(artifact)) if artifact.media_type == "image/svg+xml" => {
                self.inline_math_artifacts.insert(source, artifact);
            }
            _ => {
                self.failed_inline_math_renders.insert(source);
            }
        }
    }

    pub(super) fn request_block_renders(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        let available =
            MAX_CONCURRENT_BLOCK_RENDERS.saturating_sub(self.pending_block_renders.len());
        if available == 0 || self.block_render_provider.is_none() {
            return;
        }
        let requests = self
            .history
            .document()
            .blocks
            .get(range)
            .unwrap_or_default()
            .iter()
            .filter_map(|block| self.block_render_request(block))
            .filter(|(id, source, _)| {
                self.block_render_sources.get(id) != Some(source)
                    && self.pending_block_renders.get(id) != Some(source)
            })
            .take(available)
            .collect::<Vec<_>>();
        for (block_id, source, request) in requests {
            self.spawn_block_render(block_id, source, request, cx);
        }
    }

    fn block_render_request(
        &self,
        block: &SourceBlock,
    ) -> Option<(SourceNodeId, String, MarkdownBlockRenderRequest)> {
        let kind = block_render_kind(block, &self.history.document().source)?;
        let source = block
            .content_range
            .as_ref()
            .map(|range| self.history.document().source[range.clone()].to_owned())?;
        Some((block.id, source.clone(), self.render_request(kind, source)))
    }

    fn render_request(
        &self,
        kind: MarkdownBlockRenderKind,
        source: String,
    ) -> MarkdownBlockRenderRequest {
        MarkdownBlockRenderRequest {
            kind,
            source,
            background: self.theme.background,
            foreground: self.theme.foreground,
            border: self.theme.border,
            muted: self.theme.muted_foreground,
            accent: self.theme.primary,
            available_width: 796.,
            scale_factor: 1.,
        }
    }

    fn spawn_block_render(
        &mut self,
        block_id: SourceNodeId,
        source: String,
        request: MarkdownBlockRenderRequest,
        cx: &mut Context<Self>,
    ) {
        let Some(provider) = self.block_render_provider.clone() else {
            return;
        };
        let generation = self.block_render_generation;
        self.pending_block_renders.insert(block_id, source.clone());
        let weak = cx.entity().downgrade();
        let task = cx.background_spawn(async move { provider(request).await });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = weak.update(cx, |editor, cx| {
                editor.finish_block_render(block_id, source, generation, result);
                cx.notify();
            });
        })
        .detach();
    }

    fn finish_block_render(
        &mut self,
        block_id: SourceNodeId,
        source: String,
        generation: u64,
        result: Result<Option<crate::MarkdownBlockRenderArtifact>, String>,
    ) {
        if self.block_render_generation != generation
            || self.pending_block_renders.get(&block_id) != Some(&source)
        {
            return;
        }
        self.pending_block_renders.remove(&block_id);
        if self.block_source(block_id).as_deref() != Some(&source) {
            return;
        }
        self.block_render_sources.insert(block_id, source);
        self.block_render_artifacts.remove(&block_id);
        self.block_render_errors.remove(&block_id);
        match result {
            Ok(Some(artifact)) => {
                self.block_render_artifacts.insert(block_id, artifact);
            }
            Ok(None) => {
                self.block_render_errors
                    .insert(block_id, "Renderer unavailable".to_owned());
            }
            Err(error) => {
                self.block_render_errors.insert(block_id, error);
            }
        }
    }

    fn block_source(&self, block_id: SourceNodeId) -> Option<String> {
        let block = self.history.document().block_by_id(block_id)?;
        let range = block.content_range.as_ref()?;
        Some(self.history.document().source[range.clone()].to_owned())
    }

    pub(super) fn render_block_output(&self, block: &SourceBlock) -> Option<gpui::AnyElement> {
        let source = self.block_source(block.id)?;
        if self.block_render_sources.get(&block.id) != Some(&source) {
            return None;
        }
        if let Some(error) = self.block_render_errors.get(&block.id) {
            return Some(self.render_block_error(block.id, error));
        }
        self.render_block_artifact(block.id)
    }

    fn render_block_artifact(&self, block_id: SourceNodeId) -> Option<gpui::AnyElement> {
        let artifact = self.block_render_artifacts.get(&block_id)?;
        (artifact.media_type == "image/svg+xml").then(|| {
            let image = Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone()));
            let height = artifact.intrinsic_height.unwrap_or(240.).clamp(64., 520.);
            gpui::div()
                .id(("markdown-rendered-block", block_id.0))
                .debug_selector(|| format!("markdown-rendered-block-{}", block_id.0))
                .w_full()
                .min_h(px(64.))
                .h(px(height))
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(self.theme.border)
                .bg(self.theme.background)
                .child(img(image).w_full().h_full().object_fit(ObjectFit::Contain))
                .into_any_element()
        })
    }

    fn render_block_error(&self, block_id: SourceNodeId, error: &str) -> gpui::AnyElement {
        gpui::div()
            .id(("markdown-render-error", block_id.0))
            .w_full()
            .min_h(px(72.))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.border.opacity(0.08))
            .text_sm()
            .text_color(self.theme.muted_foreground)
            .child(format!("Preview unavailable: {error}"))
            .into_any_element()
    }
}

fn block_render_kind(block: &SourceBlock, source: &str) -> Option<MarkdownBlockRenderKind> {
    match &block.kind {
        SourceBlockKind::MathBlock { .. } => Some(MarkdownBlockRenderKind::Math),
        SourceBlockKind::CodeFence {
            language_range: Some(range),
            ..
        } if source[range.clone()].eq_ignore_ascii_case("mermaid") => {
            Some(MarkdownBlockRenderKind::Mermaid)
        }
        _ => None,
    }
}
