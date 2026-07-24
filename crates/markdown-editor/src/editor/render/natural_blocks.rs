use super::MarkdownEditor;
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled};
use markdown_source::{SourceBlock, SourceBlockKind};

impl MarkdownEditor {
    pub(super) fn render_standard_block(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let frame = gpui::div()
            .id(("markdown-block-frame", block.id.0))
            .debug_selector(|| format!("markdown-block-frame-{}", block.id.0))
            .w_full()
            .min_w_0();
        if self.active_block != Some(block.id) || matches!(block.kind, SourceBlockKind::Table(_)) {
            return frame
                .child(self.render_natural_block(block, cx))
                .into_any_element();
        }
        frame
            .child(self.render_active_block(block, cx))
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
        if let Some(rendered) = self.render_block_output(block, cx) {
            return self.render_artifact_preview(block, rendered, cx);
        }
        self.render_preview_block(block, cx)
    }
}
