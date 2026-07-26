use super::{MARKDOWN_BODY_FONT_SIZE, MarkdownEditor};
use crate::editor::surface::MarkdownSurfaceKey;
use gpui::{
    Context, InteractiveElement, IntoElement, MouseButton, ParentElement, Styled,
    prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ElementExt,
    text::{MarkdownPalette, TextView, TextViewStyle},
};
use markdown_source::SourceBlock;

impl MarkdownEditor {
    /// Keeps native HTML rendering and the block's source Input mounted in one
    /// permanent grid cell. Activating the block only changes which layer is
    /// visible; both layers keep contributing their natural size, so neither
    /// Input identity nor document geometry changes on click.
    pub(super) fn render_html_shell(
        &self,
        block: &SourceBlock,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let key = MarkdownSurfaceKey::block(block.id);
        let active = self.active_block == Some(block.id) && self.active_surface_key() == key;
        let editor = cx.entity();
        let click_editor = editor.clone();
        let block_id = block.id;
        let input_layer = gpui::div()
            .id(("markdown-html-input-layer", block.id.0))
            .debug_selector(move || format!("markdown-html-input-layer-{}", block_id.0))
            .col_start(1)
            .row_start(1)
            .w_full()
            .min_w_0()
            .opacity(if active { 1. } else { 0. })
            .child(self.render_block_edit_surface(block, false, cx));
        let native_layer = gpui::div()
            .id(("markdown-html-native-layer", block.id.0))
            .debug_selector(move || format!("markdown-html-native-layer-{}", block_id.0))
            .col_start(1)
            .row_start(1)
            .w_full()
            .min_w_0()
            .cursor_text()
            .text_size(px(MARKDOWN_BODY_FONT_SIZE))
            .text_color(self.theme.foreground)
            .when(active, |this| this.invisible())
            .on_mouse_down(MouseButton::Left, move |event, window, cx| {
                click_editor.update(cx, |editor, cx| {
                    if event.click_count == 1 && !event.modifiers.shift {
                        editor.activate_surface_at_position(key, event.position, window, cx);
                    } else {
                        editor.focus_surface(key, window, cx);
                    }
                });
            })
            .child(
                TextView::html(
                    ("markdown-html-native-view", block.id.0),
                    block.original_source.clone(),
                )
                .style(self.html_preview_style())
                .w_full(),
            );
        gpui::div()
            .id(("markdown-html-shell", block.id.0))
            .debug_selector(move || format!("markdown-html-shell-{}", block_id.0))
            .grid()
            .grid_cols(1)
            .grid_rows(1)
            .w_full()
            .min_w_0()
            .on_prepaint(move |bounds, _, cx| {
                editor.update(cx, |editor, cx| {
                    editor.record_measured_block_height(block_id, bounds.size.height, cx);
                });
            })
            .child(input_layer)
            .child(native_layer)
            .into_any_element()
    }

    fn html_preview_style(&self) -> TextViewStyle {
        let muted_background = self.theme.border.opacity(0.12);
        let mut style = TextViewStyle::default().markdown_palette(MarkdownPalette {
            is_dark: self.theme.background.l < 0.5,
            foreground: self.theme.foreground,
            muted_foreground: self.theme.muted_foreground,
            border: self.theme.border,
            code_background: muted_background,
            code_foreground: self.theme.foreground,
            table_header: self.theme.border.opacity(0.16),
            table_row: self.theme.background,
            table_row_alt: self.theme.border.opacity(0.08),
            quote_border: self.theme.border,
            link: self.theme.primary,
        });
        style.heading_base_font_size = px(MARKDOWN_BODY_FONT_SIZE);
        style.highlight_theme = self.theme.highlight_theme.clone();
        style
    }
}
