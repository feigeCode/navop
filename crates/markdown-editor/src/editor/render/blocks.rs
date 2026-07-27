use super::{
    MarkdownEditor,
    layout_metrics::{
        DOCUMENT_BOTTOM_PADDING, DOCUMENT_MAX_WIDTH, DOCUMENT_SIDE_PADDING, DOCUMENT_TOP_PADDING,
        should_virtualize, virtual_item_sizes,
    },
};
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ElementExt,
    scroll::{Scrollbar, ScrollbarShow},
    v_flex, v_virtual_list,
};
use markdown_source::SourceBlock;
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
        let mut item_sizes = virtual_item_sizes(
            &self.history.document().blocks,
            &self.measured_block_heights,
        );
        if let Some((index, _)) = self.active_empty_gap_placement()
            && let Some(size) = item_sizes.get_mut(index)
        {
            size.height += px(super::MARKDOWN_BODY_LINE_HEIGHT);
        }
        let item_sizes = Rc::new(item_sizes);
        let blocks = v_virtual_list(
            cx.entity(),
            "markdown-block-list",
            item_sizes.clone(),
            move |editor, visible_range, _, cx| {
                editor.request_block_renders(visible_range.clone(), cx);
                editor.request_inline_math_renders(visible_range.clone(), cx);
                visible_range
                    .map(|index| editor.render_block(index, &item_sizes, cx))
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
                .enumerate()
                .map(|(index, block)| self.render_standard_block(index, &block, cx)),
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
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(block) = self.history.document().blocks.get(index).cloned() else {
            return gpui::div().into_any_element();
        };
        let block_count = self.history.document().blocks.len();
        let empty_gap = self.active_empty_gap_placement();
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
                    .when(empty_gap == Some((index, true)), |this| {
                        this.child(self.render_empty_gap_surface())
                    })
                    .child(self.render_natural_block(&block, cx))
                    .when(empty_gap == Some((index, false)), |this| {
                        this.child(self.render_empty_gap_surface())
                    }),
            )
            .into_any_element()
    }

    /// Places the transient empty Input next to its neighboring parsed block.
    ///
    /// Markdown parsers intentionally do not create an AST paragraph for an
    /// empty line. Keeping that line inside an adjacent virtual item lets the
    /// user continue typing without inventing placeholder source characters or
    /// replacing the document's scrolling implementation.
    pub(super) fn active_empty_gap_placement(&self) -> Option<(usize, bool)> {
        if self.active_surface_key() != MarkdownSurfaceKey::Empty
            || self.active_block.is_some()
            || self.history.document().blocks.is_empty()
        {
            return None;
        }
        let range = &self
            .surface(MarkdownSurfaceKey::Empty)?
            .projection
            .source_range;
        if !range.is_empty() {
            return None;
        }
        let blocks = &self.history.document().blocks;
        let insertion_index = blocks.partition_point(|block| block.source_range.end <= range.start);
        if insertion_index < blocks.len() {
            Some((insertion_index, true))
        } else {
            Some((blocks.len() - 1, false))
        }
    }

    fn rendered_block_height(
        &self,
        block: &SourceBlock,
        preview_height: gpui::Pixels,
    ) -> gpui::Pixels {
        self.measured_block_heights
            .get(&block.id)
            .copied()
            .map_or(preview_height, |measured| preview_height.max(measured))
    }

    /// Keeps the rendered artifact and its source Input in one permanent grid
    /// cell. Both layers participate in layout from the first render, so
    /// activating the source can only change focus/visibility, never the
    /// block's reserved geometry or InputState identity.
    pub(super) fn render_artifact_shell(
        &self,
        block: &SourceBlock,
        rendered: gpui::AnyElement,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = MarkdownSurfaceKey::block(block.id);
        let active = self.active_block == Some(block.id) && self.active_surface_key() == key;
        let editor = cx.entity();
        let click_editor = editor.clone();
        let block_id = block.id;
        let language_header = self.render_code_language_header(block, cx);
        let input_layer = gpui::div()
            .id(("markdown-artifact-input-layer", block.id.0))
            .debug_selector(move || format!("markdown-artifact-input-layer-{}", block_id.0))
            .col_start(1)
            .row_start(1)
            .w_full()
            .min_w_0()
            .opacity(if active { 1. } else { 0. })
            .child(self.render_block_edit_surface(block, false, cx));
        let rendered_layer = gpui::div()
            .id(("markdown-artifact-rendered-layer", block.id.0))
            .debug_selector(move || format!("markdown-artifact-rendered-layer-{}", block_id.0))
            .col_start(1)
            .row_start(1)
            .w_full()
            .min_w_0()
            .cursor_text()
            .when(active, |this| this.invisible())
            .on_mouse_down(gpui::MouseButton::Left, move |event, window, cx| {
                click_editor.update(cx, |editor, cx| {
                    if event.click_count == 1 && !event.modifiers.shift {
                        editor.activate_surface_at_position(key, event.position, window, cx);
                    } else {
                        editor.focus_surface(key, window, cx);
                    }
                });
            })
            .child(rendered);
        let layers = gpui::div()
            .grid()
            .grid_cols(1)
            .w_full()
            .min_w_0()
            .child(input_layer)
            .child(rendered_layer);
        v_flex()
            .id(("markdown-artifact-shell", block.id.0))
            .debug_selector(move || format!("markdown-artifact-shell-{}", block_id.0))
            .w_full()
            .min_w_0()
            .on_prepaint(move |bounds, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor.record_measured_block_height(block_id, bounds.size.height, cx);
                });
            })
            .children(language_header)
            .child(layers)
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
