use super::MarkdownEditor;
use gpui::{
    Context, Image, ImageFormat, InteractiveElement, IntoElement, MouseButton, ObjectFit,
    ParentElement, SharedString, Styled, StyledImage, img, rems,
};
use gpui_component::{
    ElementExt, Sizable,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle},
    text::{MarkdownPalette, TextView, TextViewStyle},
    v_flex,
};
use markdown_source::{SourceBlock, SourceBlockKind};
use std::{cell::Cell, rc::Rc, sync::Arc};

mod action_handlers;
mod active_block;
mod active_inline_math;
mod active_list_markers;
mod block_renderer;
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

    fn render_preview_block(
        &self,
        block: &SourceBlock,
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
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                let line = clicked_line(event.position.y, click_bounds.get().top());
                editor.update(cx, |editor, cx| {
                    editor.activate_block_line(block_id, line, window, cx);
                });
            })
            .child(self.preview_content(block))
            .into_any_element()
    }

    fn preview_content(&self, block: &SourceBlock) -> gpui::AnyElement {
        match block.kind {
            SourceBlockKind::Html => TextView::html(
                SharedString::from(format!("markdown-html-block-{}", block.id.0)),
                block.original_source.clone(),
            )
            .style(self.text_view_style())
            .text_size(gpui::px(MARKDOWN_BODY_FONT_SIZE))
            .line_height(gpui::px(MARKDOWN_BODY_LINE_HEIGHT))
            .into_any_element(),
            SourceBlockKind::FrontMatter | SourceBlockKind::RawMarkdown => self.raw_card(block),
            _ => self.markdown_preview(block),
        }
    }

    fn markdown_preview(&self, block: &SourceBlock) -> gpui::AnyElement {
        let math_artifacts = self.inline_math_artifacts.clone();
        TextView::markdown(
            SharedString::from(format!("markdown-block-{}", block.id.0)),
            block.original_source.clone(),
        )
        .style(self.text_view_style())
        .text_size(gpui::px(MARKDOWN_BODY_FONT_SIZE))
        .line_height(gpui::px(MARKDOWN_BODY_LINE_HEIGHT))
        .inline_math_renderer(move |source, _, _| {
            let Some(artifact) = math_artifacts.get(source) else {
                return gpui::div().child(source.to_owned()).into_any_element();
            };
            let image = Arc::new(Image::from_bytes(ImageFormat::Svg, artifact.bytes.clone()));
            let width = artifact.intrinsic_width.unwrap_or(96.).clamp(24., 360.);
            let height = artifact.intrinsic_height.unwrap_or(24.).clamp(16., 64.);
            img(image)
                .w(gpui::px(width))
                .h(gpui::px(height))
                .object_fit(ObjectFit::Contain)
                .into_any_element()
        })
        .into_any_element()
    }

    fn raw_card(&self, block: &SourceBlock) -> gpui::AnyElement {
        let label = match block.kind {
            SourceBlockKind::FrontMatter => "Frontmatter",
            _ => "Raw Markdown",
        };
        v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.border.opacity(0.08))
            .child(
                gpui::div()
                    .text_xs()
                    .text_color(self.theme.muted_foreground)
                    .child(label),
            )
            .children(block.original_source.lines().map(|line| {
                gpui::div()
                    .text_sm()
                    .text_color(self.theme.foreground)
                    .child(line.to_owned())
            }))
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

    fn text_view_style(&self) -> TextViewStyle {
        TextViewStyle::default()
            .paragraph_gap(rems(0.62))
            .heading_font_size(|level, _| match level {
                1 => gpui::px(30.),
                2 => gpui::px(24.),
                3 => gpui::px(20.),
                4 => gpui::px(17.),
                _ => gpui::px(16.),
            })
            .markdown_palette(MarkdownPalette {
                is_dark: self.theme.background.l < 0.5,
                foreground: self.theme.foreground,
                muted_foreground: self.theme.muted_foreground,
                border: self.theme.border,
                code_background: self.theme.border.opacity(0.2),
                code_foreground: self.theme.foreground,
                table_header: self.theme.border.opacity(0.24),
                table_row: self.theme.background,
                table_row_alt: self.theme.border.opacity(0.1),
                quote_border: self.theme.border,
                link: self.theme.primary,
            })
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

fn clicked_line(position: gpui::Pixels, top: gpui::Pixels) -> usize {
    ((position - top).as_f32().max(0.) / 24.).floor() as usize
}
