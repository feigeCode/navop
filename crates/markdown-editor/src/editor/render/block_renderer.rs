use super::{MarkdownEditor, layout_metrics::render_surface_reserved_height};
use crate::{MarkdownBlockRenderArtifact, MarkdownBlockRenderKind, MarkdownBlockRenderRequest};
use gpui::{
    AppContext, Context, Corners, Image, ImageFormat, InteractiveElement, IntoElement, ObjectFit,
    ParentElement, Styled, canvas, px,
};
use gpui_component::{Sizable, button::Button};
use markdown_source::{SourceBlock, SourceBlockKind, SourceNodeId};
use std::ops::Range;
use std::sync::Arc;

const MAX_CONCURRENT_BLOCK_RENDERS: usize = 4;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(in crate::editor) struct RenderCacheKey {
    kind: MarkdownBlockRenderKind,
    source: String,
    background: [u32; 4],
    foreground: [u32; 4],
    border: [u32; 4],
    muted: [u32; 4],
    accent: [u32; 4],
    available_width: u32,
    scale_factor: u32,
}

impl RenderCacheKey {
    pub(in crate::editor) fn from_request(request: &MarkdownBlockRenderRequest) -> Self {
        Self {
            kind: request.kind,
            source: request.source.clone(),
            background: color_key(request.background),
            foreground: color_key(request.foreground),
            border: color_key(request.border),
            muted: color_key(request.muted),
            accent: color_key(request.accent),
            available_width: request.available_width.to_bits(),
            scale_factor: request.scale_factor.to_bits(),
        }
    }
}

#[derive(Clone)]
pub(in crate::editor) enum CachedRender {
    Artifact(MarkdownBlockRenderArtifact),
    Error(String),
}

#[derive(Clone)]
pub(in crate::editor) enum RenderWaiter {
    Block {
        block_id: SourceNodeId,
        source: String,
    },
    InlineMath {
        source: String,
    },
}

