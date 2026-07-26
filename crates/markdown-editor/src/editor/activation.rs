use super::MarkdownEditor;
use super::surface::MarkdownSurfaceKey;
use gpui::{Context, Pixels, Point, Window};
use markdown_source::SourceSelection;

impl MarkdownEditor {
    /// Restores the preview projection of the surface that was active before
    /// switching to another mounted input.
    ///
    /// A surface is kept mounted for the lifetime of the document so that
    /// activating a block does not replace its layout tree.  Consequently we
    /// cannot rely on an input blur event to collapse the old projection:
    /// GPUI may focus the next input before the old input has emitted blur (and
    /// the first click path can bypass that event altogether).  Collapse it
    /// explicitly while the old surface's source selection is still
    /// available, then let the caller activate the new key.
    fn collapse_previous_surface(
        &mut self,
        next_key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let previous_key = self.active_surface_key();
        if previous_key == next_key {
            return;
        }
        let Some(selection) = self.surface_selection(previous_key, cx) else {
            return;
        };
        self.collapse_surface_projection(previous_key, selection, window, cx);
    }

    /// Activates an already-mounted edit surface and maps the window-space
    /// click through that surface's own laid-out input.
    ///
    /// This is the first-click fallback for a block whose `InputState` has
    /// existed since the document was rendered. It deliberately does not
    /// estimate a visual line or rebuild an editor subtree: the input's layout
    /// remains the source of truth for wrapped lines and horizontal caret
    /// placement.
    pub(super) fn activate_surface_at_position(
        &mut self,
        key: MarkdownSurfaceKey,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(surface) = self.surface(key) else {
            return false;
        };
        let input = surface.input.clone();
        let Some(display_offset) = input.read(cx).offset_for_position(position) else {
            return false;
        };
        let source_offset = surface.projection.display_to_source(display_offset);
        self.collapse_previous_surface(key, window, cx);
        if !self.set_active_surface(key) {
            return false;
        }
        self.sync_surface_selection(
            key,
            SourceSelection {
                anchor: source_offset,
                head: source_offset,
            },
            window,
            cx,
        );
        input.update(cx, |input, cx| input.focus(window, cx));
        true
    }

    /// Makes an already-mounted surface active without changing the selection.
    ///
    /// The input still owns double/triple-click and shift-selection behavior;
    /// this helper only supplies the missing active/focus transition on the
    /// first interaction.
    pub(super) fn focus_surface(
        &mut self,
        key: MarkdownSurfaceKey,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(input) = self.surface(key).map(|surface| surface.input.clone()) else {
            return false;
        };
        self.collapse_previous_surface(key, window, cx);
        if !self.set_active_surface(key) {
            return false;
        }
        input.update(cx, |input, cx| input.focus(window, cx));
        cx.notify();
        true
    }
}
