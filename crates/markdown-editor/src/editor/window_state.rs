//! Window-level editor state such as scrolling and view-mode switching.

use super::*;

impl Editor {
    /// Returns the scroll correction needed to keep the row intersecting
    /// `scroll_y` at the same viewport position after cached row strides
    /// change. Only rows before the anchor affect its painted top.
    pub(super) fn row_stride_anchor_delta(
        old_strides: &[f32],
        new_strides: &[f32],
        scroll_y: f32,
    ) -> f32 {
        if old_strides.is_empty() || old_strides.len() != new_strides.len() {
            return 0.0;
        }

        let old_total: f32 = old_strides.iter().map(|stride| stride.max(0.0)).sum();
        let anchor_y = scroll_y.max(0.0).min(old_total.max(0.0));
        let mut old_cursor = 0.0f32;
        let mut anchor = old_strides.len() - 1;
        for (index, stride) in old_strides.iter().enumerate() {
            let next = old_cursor + stride.max(0.0);
            if anchor_y < next {
                anchor = index;
                break;
            }
            old_cursor = next;
        }

        old_strides[..anchor]
            .iter()
            .zip(&new_strides[..anchor])
            .map(|(old, new)| new.max(0.0) - old.max(0.0))
            .sum()
    }

    pub(super) fn scrollbar_geometry(
        viewport_height: f32,
        max_scroll_y: f32,
        current_scroll_y: f32,
    ) -> ScrollbarGeometry {
        let track_height = viewport_height.max(20.0);
        let content_height = viewport_height + max_scroll_y;
        let thumb_height = if max_scroll_y > 0.5 {
            (track_height * (viewport_height / content_height)).clamp(28.0, track_height)
        } else {
            track_height
        };
        let progress = if max_scroll_y > 0.0 {
            current_scroll_y.clamp(0.0, max_scroll_y) / max_scroll_y
        } else {
            0.0
        };
        let thumb_top = (track_height - thumb_height).max(0.0) * progress;
        ScrollbarGeometry {
            track_height,
            thumb_height,
            thumb_top,
            max_scroll_y,
        }
    }

    pub(super) fn scroll_offset_for_thumb_top(
        thumb_top: f32,
        track_height: f32,
        thumb_height: f32,
        max_scroll_y: f32,
    ) -> f32 {
        if max_scroll_y <= 0.0 {
            return 0.0;
        }

        let travel = (track_height - thumb_height).max(0.0);
        if travel <= 0.0 {
            return 0.0;
        }

        let progress = (thumb_top / travel).clamp(0.0, 1.0);
        max_scroll_y * progress
    }

    /// Picks the contiguous run of rows to mount; the culled runs become two
    /// spacers and the focused row stays mounted. `strides[i]` is row `i`'s
    /// footprint (height plus trailing gap); being scroll-invariant, their running
    /// sum places each row against a band from the current scroll offset.
    /// Unmeasured rows use a lower-bound estimate, so the window never lands on a
    /// spacer. Pure, so it is unit-tested headlessly.
    pub(super) fn rendered_window(
        strides: &[f32],
        scroll_y: f32,
        viewport_height: f32,
        overdraw: f32,
        focus_row: Option<usize>,
    ) -> RenderWindow {
        let n = strides.len();
        if n == 0 {
            return RenderWindow {
                run_start: 0,
                run_end: 0,
                top_h: 0.0,
                bottom_h: 0.0,
            };
        }

        let total: f32 = strides.iter().map(|stride| stride.max(0.0)).sum();

        // `scroll_y` comes from GPUI's real scroll container. That container is
        // taller than the virtual row model because it also includes editor
        // padding and the deliberate "scroll beyond bottom" area. It can also
        // temporarily retain a max offset measured before row-height estimates
        // settle. In both cases the real offset may be past the estimated rows.
        //
        // Windowing against that unbounded value makes the visible band miss
        // every row. The old fallback then mounted only the final row, leaving
        // most (or all) of the viewport blank. Select rows using the natural
        // row-content scroll range instead; the actual scroll container still
        // keeps its trailing room, but the mounted run always fills the part of
        // the viewport occupied by document content.
        let viewport_height = viewport_height.max(0.0);
        let row_scroll_y = scroll_y.max(0.0).min((total - viewport_height).max(0.0));
        let band_top = row_scroll_y - overdraw;
        let band_bottom = row_scroll_y + viewport_height + overdraw;

        let mut run_start = n;
        let mut run_end = 0usize;
        let mut top_of_start = 0.0f32;
        let mut bottom_of_end = 0.0f32;
        let mut cursor = 0.0f32;
        for (index, &stride) in strides.iter().enumerate() {
            let top = cursor;
            let bottom = cursor + stride.max(0.0);
            if bottom >= band_top && top <= band_bottom {
                if index < run_start {
                    run_start = index;
                    top_of_start = top;
                }
                run_end = index + 1;
                bottom_of_end = bottom;
            }
            cursor = bottom;
        }
        debug_assert!((cursor - total).abs() < 0.01);

        // Nothing hit the band (for example, all rows have a zero footprint):
        // mount the last row so the viewport never lands on a spacer.
        if run_start >= run_end {
            run_start = n - 1;
            run_end = n;
            top_of_start = total - strides[n - 1].max(0.0);
            bottom_of_end = total;
        }

        // Keep the focused row mounted; GPUI blurs an unmounted caret. Reaching a
        // far focus row widens the run, but autoscroll makes that rare.
        if let Some(focus_row) = focus_row {
            let focus_row = focus_row.min(n - 1);
            if focus_row < run_start {
                run_start = focus_row;
                top_of_start = strides[..focus_row].iter().map(|s| s.max(0.0)).sum();
            }
            if focus_row + 1 > run_end {
                run_end = focus_row + 1;
                bottom_of_end = strides[..=focus_row].iter().map(|s| s.max(0.0)).sum();
            }
        }

        RenderWindow {
            run_start,
            run_end,
            top_h: top_of_start.max(0.0),
            bottom_h: (total - bottom_of_end).max(0.0),
        }
    }

