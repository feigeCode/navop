use super::MarkdownEditor;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, prelude::FluentBuilder as _,
};
use markdown_source::{SourceBlock, SourceBlockKind};

impl MarkdownEditor {
    pub(super) fn render_standard_block(
        &self,
        index: usize,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let empty_gap = self.active_empty_gap_placement();
        let frame = gpui::div()
            .id(("markdown-block-frame", block.id.0))
            .debug_selector(|| format!("markdown-block-frame-{}", block.id.0))
            .flex()
            .flex_col()
            .w_full()
            .min_w_0();
        frame
            .when(empty_gap == Some((index, true)), |this| {
                this.child(self.render_empty_gap_surface())
            })
            .child(self.render_natural_block(block, cx))
            .when(empty_gap == Some((index, false)), |this| {
                this.child(self.render_empty_gap_surface())
            })
            .into_any_element()
    }

    pub(super) fn render_natural_block(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if let SourceBlockKind::Table(table) = &block.kind {
            return self.render_table(block, table, cx);
        }
        if matches!(block.kind, SourceBlockKind::Html) {
            return self.render_html_shell(block, cx);
        }
        if self.should_render_artifact_shell(block) {
            let rendered = self
                .render_block_output(block, cx)
                .unwrap_or_else(|| self.render_block_placeholder(block));
            return self.render_artifact_shell(block, rendered, cx);
        }
        self.render_block_edit_surface(block, true, cx)
    }
}
