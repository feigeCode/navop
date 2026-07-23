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
            .relative()
            .child(self.render_active_placeholder(block))
            .child(
                gpui::div()
                    .absolute()
                    .top_0()
                    .right_0()
                    .bottom_0()
                    .left_0()
                    .child(self.render_active_block(block, None)),
            )
            .into_any_element()
    }

    fn render_active_placeholder(&self, block: &SourceBlock) -> gpui::AnyElement {
        gpui::div()
            .id(("markdown-active-placeholder", block.id.0))
            .debug_selector(|| format!("markdown-active-placeholder-{}", block.id.0))
            .w_full()
            .opacity(0.)
            .child(self.render_preview_content(block))
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
        if let Some(rendered) = self.render_block_output(block) {
            return self.render_artifact_preview(block, rendered, cx);
        }
        self.render_preview_block(block, cx)
    }

    pub(super) fn render_preview_content(&self, block: &SourceBlock) -> gpui::AnyElement {
        self.render_block_output(block)
            .unwrap_or_else(|| self.preview_content(block))
    }
}
