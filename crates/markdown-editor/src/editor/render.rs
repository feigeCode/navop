use super::MarkdownEditor;
use gpui::{Context, InteractiveElement, IntoElement, ParentElement, Styled, rems};
use gpui_component::{
    Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle},
};

mod action_handlers;
mod active_block;
mod active_inline_math;
mod active_list_markers;
pub(super) mod block_renderer;
mod blocks;
pub(super) mod layout_metrics;
mod list_marker_source;
mod natural_blocks;
mod table;
mod table_toolbar;

pub(super) const MARKDOWN_BODY_FONT_SIZE: f32 = 16.;
pub(super) const MARKDOWN_BODY_LINE_HEIGHT: f32 = 24.;

impl MarkdownEditor {
    fn render_empty_document(&self) -> gpui::AnyElement {
        gpui::div()
            .id("markdown-empty-document")
            .debug_selector(|| "markdown-empty-document".to_owned())
            .w_full()
            .min_h(rems(4.))
            .child(
                Input::new(&self.input)
                    .size_full()
                    .bare()
                    .bordered(false)
                    .focus_bordered(false)
                    .local_style(self.input_style())
                    .editor_scrollbar(false)
                    .text_layout_margin(false)
                    .caret_color(self.theme.primary),
            )
            .into_any_element()
    }

    fn input_style(&self) -> LocalInputStyle {
        LocalInputStyle {
            background: self.theme.background,
            foreground: self.theme.foreground,
            muted_foreground: self.theme.muted_foreground,
            border: self.theme.border,
        }
    }

    fn render_image_properties(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        let editor = cx.entity();
        let delete_editor = editor.clone();
        h_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_2()
            .border_t_1()
            .border_color(self.theme.border)
            .bg(self.theme.background)
            .child(
                gpui::div()
                    .w(rems(14.))
                    .child(Input::new(&self.image_alt_input).local_style(self.input_style())),
            )
            .child(
                gpui::div().flex_1().min_w_0().child(
                    Input::new(&self.image_destination_input).local_style(self.input_style()),
                ),
            )
            .child(
                Button::new("markdown-image-save")
                    .label("Save")
                    .small()
                    .on_click(move |_, window, cx| {
                        editor.update(cx, |editor, cx| {
                            let _ = editor.save_active_image_properties(window, cx);
                        });
                    }),
            )
            .child(
                Button::new("markdown-image-delete")
                    .label("Delete")
                    .small()
                    .ghost()
                    .on_click(move |_, window, cx| {
                        delete_editor.update(cx, |editor, cx| {
                            editor.delete_active_image(window, cx);
                        });
                    }),
            )
            .into_any_element()
    }
}
