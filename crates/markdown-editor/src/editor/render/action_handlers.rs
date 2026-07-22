use super::MarkdownEditor;
use crate::actions::*;
use gpui::{
    Context, InteractiveElement, ParentElement, Render, Styled, Window, prelude::FluentBuilder,
};
use gpui_component::v_flex;

impl Render for MarkdownEditor {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let content = self.render_editor_content(cx);
        Self::bind_editor_actions(gpui::div().key_context(EDITOR_CONTEXT), cx)
            .size_full()
            .min_h_0()
            .min_w_0()
            .bg(self.theme.background)
            .child(
                v_flex()
                    .size_full()
                    .min_h_0()
                    .min_w_0()
                    .child(content)
                    .when(self.active_image_properties().is_some(), |editor| {
                        editor.child(self.render_image_properties(cx))
                    }),
            )
    }
}

impl MarkdownEditor {
    fn bind_editor_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        let element = Self::bind_history_actions(element, cx);
        let element = Self::bind_inline_actions(element, cx);
        let element = Self::bind_heading_actions(element, cx);
        let element = Self::bind_block_actions(element, cx);
        Self::bind_image_actions(element, cx)
    }

    fn bind_history_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(cx.listener(|editor, _: &UndoSourceEdit, window, cx| {
                let _ = editor.undo(window, cx);
            }))
            .on_action(cx.listener(|editor, _: &RedoSourceEdit, window, cx| {
                let _ = editor.redo(window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SelectAll, window, cx| {
                editor.select_all(window, cx);
            }))
    }

    fn bind_inline_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(cx.listener(|editor, _: &ToggleBold, window, cx| {
                let _ =
                    editor.toggle_inline_format(markdown_source::InlineFormat::Bold, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleItalic, window, cx| {
                let _ =
                    editor.toggle_inline_format(markdown_source::InlineFormat::Italic, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleUnderline, window, cx| {
                let _ = editor.toggle_inline_format(
                    markdown_source::InlineFormat::Underline,
                    window,
                    cx,
                );
            }))
            .on_action(cx.listener(|editor, _: &ToggleStrike, window, cx| {
                let _ =
                    editor.toggle_inline_format(markdown_source::InlineFormat::Strike, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleInlineCode, window, cx| {
                let _ =
                    editor.toggle_inline_format(markdown_source::InlineFormat::Code, window, cx);
            }))
    }

    fn bind_heading_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(cx.listener(|editor, _: &SetParagraph, window, cx| {
                let _ = editor.set_active_heading(None, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading1, window, cx| {
                let _ = editor.set_active_heading(Some(1), window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading2, window, cx| {
                let _ = editor.set_active_heading(Some(2), window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading3, window, cx| {
                let _ = editor.set_active_heading(Some(3), window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading4, window, cx| {
                let _ = editor.set_active_heading(Some(4), window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading5, window, cx| {
                let _ = editor.set_active_heading(Some(5), window, cx);
            }))
            .on_action(cx.listener(|editor, _: &SetHeading6, window, cx| {
                let _ = editor.set_active_heading(Some(6), window, cx);
            }))
    }

    fn bind_block_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        let element = element
            .on_action(cx.listener(|editor, _: &ToggleBulletList, window, cx| {
                let _ = editor.toggle_active_list(markdown_source::ListFormat::Bullet, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleOrderedList, window, cx| {
                let _ = editor.toggle_active_list(markdown_source::ListFormat::Ordered, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleTaskList, window, cx| {
                let _ = editor.toggle_active_list(markdown_source::ListFormat::Task, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleQuote, window, cx| {
                let _ = editor.toggle_active_blockquote(window, cx);
            }))
            .on_action(cx.listener(|editor, _: &ToggleCodeBlock, window, cx| {
                let _ = editor.toggle_active_code_fence(None, window, cx);
            }));
        Self::bind_block_management_actions(element, cx)
    }

    fn bind_block_management_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(cx.listener(|editor, _: &MoveBlockUp, window, cx| {
                let _ =
                    editor.move_active_block(markdown_source::BlockMoveDirection::Up, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &MoveBlockDown, window, cx| {
                let _ =
                    editor.move_active_block(markdown_source::BlockMoveDirection::Down, window, cx);
            }))
            .on_action(cx.listener(|editor, _: &DuplicateBlock, window, cx| {
                let _ = editor.duplicate_active_block(window, cx);
            }))
            .on_action(cx.listener(|editor, _: &DeleteBlock, window, cx| {
                let _ = editor.delete_active_block(window, cx);
            }))
    }

    fn bind_image_actions(element: gpui::Div, cx: &mut Context<Self>) -> gpui::Div {
        element
            .on_action(
                cx.listener(|editor, _: &DeleteActiveImageBackward, window, cx| {
                    if !editor.delete_active_image(window, cx) {
                        cx.propagate();
                    }
                }),
            )
            .on_action(
                cx.listener(|editor, _: &DeleteActiveImageForward, window, cx| {
                    if !editor.delete_active_image(window, cx) {
                        cx.propagate();
                    }
                }),
            )
    }
}
