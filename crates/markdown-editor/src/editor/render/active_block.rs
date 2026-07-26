use super::{MARKDOWN_BODY_FONT_SIZE, MARKDOWN_BODY_LINE_HEIGHT, MarkdownEditor};
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{ElementExt, StyledExt, input::Input, v_flex};
use markdown_source::{SourceBlock, SourceBlockKind};

impl MarkdownEditor {
    /// Renders the one long-lived edit surface owned by this block.
    ///
    /// Focus only changes projection/caret state and the compatibility debug
    /// selector. It must never select a different child tree: doing so would
    /// discard InputState layout/focus state and make the block jump between
    /// preview and editing metrics.
    pub(super) fn render_block_edit_surface(
        &self,
        block: &SourceBlock,
        records_block_height: bool,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = MarkdownSurfaceKey::block(block.id);
        let surface = self
            .surface(key)
            .expect("every non-table markdown block must own an edit surface");
        let input = surface.input.clone();
        let active = self.active_block == Some(block.id) && self.active_surface_key() == key;
        let editor = cx.entity();
        let click_editor = editor.clone();
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
                    .id(("markdown-block-input-slot", block.id.0))
                    .debug_selector(move || format!("markdown-block-input-slot-{}", block_id.0))
                    .flex()
                    .w_full()
                    .min_w_0()
                    .child(self.surface_input(&input, heading)),
            )
            .children(self.inline_math_overlays(key))
            .when_some(list_gutter, |this, gutter| {
                this.pl(px(gutter))
                    .child(self.list_marker_overlay(key, block, active, cx))
            });
        v_flex()
            .id(("markdown-edit-surface", block.id.0))
            .debug_selector(move || {
                if active {
                    format!("markdown-active-block-{}", block_id.0)
                } else {
                    format!("markdown-preview-block-{}", block_id.0)
                }
            })
            .w_full()
            .min_w_0()
            .cursor_text()
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                click_editor.update(cx, |editor, cx| {
                    if editor.active_block == Some(block_id) && editor.active_surface_key() == key {
                        return;
                    }
                    if event.click_count == 1 && !event.modifiers.shift {
                        editor.activate_surface_at_position(key, event.position, window, cx);
                    } else {
                        editor.focus_surface(key, window, cx);
                    }
                });
            })
            .when(records_block_height, |this| {
                this.on_prepaint(move |bounds, _, cx| {
                    editor.update(cx, |editor, cx| {
                        editor.record_measured_block_height(block_id, bounds.size.height, cx);
                    });
                })
            })
            .child(content)
            .into_any_element()
    }

    fn surface_input(
        &self,
        input: &gpui::Entity<gpui_component::input::InputState>,
        heading: Option<u8>,
    ) -> Input {
        Input::new(input)
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
        SourceBlockKind::CodeFence { .. }
            | SourceBlockKind::MathBlock { .. }
            | SourceBlockKind::Html
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
