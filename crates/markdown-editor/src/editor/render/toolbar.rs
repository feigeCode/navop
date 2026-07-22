use super::MarkdownEditor;
use gpui::{Context, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
};
use markdown_source::{BlockMoveDirection, SourceBlock};

impl MarkdownEditor {
    pub(super) fn render_block_toolbar(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let down_editor = editor.clone();
        let quote_editor = editor.clone();
        let code_editor = editor.clone();
        let delete_editor = editor.clone();
        h_flex()
            .w_full()
            .justify_end()
            .gap_1()
            .child(
                block_button(block, "up", "Up").on_click(move |_, window, cx| {
                    editor.update(cx, |editor, cx| {
                        let _ = editor.move_active_block(BlockMoveDirection::Up, window, cx);
                    });
                }),
            )
            .child(
                block_button(block, "down", "Down").on_click(move |_, window, cx| {
                    down_editor.update(cx, |editor, cx| {
                        let _ = editor.move_active_block(BlockMoveDirection::Down, window, cx);
                    });
                }),
            )
            .child(
                block_button(block, "quote", "Quote").on_click(move |_, window, cx| {
                    quote_editor.update(cx, |editor, cx| {
                        let _ = editor.toggle_active_blockquote(window, cx);
                    });
                }),
            )
            .child(
                block_button(block, "code", "Code").on_click(move |_, window, cx| {
                    code_editor.update(cx, |editor, cx| {
                        let _ = editor.toggle_active_code_fence(None, window, cx);
                    });
                }),
            )
            .child(
                block_button(block, "delete", "Delete").on_click(move |_, window, cx| {
                    delete_editor.update(cx, |editor, cx| {
                        let _ = editor.delete_active_block(window, cx);
                    });
                }),
            )
            .into_any_element()
    }
}

fn block_button(block: &SourceBlock, action: &str, label: &str) -> Button {
    Button::new(SharedString::from(format!(
        "markdown-block-{action}-{}",
        block.id.0
    )))
    .label(label.to_owned())
    .xsmall()
    .ghost()
}
