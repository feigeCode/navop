use super::super::MarkdownEditor;
use super::super::surface::MarkdownSurfaceKey;
use gpui::{Context, Window};

impl MarkdownEditor {
    pub(in crate::editor) fn surface_input_changed(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.syncing_input || !self.set_active_surface(key) {
            return;
        }
        let Some(surface) = self.surface(key) else {
            return;
        };
        let value = surface.input.read(cx).value().to_string();
        if value == surface.projection.text {
            return;
        }
        let cursor = surface
            .projection
            .display_to_source(surface.input.read(cx).selected_range().end);
        if let Some(edit) = surface.projection.edit_for_value(&value)
            && !self.surface_is_source_code(key)
            && edit.source_range.is_empty()
            && edit.replacement == "\n"
        {
            self.pending_newline = Some((key, edit.source_range.start));
            cx.defer_in(window, move |editor, window, cx| {
                editor.flush_pending_newline(key, window, cx);
            });
            return;
        }
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_surface(key, cursor, window, cx);
        }
    }

    pub(in crate::editor) fn surface_input_entered(
        &mut self,
        key: MarkdownSurfaceKey,
        secondary: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.syncing_input || !self.set_active_surface(key) {
            return;
        }
        if self.surface_is_source_code(key) {
            let Some(surface) = self.surface(key) else {
                return;
            };
            let value = surface.input.read(cx).value().to_string();
            let cursor = surface
                .projection
                .display_end_to_source(surface.input.read(cx).selected_range().end);
            if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
                self.resync_surface(key, cursor, window, cx);
            }
            return;
        }
        let Some((pending_key, source_offset)) = self.pending_newline.take() else {
            return;
        };
        if pending_key != key {
            self.pending_newline = Some((pending_key, source_offset));
            return;
        }
        if !secondary && matches!(self.split_active_block(source_offset, window, cx), Ok(true)) {
            self.focus_active_surface(window, cx);
            return;
        }
        let Some(surface) = self.surface(key) else {
            return;
        };
        let value = surface.input.read(cx).value().to_string();
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_surface(key, source_offset, window, cx);
        }
    }

    fn flush_pending_newline(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((pending_key, source_offset)) = self.pending_newline.take() else {
            return;
        };
        if pending_key != key {
            self.pending_newline = Some((pending_key, source_offset));
            return;
        }
        let Some(surface) = self.surface(key) else {
            return;
        };
        let value = surface.input.read(cx).value().to_string();
        if !matches!(self.edit_projected_value(&value, window, cx), Ok(true)) {
            self.resync_surface(key, source_offset, window, cx);
        }
    }

    pub(in crate::editor) fn surface_focused(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.syncing_input || !self.set_active_surface(key) {
            return;
        }
        let selection = self.surface_selection(key, cx).unwrap_or_default();
        self.sync_surface_selection(key, selection, window, cx);
    }

    pub(in crate::editor) fn surface_blurred(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.syncing_input || self.active_surface != Some(key) {
            return;
        }
        let Some(selection) = self.surface_selection(key, cx) else {
            return;
        };
        self.collapse_surface_projection(key, selection, window, cx);
    }

    pub(in crate::editor) fn surface_cursor_changed(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.syncing_input || self.active_surface != Some(key) {
            return;
        }
        let Some(surface) = self.surface(key) else {
            return;
        };
        let display_cursor = surface.input.read(cx).selected_range().end;
        let active_inline = self.active_inline_at_display(key, display_cursor);
        if active_inline == surface.projection.active_inline {
            return;
        }
        let selection = self.surface_selection(key, cx).unwrap_or_default();
        self.sync_surface_selection(key, selection, window, cx);
    }

    fn surface_is_source_code(&self, key: MarkdownSurfaceKey) -> bool {
        self.surface(key).is_some_and(|surface| {
            matches!(
                surface.mode,
                super::super::surface::MarkdownInputMode::Code(_)
            )
        })
    }

    fn focus_active_surface(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.active_surface_key();
        if let Some(surface) = self.surface(key) {
            surface
                .input
                .update(cx, |input, cx| input.focus(window, cx));
        }
    }
}
