use super::MarkdownEditor;
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, SharedString, Styled,
    prelude::FluentBuilder, rems,
};
use gpui_component::{
    Sizable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, LocalInputStyle},
    scroll::ScrollableElement,
    text::{MarkdownPalette, TextView, TextViewStyle},
    v_flex,
};
use markdown_source::{SourceBlock, SourceBlockKind};

mod action_handlers;
mod table;
mod toolbar;

impl MarkdownEditor {
    fn render_blocks(&self, cx: &mut Context<Self>) -> impl gpui::IntoElement {
        let active_block = self.active_block;
        let blocks = self.history.document().blocks.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .min_w_0()
            .overflow_y_scrollbar()
            .px_4()
            .py_3()
            .when(blocks.is_empty(), |editor| {
                editor.child(self.render_empty_document())
            })
            .children(blocks.into_iter().map(|block| match &block.kind {
                SourceBlockKind::Table(table) => self.render_table(&block, table, cx),
                _ if active_block == Some(block.id) => self.render_active_block(&block, cx),
                _ => self.render_preview_block(&block, cx),
            }))
    }

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
                    .caret_color(self.theme.primary),
            )
            .into_any_element()
    }

    fn render_active_block(&self, block: &SourceBlock, cx: &mut Context<Self>) -> gpui::AnyElement {
        let rows = block.original_source.lines().count().max(1) as f32;
        let heading_level = match block.kind {
            SourceBlockKind::Heading { level, .. } => Some(level),
            _ => None,
        };
        v_flex()
            .id(("markdown-active-block", block.id.0))
            .debug_selector(|| format!("markdown-active-block-{}", block.id.0))
            .w_full()
            .min_w_0()
            .child(
                gpui::div()
                    .w_full()
                    .h(rems(rows.mul_add(1.5, heading_height_extra(heading_level))))
                    .when_some(heading_level, apply_heading_style)
                    .child(
                        Input::new(&self.input)
                            .size_full()
                            .bare()
                            .bordered(false)
                            .focus_bordered(false)
                            .local_style(self.input_style())
                            .highlight_theme(self.theme.highlight_theme.clone())
                            .caret_color(self.theme.primary)
                            .indent_guide_color(self.theme.border),
                    ),
            )
            .child(self.render_block_toolbar(block, cx))
            .into_any_element()
    }

    fn render_preview_block(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let block_id = block.id;
        gpui::div()
            .id(("markdown-preview-block", block.id.0))
            .debug_selector(|| format!("markdown-preview-block-{}", block.id.0))
            .w_full()
            .min_w_0()
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                editor.update(cx, |editor, cx| {
                    editor.activate_block(block_id, window, cx);
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
            .into_any_element(),
            SourceBlockKind::FrontMatter | SourceBlockKind::RawMarkdown => self.raw_card(block),
            _ => TextView::markdown(
                SharedString::from(format!("markdown-block-{}", block.id.0)),
                block.original_source.clone(),
            )
            .style(self.text_view_style())
            .into_any_element(),
        }
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
        TextViewStyle::default().markdown_palette(MarkdownPalette {
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

fn heading_height_extra(level: Option<u8>) -> f32 {
    level.map_or(0.75, |level| match level {
        1 => 1.8,
        2 => 1.5,
        3 => 1.25,
        _ => 1.,
    })
}

fn apply_heading_style(element: gpui::Div, level: u8) -> gpui::Div {
    match level {
        1 => element.text_2xl().font_bold(),
        2 => element.text_xl().font_bold(),
        3 => element.text_lg().font_semibold(),
        _ => element.font_semibold(),
    }
}
