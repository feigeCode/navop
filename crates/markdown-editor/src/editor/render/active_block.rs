use super::{MARKDOWN_BODY_FONT_SIZE, MARKDOWN_BODY_LINE_HEIGHT, MarkdownEditor};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, prelude::FluentBuilder as _,
    px,
};
use gpui_component::{ElementExt, StyledExt, input::Input, v_flex};
use markdown_source::{SourceBlock, SourceBlockKind};

impl MarkdownEditor {
    pub(super) fn render_active_block(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let editor = cx.entity();
        let block_id = block.id;
        let heading = heading_level(block);
        let source_code = is_source_code(block);
        let list_gutter = super::list_marker_source::list_gutter_width(block);
        let content = gpui::div()
            .flex()
            .flex_col()
            .w_full()
            .min_w_0()
            .when(source_code, |this| self.style_source_editor(this))
            .when(matches!(block.kind, SourceBlockKind::BlockQuote), |this| {
                this.border_l_3()
                    .border_color(self.theme.border)
                    .px_4()
                    .text_color(self.theme.muted_foreground)
            })
            .relative()
            .child(
                gpui::div()
                    .debug_selector(|| "markdown-active-input-slot".to_owned())
                    .flex()
                    .w_full()
                    .min_w_0()
                    .child(self.active_input(heading)),
            )
            .children(self.active_inline_math_overlays())
            .when_some(list_gutter, |this, gutter| {
                this.pl(px(gutter))
                    .child(self.active_list_marker_overlay(block))
            });
        v_flex()
            .id(("markdown-active-block", block.id.0))
            .debug_selector(|| format!("markdown-active-block-{}", block.id.0))
            .w_full()
            .min_w_0()
            .on_prepaint(move |bounds, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor.record_measured_block_height(block_id, bounds.size.height, cx);
                });
            })
            .child(content)
            .into_any_element()
    }

    fn active_input(&self, heading: Option<u8>) -> Input {
        Input::new(&self.input)
            .w_full()
            .h_auto()
            .bare()
            .bordered(false)
            .focus_bordered(false)
            .local_style(self.input_style())
            .highlight_theme(self.theme.highlight_theme.clone())
            .caret_color(self.theme.primary)
            .indent_guide_color(self.theme.border)
            .editor_scrollbar(false)
            .text_layout_margin(false)
            .text_size(px(MARKDOWN_BODY_FONT_SIZE))
            .line_height(px(MARKDOWN_BODY_LINE_HEIGHT))
            .when_some(heading, style_heading_input)
    }

    fn style_source_editor(&self, element: gpui::Div) -> gpui::Div {
        element
            .rounded_md()
            .border_1()
            .border_color(self.theme.border)
            .bg(self.theme.border.opacity(0.12))
            .p_2()
    }
}

fn heading_level(block: &SourceBlock) -> Option<u8> {
    match block.kind {
        SourceBlockKind::Heading { level, .. } => Some(level),
        _ => None,
    }
}

fn is_source_code(block: &SourceBlock) -> bool {
    matches!(
        block.kind,
        SourceBlockKind::CodeFence { .. } | SourceBlockKind::MathBlock { .. }
    )
}

fn style_heading_input(input: Input, level: u8) -> Input {
    match level {
        1 => input.text_size(px(30.)).line_height(px(36.)).font_bold(),
        2 => input.text_size(px(24.)).line_height(px(30.)).font_bold(),
        3 => input
            .text_size(px(20.))
            .line_height(px(26.))
            .font_semibold(),
        4 => input.text_size(px(17.)).font_semibold(),
        _ => input.text_size(px(16.)).font_semibold(),
    }
}

fn heading_height_extra(level: Option<u8>) -> f32 {
    level.map_or(0., |level| match level {
        1 => 1.8,
        2 => 1.5,
        3 => 1.25,
        _ => 1.,
    })
}

pub(super) fn active_block_height(rows: f32, heading_level: Option<u8>, source_code: bool) -> f32 {
    if source_code {
        return rows.mul_add(1.5, 1.25).max(5.5);
    }
    rows.mul_add(1.5, heading_height_extra(heading_level))
}
