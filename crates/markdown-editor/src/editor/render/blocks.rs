use super::{MarkdownEditor, VIRTUALIZATION_THRESHOLD};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled, px,
};
use gpui_component::{
    scroll::{Scrollbar, ScrollbarShow},
    v_flex, v_virtual_list,
};
use markdown_source::{SourceBlock, SourceBlockKind};
use std::rc::Rc;

impl MarkdownEditor {
    pub(super) fn render_editor_content(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        self.render_blocks(cx)
    }

    fn render_blocks(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.history.document().blocks.is_empty() {
            return v_flex()
                .size_full()
                .min_h_0()
                .min_w_0()
                .px_4()
                .py_3()
                .child(self.render_empty_document())
                .into_any_element();
        }
        if self.history.document().blocks.len() < VIRTUALIZATION_THRESHOLD {
            return self.render_standard_blocks(cx);
        }
        self.render_virtual_blocks(cx)
    }

    fn render_virtual_blocks(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let active_block = self.active_block;
        let item_sizes = Rc::new(
            self.history
                .document()
                .blocks
                .iter()
                .map(block_size)
                .collect::<Vec<_>>(),
        );
        let blocks = v_virtual_list(
            cx.entity(),
            "markdown-block-list",
            item_sizes.clone(),
            move |editor, visible_range, _, cx| {
                visible_range
                    .map(|index| editor.render_block(index, &item_sizes, active_block, cx))
                    .collect()
            },
        )
        .size_full()
        .px_4()
        .py_3()
        .track_scroll(&self.block_scroll);
        self.render_viewport(
            blocks.into_any_element(),
            Scrollbar::vertical(&self.block_scroll)
                .scrollbar_show(ScrollbarShow::Always)
                .into_any_element(),
        )
    }

    fn render_standard_blocks(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let content = v_flex().w_full().min_w_0().px_4().py_3().children(
            self.history
                .document()
                .blocks
                .clone()
                .into_iter()
                .map(|block| match &block.kind {
                    SourceBlockKind::Table(table) => self.render_table(&block, table, cx),
                    _ if self.active_block == Some(block.id) => self.render_active_block(&block),
                    _ => self.render_preview_block(&block, cx),
                }),
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
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .w(px(16.))
                    .child(scrollbar),
            )
            .into_any_element()
    }

    fn render_block(
        &mut self,
        index: usize,
        item_sizes: &[gpui::Size<gpui::Pixels>],
        active_block: Option<markdown_source::SourceNodeId>,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(block) = self.history.document().blocks.get(index).cloned() else {
            return gpui::div().into_any_element();
        };
        gpui::div()
            .w_full()
            .h(item_sizes.get(index).map_or(px(40.), |size| size.height))
            .child(match &block.kind {
                SourceBlockKind::Table(table) => self.render_table(&block, table, cx),
                _ if active_block == Some(block.id) => self.render_active_block(&block),
                _ => self.render_preview_block(&block, cx),
            })
            .into_any_element()
    }
}

fn block_size(block: &SourceBlock) -> gpui::Size<gpui::Pixels> {
    let lines = block.original_source.lines().count().max(1) as f32;
    let height = match &block.kind {
        SourceBlockKind::Heading { level, .. } => match level {
            1 => 52.,
            2 => 46.,
            3 => 40.,
            _ => 36.,
        },
        SourceBlockKind::Table(table) => {
            table.rows.len().saturating_sub(1).max(1) as f32 * 42. + 16.
        }
        SourceBlockKind::CodeFence { .. }
        | SourceBlockKind::FrontMatter
        | SourceBlockKind::Html
        | SourceBlockKind::RawMarkdown => lines.mul_add(24., 20.),
        SourceBlockKind::OrderedList { .. }
        | SourceBlockKind::UnorderedList
        | SourceBlockKind::BlockQuote => lines.mul_add(30., 8.),
        _ => lines.mul_add(30., 8.),
    };
    gpui::size(px(0.), px(height))
}