    /// Linearly interpolates the editor content width ratio based on viewport
    /// width. The column stays full-width until `centered_shrink_start`, then
    /// shrinks to `centered_min_ratio` at `centered_shrink_end`.
    pub(super) fn centered_column_ratio(
        viewport_width: f32,
        dimensions: &crate::theme::ThemeDimensions,
    ) -> f32 {
        if viewport_width <= dimensions.centered_shrink_start {
            return 1.0;
        }

        let t = ((viewport_width - dimensions.centered_shrink_start)
            / (dimensions.centered_shrink_end - dimensions.centered_shrink_start))
            .clamp(0.0, 1.0);
        1.0 - t * (1.0 - dimensions.centered_min_ratio)
    }

    pub(crate) fn centered_column_width(
        viewport_width: f32,
        dimensions: &crate::theme::ThemeDimensions,
    ) -> f32 {
        let available_content_width = (viewport_width - dimensions.editor_padding * 2.0).max(1.0);
        let centered_ratio = Self::centered_column_ratio(viewport_width, dimensions);
        (available_content_width * centered_ratio)
            .max(320.0)
            .min(available_content_width)
    }

    pub(crate) fn on_toggle_view_mode_action(
        &mut self,
        _: &crate::components::ToggleViewMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.toggle_view_mode_from_ui(cx);
    }

    pub(super) fn toggle_view_mode_from_ui(&mut self, cx: &mut Context<Self>) {
        self.end_block_pointer_selection_sessions(cx);
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.toggle_view_mode(cx);
    }

    pub(crate) fn on_undo(
        &mut self,
        _: &crate::components::Undo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.undo_document(cx);
    }

    pub(crate) fn on_redo(
        &mut self,
        _: &crate::components::Redo,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.redo_document(cx);
    }

    pub(crate) fn toggle_view_mode(&mut self, cx: &mut Context<Self>) {
        let target = match self.view_mode {
            ViewMode::Rendered => ViewMode::Source,
            ViewMode::Source => ViewMode::Rendered,
        };
        self.switch_to_view_mode(target, cx);
    }

