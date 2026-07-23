use super::{
    MarkdownEditor,
    active_block::active_block_height,
    layout_metrics::{
        DOCUMENT_BOTTOM_PADDING, DOCUMENT_MAX_WIDTH, DOCUMENT_SIDE_PADDING, DOCUMENT_TOP_PADDING,
        block_size, should_virtualize, virtual_item_sizes,
    },
};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ElementExt,
    scroll::{Scrollbar, ScrollbarShow},
    v_flex, v_virtual_list,
};
use markdown_source::{SourceBlock, SourceBlockKind};
use std::cell::Cell;
use std::rc::Rc;

impl MarkdownEditor {
    pub(super) fn render_editor_content(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.render_blocks(cx)
    }

    fn render_blocks(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.history.document().blocks.is_empty() {
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .child(document_column().child(self.render_empty_document()))
                .into_any_element();
        }
        if should_virtualize(&self.history.document().blocks) {
            return self.render_virtual_blocks(cx);
        }
        self.render_standard_blocks(cx)
    }

    fn render_virtual_blocks(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active_block = self.active_block;
        let item_sizes = Rc::new(virtual_item_sizes(
            &self.history.document().blocks,
            &self.measured_block_heights,
        ));
        let blocks = v_virtual_list(
            cx.entity(),
            "markdown-block-list",
            item_sizes.clone(),
            move |editor, visible_range, _, cx| {
                editor.request_block_renders(visible_range.clone(), cx);
                editor.request_inline_math_renders(visible_range.clone(), cx);
                visible_range
                    .map(|index| editor.render_block(index, &item_sizes, active_block, cx))
                    .collect()
            },
        )
        .size_full()
        .track_scroll(&self.block_scroll);
        self.render_viewport(
            blocks.into_any_element(),
            Scrollbar::vertical(&self.block_scroll)
                .scrollbar_show(ScrollbarShow::Always)
                .colors(
                    self.theme.muted_foreground.opacity(0.46),
                    self.theme.foreground.opacity(0.68),
                    self.theme.border.opacity(0.1),
                )
                .into_any_element(),
        )
    }

    fn render_standard_blocks(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let block_count = self.history.document().blocks.len();
        self.request_block_renders(0..block_count, cx);
        self.request_inline_math_renders(0..block_count, cx);
        let blocks = self.history.document().blocks.clone();
        let content = document_column().children(
            blocks
                .into_iter()
                .map(|block| self.render_standard_block(&block, cx)),
        );
        let blocks = gpui::div()
            .id("markdown-standard-block-list")
            .size_full()
            .overflow_y_scroll()
            .track_scroll(&self.document_scroll)
            .child(content);
        self.render_viewport(
            blocks.into_any_element(),
            Scrollbar::vertical(&self.document_scroll)
                .scrollbar_show(ScrollbarShow::Always)
                .colors(
                    self.theme.muted_foreground.opacity(0.46),
                    self.theme.foreground.opacity(0.68),
                    self.theme.border.opacity(0.1),
                )
                .into_any_element(),
        )
    }

    fn render_viewport(
        &self,
        blocks: gpui::AnyElement,
        scrollbar: gpui::AnyElement,
    ) -> gpui::AnyElement {
        gpui::div()
            .id("markdown-block-viewport")
            .size_full()
            .min_h_0()
            .min_w_0()
            .relative()
            .overflow_hidden()
            .child(blocks)
            .child(
                gpui::div()
                    .id("markdown-editor-scrollbar")
                    .debug_selector(|| "markdown-editor-scrollbar".to_owned())
                    .absolute()
                    .top(px(8.))
                    .right(px(4.))
                    .bottom(px(8.))
                    .w(px(16.))
                    .rounded_full()
                    .child(scrollbar),
            )
            .into_any_element()
    }