impl MarkdownEditor {
    /// Whether this block owns a permanent rendered/source shell.
    ///
    /// Provider availability decides the shell shape. Render completion only
    /// changes the child inside its rendered layer, so a pending request never
    /// falls back to a differently shaped Input-only tree.
    pub(super) fn should_render_artifact_shell(&self, block: &SourceBlock) -> bool {
        self.block_render_provider.is_some() && self.block_render_request(block).is_some()
    }

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
            let request = self.render_request(MarkdownBlockRenderKind::Math, source.clone());
            self.enqueue_render(request, RenderWaiter::InlineMath { source }, cx);
        }
    }

    pub(super) fn request_block_renders(&mut self, range: Range<usize>, cx: &mut Context<Self>) {
        if self.block_render_provider.is_none() {
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
            .collect::<Vec<_>>();
        for (block_id, source, request) in requests {
            self.enqueue_block_render(block_id, source, request, cx);
        }
    }

    pub(in crate::editor) fn block_render_request(
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

    pub(in crate::editor) fn enqueue_block_render(
        &mut self,
        block_id: SourceNodeId,
        source: String,
        request: MarkdownBlockRenderRequest,
        cx: &mut Context<Self>,
    ) {
        self.pending_block_renders.insert(block_id, source.clone());
        self.enqueue_render(request, RenderWaiter::Block { block_id, source }, cx);
    }

    fn enqueue_render(
        &mut self,
        request: MarkdownBlockRenderRequest,
        waiter: RenderWaiter,
        cx: &mut Context<Self>,
    ) {
        let key = RenderCacheKey::from_request(&request);
        if let Some(cached) = self.block_render_cache.get(&key).cloned() {
            self.apply_cached_render(waiter, cached);
            return;
        }
        self.mark_waiter_pending(&waiter);
        if let Some(waiters) = self.pending_shared_renders.get_mut(&key) {
            waiters.push(waiter);
            return;
        }
        if self.pending_shared_renders.len() >= MAX_CONCURRENT_BLOCK_RENDERS {
            self.clear_waiter_pending(&waiter);
            return;
        }
        let Some(provider) = self.block_render_provider.clone() else {
            self.clear_waiter_pending(&waiter);
            return;
        };
        self.pending_shared_renders
            .insert(key.clone(), vec![waiter]);
        let generation = self.block_render_generation;
        let weak = cx.entity().downgrade();
        let task = cx.background_spawn(async move { provider(request).await });
        cx.spawn(async move |_, cx| {
            let result = task.await;
            let _ = weak.update(cx, |editor, cx| {
                editor.finish_shared_render(key, generation, result);
                editor.refresh_projection_highlights(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn mark_waiter_pending(&mut self, waiter: &RenderWaiter) {
        if let RenderWaiter::InlineMath { source } = waiter {
            self.pending_inline_math_renders.insert(source.clone());
        }
    }

    fn clear_waiter_pending(&mut self, waiter: &RenderWaiter) {
        match waiter {
            RenderWaiter::Block { block_id, .. } => {
                self.pending_block_renders.remove(block_id);
            }
            RenderWaiter::InlineMath { source } => {
                self.pending_inline_math_renders.remove(source);
            }
        }
    }

    fn finish_shared_render(
        &mut self,
        key: RenderCacheKey,
        generation: u64,
        result: Result<Option<MarkdownBlockRenderArtifact>, String>,
    ) {
        if self.block_render_generation != generation {
            return;
        }
        let Some(waiters) = self.pending_shared_renders.remove(&key) else {
            return;
        };
        let cached = match result {
            Ok(Some(artifact)) if artifact.media_type == "image/svg+xml" => {
                CachedRender::Artifact(artifact)
            }
            Ok(Some(artifact)) => {
                CachedRender::Error(format!("不支持的渲染格式：{}", artifact.media_type))
            }
            Ok(None) => CachedRender::Error("渲染器当前不可用".to_owned()),
            Err(error) => CachedRender::Error(error),
        };
        self.block_render_cache.insert(key, cached.clone());
        for waiter in waiters {
            self.apply_cached_render(waiter, cached.clone());
        }
    }

    fn apply_cached_render(&mut self, waiter: RenderWaiter, cached: CachedRender) {
        self.clear_waiter_pending(&waiter);
        match waiter {
            RenderWaiter::InlineMath { source } => match cached {
                CachedRender::Artifact(artifact) => {
                    self.failed_inline_math_renders.remove(&source);
                    self.inline_math_artifacts.insert(source, artifact);
                }
                CachedRender::Error(_) => {
                    self.failed_inline_math_renders.insert(source);
                }
            },
            RenderWaiter::Block { block_id, source } => {
                if self.block_source(block_id).as_deref() != Some(&source) {
                    return;
                }
                self.block_render_sources.insert(block_id, source);
                self.block_render_artifacts.remove(&block_id);
                self.block_render_errors.remove(&block_id);
                match cached {
                    CachedRender::Artifact(artifact) => {
                        self.block_render_artifacts.insert(block_id, artifact);
                    }
                    CachedRender::Error(error) => {
                        self.block_render_errors.insert(block_id, error);
                    }
                }
            }
        }
    }

    fn block_source(&self, block_id: SourceNodeId) -> Option<String> {
        let block = self.history.document().block_by_id(block_id)?;
        let range = block.content_range.as_ref()?;
        Some(self.history.document().source[range.clone()].to_owned())
    }

    pub(super) fn render_block_output(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> Option<gpui::AnyElement> {
        let source = self.block_source(block.id)?;
        if self.block_render_sources.get(&block.id) != Some(&source) {
            return None;
        }
        if let Some(error) = self.block_render_errors.get(&block.id) {
            return Some(self.render_block_error(block, &source, error, cx));
        }
        self.render_block_artifact(block)
    }

    pub(super) fn render_block_placeholder(&self, block: &SourceBlock) -> gpui::AnyElement {
        let block_id = block.id;
        let height = render_surface_reserved_height(block).unwrap_or(240.);
        let label = if matches!(block.kind, SourceBlockKind::MathBlock { .. }) {
            "正在渲染公式…"
        } else {
            "正在渲染图表…"
        };
        gpui::div()
            .id(("markdown-render-placeholder", block_id.0))
            .debug_selector(move || format!("markdown-render-placeholder-{}", block_id.0))
            .w_full()
            .h(px(height))
            .min_h(px(height))
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.border.opacity(0.06))
            .flex()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(self.theme.muted_foreground)
            .child(label)
            .into_any_element()
    }

    fn render_block_artifact(&self, block: &SourceBlock) -> Option<gpui::AnyElement> {
        let block_id = block.id;
        let artifact = self.block_render_artifacts.get(&block_id)?;
        (artifact.media_type == "image/svg+xml").then(|| {
            let image = Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone()));
            let reserved_height = render_surface_reserved_height(block).unwrap_or(64.);
            let height = artifact
                .intrinsic_height
                .unwrap_or(240.)
                .clamp(64., 520.)
                .max(reserved_height);
            let image_bounds_id = block_id;
            let image_canvas = canvas(
                move |_, window, cx| image.use_render_image(window, cx),
                move |bounds, image, window, _| {
                    let Some(image) = image else {
                        return;
                    };
                    let image_bounds = ObjectFit::Contain.get_bounds(bounds, image.size(0));
                    let _ = window.paint_image(image_bounds, Corners::default(), image, 0, false);
                },
            )
            .size_full();
            gpui::div()
                .id(("markdown-rendered-block", block_id.0))
                .debug_selector(|| format!("markdown-rendered-block-{}", block_id.0))
                .w_full()
                .min_h(px(reserved_height))
                .h(px(height))
                .p_3()
                .rounded_md()
                .border_1()
                .border_color(self.theme.border)
                .bg(self.theme.background)
                .child(
                    gpui::div()
                        .id(("markdown-rendered-image-bounds", block_id.0))
                        .debug_selector(move || {
                            format!("markdown-rendered-image-bounds-{}", image_bounds_id.0)
                        })
                        .size_full()
                        .overflow_hidden()
                        .child(image_canvas),
                )
                .into_any_element()
        })
    }

    fn render_block_error(
        &self,
        block: &SourceBlock,
        source: &str,
        error: &str,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let block_id = block.id;
        let reserved_height = render_surface_reserved_height(block).unwrap_or(96.);
        let editor = cx.entity();
        gpui::div()
            .id(("markdown-render-error", block_id.0))
            .debug_selector(|| format!("markdown-render-error-{}", block_id.0))
            .w_full()
            .min_h(px(reserved_height.max(96.)))
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.border.opacity(0.08))
            .flex()
            .flex_col()
            .gap_2()
            .text_sm()
            .text_color(self.theme.muted_foreground)
            .child(format!("预览渲染失败：{error}"))
            .child(
                gpui::div()
                    .p_2()
                    .rounded_sm()
                    .bg(self.theme.background)
                    .font_family("monospace")
                    .text_color(self.theme.foreground)
                    .child(source.to_owned()),
            )
            .child(
                gpui::div()
                    .on_mouse_down(gpui::MouseButton::Left, |_, _, cx| {
                        // The rendered block itself opens source editing on mouse down.
                        // Keep the retry control independent so its click can complete.
                        cx.stop_propagation();
                    })
                    .child(
                        Button::new(("markdown-render-retry", block_id.0))
                            .debug_selector(|| format!("markdown-render-retry-{}", block_id.0))
                            .label("重试渲染")
                            .small()
                            .on_click(move |_, _, cx| {
                                editor.update(cx, |editor, cx| {
                                    editor.retry_block_render(block_id, cx);
                                });
                            }),
                    ),
            )
            .into_any_element()
    }
}

fn color_key(color: gpui::Hsla) -> [u32; 4] {
    [
        color.h.to_bits(),
        color.s.to_bits(),
        color.l.to_bits(),
        color.a.to_bits(),
    ]
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