    pub(super) fn switch_to_view_mode(&mut self, target: ViewMode, cx: &mut Context<Self>) -> bool {
        if self.view_mode == target {
            return false;
        }

        self.end_block_pointer_selection_sessions(cx);
        let selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.clear_cross_block_selection(cx);
        self.rendered_select_all_cycle = None;
        match target {
            ViewMode::Source => {
                debug_assert_eq!(self.view_mode, ViewMode::Rendered);
                let markdown = self.document.markdown_text(cx);
                let block = Self::new_block(cx, BlockRecord::paragraph(markdown));
                block.update(cx, |block, _cx| block.set_source_document_mode());
                self.document.replace_roots(vec![block], cx);
                self.view_mode = ViewMode::Source;
                self.table_cells.clear();
                self.rebuild_image_runtimes(cx);
            }
            ViewMode::Rendered => {
                debug_assert_eq!(self.view_mode, ViewMode::Source);
                let source = self.document.raw_source_text(cx);
                let roots = Self::build_rendered_roots(cx, &source);
                self.document.replace_roots(roots, cx);
                self.view_mode = ViewMode::Rendered;
                self.rebuild_table_runtimes(cx);
                self.rebuild_image_runtimes(cx);
            }
        }

        self.apply_selection_snapshot_in_current_mode(&selection_snapshot, cx);
        self.pending_scroll_active_block_into_view = true;
        self.pending_scroll_recheck_after_layout = true;
        self.last_scroll_viewport_size = None;
        self.table_axis_preview = None;
        self.table_axis_selection = None;
        self.dismiss_contextual_overlays(cx);
        self.sync_table_axis_visuals(cx);
        self.refresh_stable_document_snapshot(cx);
        cx.notify();
        true
    }

    /// Marks the host-managed document dirty.
    pub(super) fn mark_dirty(&mut self, cx: &mut Context<Self>) {
        if !self.document_dirty {
            self.document_dirty = true;
            cx.notify();
        }
    }

    pub(super) fn request_active_block_scroll_into_view(&mut self, cx: &mut Context<Self>) {
        self.pending_scroll_recheck_after_layout = true;
        if !self.pending_scroll_active_block_into_view {
            self.pending_scroll_active_block_into_view = true;
            cx.notify();
        }
    }

    pub(super) fn viewport_size_changed(previous: Size<Pixels>, current: Size<Pixels>) -> bool {
        const EPSILON: f32 = 0.5;

        (f32::from(previous.width) - f32::from(current.width)).abs() > EPSILON
            || (f32::from(previous.height) - f32::from(current.height)).abs() > EPSILON
    }

    pub(crate) fn request_open_link_prompt(
        &mut self,
        prompt_target: String,
        open_target: String,
        cx: &mut Context<Self>,
    ) {
        self.pending_open_link = Some(PendingOpenLink {
            prompt_target,
            open_target,
        });
        cx.notify();
    }
}

#[cfg(test)]
mod tests {
    use super::Editor;

    fn uniform_strides(count: usize, height: f32) -> Vec<f32> {
        vec![height; count]
    }

    #[test]
    fn row_stride_anchor_delta_compensates_changes_before_anchor() {
        let old = [20.0, 20.0, 20.0, 20.0];
        let new = [30.0, 25.0, 20.0, 20.0];

        assert_eq!(Editor::row_stride_anchor_delta(&old, &new, 45.0), 15.0);
    }

    #[test]
    fn row_stride_anchor_delta_ignores_changes_after_anchor() {
        let old = [20.0, 20.0, 20.0, 20.0];
        let new = [20.0, 20.0, 60.0, 80.0];

        assert_eq!(Editor::row_stride_anchor_delta(&old, &new, 25.0), 0.0);
    }

    #[test]
    fn row_stride_anchor_delta_uses_last_row_for_stale_bottom_offset() {
        let old = [20.0, 20.0, 20.0];
        let new = [30.0, 25.0, 20.0];

        assert_eq!(Editor::row_stride_anchor_delta(&old, &new, 4_000.0), 15.0);
    }

    #[test]
    fn rendered_window_clamps_scroll_past_estimated_rows_to_document_bottom() {
        let strides = uniform_strides(500, 20.0);
        let viewport_height = 400.0;

        let window = Editor::rendered_window(&strides, 20_000.0, viewport_height, 0.0, None);
        let mounted_height: f32 = strides[window.run_start..window.run_end].iter().sum();

        assert_eq!(window.run_end, strides.len());
        assert_eq!(window.bottom_h, 0.0);
        assert!(
            mounted_height >= viewport_height,
            "bottom window must mount enough rows to fill the document portion of the viewport"
        );
    }

    #[test]
    fn rendered_window_mounts_short_estimated_document_at_stale_scroll_offset() {
        let strides = uniform_strides(10, 20.0);

        let window = Editor::rendered_window(&strides, 4_000.0, 800.0, 0.0, None);

        assert_eq!(window.run_start, 0);
        assert_eq!(window.run_end, strides.len());
        assert_eq!(window.top_h, 0.0);
        assert_eq!(window.bottom_h, 0.0);
    }
}