    fn render_block(
        &self,
        index: usize,
        item_sizes: &[gpui::Size<gpui::Pixels>],
        active_block: Option<markdown_source::SourceNodeId>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(block) = self.history.document().blocks.get(index).cloned() else {
            return gpui::div().into_any_element();
        };
        let block_count = self.history.document().blocks.len();
        let content_height = self.rendered_block_height(&block, block_size(&block).height);
        gpui::div()
            .id(("markdown-block-frame", block.id.0))
            .debug_selector(move || format!("markdown-block-frame-{}", block.id.0))
            .w_full()
            .h(self.rendered_block_height(
                &block,
                item_sizes.get(index).map_or(px(40.), |size| size.height),
            ))
            .child(
                document_column()
                    .py_0()
                    .when(index == 0, |this| this.pt(px(DOCUMENT_TOP_PADDING)))
                    .when(index + 1 == block_count, |this| {
                        this.pb(px(DOCUMENT_BOTTOM_PADDING))
                    })
                    .child(self.render_block_content(&block, active_block, content_height, cx)),
            )
            .into_any_element()
    }

    fn rendered_block_height(
        &self,
        block: &SourceBlock,
        preview_height: gpui::Pixels,
    ) -> gpui::Pixels {
        if self.active_block != Some(block.id) {
            return preview_height;
        }
        let rows = self.projection.text.lines().count().max(1) as f32;
        let heading = match block.kind {
            SourceBlockKind::Heading { level, .. } => Some(level),
            _ => None,
        };
        let source_code = matches!(
            block.kind,
            SourceBlockKind::CodeFence { .. } | SourceBlockKind::MathBlock { .. }
        );
        let active = px(active_block_height(rows, heading, source_code) * 16.);
        preview_height.max(active)
    }

    fn render_block_content(
        &self,
        block: &SourceBlock,
        active_block: Option<markdown_source::SourceNodeId>,
        reserved_height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if active_block != Some(block.id)
            && let Some(rendered) = self.render_block_output(block)
        {
            return self.render_artifact_preview(block, rendered, cx);
        }
        match &block.kind {
            SourceBlockKind::Table(table) => self.render_table(block, table, cx),
            _ if active_block == Some(block.id) => {
                self.render_active_block(block, Some(reserved_height))
            }
            _ => self.render_preview_block(block, cx),
        }
    }

    pub(super) fn render_artifact_preview(
        &self,
        block: &SourceBlock,
        rendered: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let block_id = block.id;
        let bounds = Rc::new(Cell::new(gpui::Bounds::default()));
        let click_bounds = bounds.clone();
        gpui::div()
            .id(("markdown-preview-block", block.id.0))
            .debug_selector(|| format!("markdown-preview-block-{}", block.id.0))
            .w_full()
            .min_w_0()
            .cursor_text()
            .on_prepaint(move |value, _, _| bounds.set(value))
            .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                let line = ((event.position.y - click_bounds.get().top())
                    .as_f32()
                    .max(0.)
                    / 24.)
                    .floor() as usize;
                editor.update(cx, |editor, cx| {
                    editor.activate_block_line(block_id, line, window, cx);
                });
            })
            .child(rendered)
            .into_any_element()
    }

    pub(super) fn record_measured_block_height(
        &mut self,
        block_id: markdown_source::SourceNodeId,
        height: gpui::Pixels,
        cx: &mut Context<Self>,
    ) {
        if !self.uses_virtual_layout() || self.history.document().block_by_id(block_id).is_none() {
            return;
        }
        let content = height.max(px(1.));
        if self
            .measured_block_heights
            .get(&block_id)
            .is_none_or(|old| (*old - content).abs() > px(1.))
        {
            self.measured_block_heights.insert(block_id, content);
            cx.notify();
        }
    }
}

fn document_column() -> gpui::Div {
    v_flex()
        .debug_selector(|| "markdown-document-column".to_owned())
        .w_full()
        .min_w_0()
        .max_w(px(DOCUMENT_MAX_WIDTH))
        .mx_auto()
        .px(px(DOCUMENT_SIDE_PADDING))
        .pt(px(DOCUMENT_TOP_PADDING))
        .pb(px(DOCUMENT_BOTTOM_PADDING))
}
