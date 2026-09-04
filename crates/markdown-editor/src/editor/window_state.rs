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

    /// Whether last frame's child indices still address the same children.
    /// Footprints are read back by index, so anything added to the scroll column
    /// would otherwise pair the wrong rows silently; a changed child count means
    /// the recorded indices are stale and the refresh must be skipped.
    pub(super) fn mounted_run_is_addressable(&self, run: MountedRun) -> bool {
        run.child_count > 0
            && self
                .scroll_handle
                .bounds_for_item(run.child_count - 1)
                .is_some()
            && self
                .scroll_handle
                .bounds_for_item(run.child_count)
                .is_none()
    }

    /// Picks the contiguous run of rows to mount; the culled runs become
    /// spacers and the focused row stays mounted, on its own island when it
    /// falls outside the run. `strides[i]` is row `i`'s
    /// footprint (height plus trailing gap); being scroll-invariant, their running
    /// sum places each row against a band from the current scroll offset.
    /// Unmeasured rows use a lower-bound estimate; where that falls short of the
    /// scroll offset the trailing run is mounted instead, so the window never
    /// lands on a spacer. Pure, so it is unit-tested headlessly.
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
                focus_island: None,
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

        // Nothing hit the band: the scroll offset is past everything the strides
        // account for, because rows the window has yet to mount are still lower
        // bounds. Fall back to the trailing run rather than a single row, so the
        // viewport stays filled while the remaining heights are learned.
        if run_start >= run_end {
            run_end = n;
            bottom_of_end = total;
            run_start = n - 1;
            top_of_start = total - strides[n - 1].max(0.0);
            let floor = (total - viewport_height - overdraw).max(0.0);
            while run_start > 0 && top_of_start > floor {
                run_start -= 1;
                top_of_start -= strides[run_start].max(0.0);
            }
        }

        // Keep the focused row mounted; GPUI blurs an unmounted caret. It goes on
        // its own island rather than widening the run, so a caret left behind
        // while reading does not drag every row between it and the viewport on
        // screen with it.
        let mut top_h = top_of_start;
        let mut bottom_h = total - bottom_of_end;
        let mut focus_island = None;
        if let Some(focus_row) = focus_row.map(|row| row.min(n - 1)) {
            let focus_top: f32 = strides[..focus_row].iter().map(|s| s.max(0.0)).sum();
            let focus_bottom = focus_top + strides[focus_row].max(0.0);
            if focus_row < run_start {
                focus_island = Some(FocusIsland {
                    row: focus_row,
                    lead_h: focus_top,
                });
                top_h = top_of_start - focus_bottom;
            } else if focus_row >= run_end {
                focus_island = Some(FocusIsland {
                    row: focus_row,
                    lead_h: focus_top - bottom_of_end,
                });
                bottom_h = total - focus_bottom;
            }
        }

        RenderWindow {
            run_start,
            run_end,
            top_h: top_h.max(0.0),
            bottom_h: bottom_h.max(0.0),
            focus_island: focus_island.map(|island| FocusIsland {
                lead_h: island.lead_h.max(0.0),
                ..island
            }),
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
        cx.emit(EditorEvent::ViewModeChanged {
            mode: self.view_mode,
        });
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
    use gpui::{Modifiers, MouseButton, TestAppContext, px};

    use super::Editor;
    use crate::components::{BlockKind, TableColumnAlignment};
    use crate::theme::ThemeManager;

    fn uniform_strides(count: usize, height: f32) -> Vec<f32> {
        vec![height; count]
    }

    fn init_editor_test_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            gpui_component::init(cx);
            ThemeManager::init(cx);
            crate::components::init(cx);
        });
    }

    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.background_executor.run_until_parked();
        cx.run_until_parked();
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

    #[test]
    fn rendered_window_keeps_focus_row_mounted() {
        let strides = uniform_strides(100, 50.0);
        // Viewport at the top, caret parked far below at row 80.
        let window = Editor::rendered_window(&strides, 0.0, 400.0, 0.0, Some(80));

        // The caret rides its own island; the rows above it stay culled.
        assert_eq!(window.run_start, 0);
        assert_eq!(window.run_end, 9);
        let island = window.focus_island.expect("caret row stays mounted");
        assert_eq!(island.row, 80);
        assert!((island.lead_h - 3550.0).abs() < 0.01);
    }

    #[test]
    fn rendered_window_focus_above_run_does_not_widen_it() {
        // Reading downward leaves the caret at the top of the document, so the rows
        // between it and the viewport must stay culled.
        let strides = uniform_strides(100, 50.0);
        let window = Editor::rendered_window(&strides, 2000.0, 400.0, 0.0, Some(0));

        assert_eq!(window.run_start, 39);
        assert_eq!(window.run_end, 49);
        let island = window.focus_island.expect("caret row stays mounted");
        assert_eq!(island.row, 0);
        assert_eq!(island.lead_h, 0.0);
        assert!((window.top_h - 1900.0).abs() < 0.01);
    }

    #[test]
    fn rendered_window_focus_inside_run_needs_no_island() {
        let strides = uniform_strides(100, 50.0);
        let window = Editor::rendered_window(&strides, 2000.0, 400.0, 0.0, Some(42));

        assert_eq!(window.run_start, 39);
        assert_eq!(window.run_end, 49);
        assert_eq!(window.focus_island, None);
    }

    #[test]
    fn rendered_window_scrolled_past_estimates_mounts_trailing_run() {
        // Rows the window has never mounted are lower bounds, so the scroll offset
        // can sit past their running sum. The tail must still fill the viewport.
        let strides = uniform_strides(100, 20.0); // total 2000
        let window = Editor::rendered_window(&strides, 9000.0, 400.0, 200.0, None);

        assert_eq!(window.run_end, 100);
        assert_eq!(window.bottom_h, 0.0);
        let mounted: f32 = strides[window.run_start..window.run_end].iter().sum();
        assert!(
            mounted >= 600.0,
            "a viewport plus overdraw must stay mounted, got {mounted}px"
        );
    }

    #[test]
    fn rendered_window_preserves_total_height() {
        let strides = uniform_strides(200, 37.0);
        let total: f32 = strides.iter().sum();

        for &(scroll_y, viewport_height, focus) in &[
            (0.0f32, 500.0f32, None),
            (3000.0, 500.0, None),
            (37.0 * 150.0, 37.0 * 5.0, Some(10usize)),
        ] {
            let window = Editor::rendered_window(&strides, scroll_y, viewport_height, 200.0, focus);
            let rendered: f32 = strides[window.run_start..window.run_end].iter().sum();
            let island: f32 = window
                .focus_island
                .map_or(0.0, |island| island.lead_h + strides[island.row]);
            assert!(
                (window.top_h + rendered + island + window.bottom_h - total).abs() < 0.01,
                "height invariant broken at scroll {scroll_y}"
            );
        }
    }

    #[gpui::test]
    async fn reading_to_the_bottom_leaves_the_caret_row_behind(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        // Scrolling never moves the caret, so it stays where the document loaded.
        // The rows between it and the viewport must not ride along.
        let markdown = (0..200)
            .map(|index| format!("## Section {index}\n\nParagraph body for section {index}.\n"))
            .collect::<Vec<_>>()
            .join("\n");
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, markdown, None));
        for _ in 0..3 {
            redraw(cx);
        }

        for _ in 0..10 {
            editor.update(cx, |editor, _cx| {
                let max = editor.scroll_handle.max_offset().y;
                editor
                    .scroll_handle
                    .set_offset(gpui::point(gpui::px(0.0), -max));
            });
            redraw(cx);
            redraw(cx);
        }

        editor.read_with(cx, |editor, _cx| {
            let run = editor.prev_mounted_run.expect("a run was mounted");
            let (run_start, run_end) = (run.row_start, run.row_end);
            let rows = editor.document.visible_blocks().len();
            assert!(
                run_start > 0,
                "the caret's row dragged the whole prefix on screen"
            );
            assert!(
                run_end - run_start < rows / 4,
                "{} of {rows} rows mounted at the bottom of the document",
                run_end - run_start
            );
        });
    }

    #[gpui::test]
    async fn right_clicking_table_cell_opens_menu_for_exact_cell(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |".to_string();
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, markdown, None));
        for _ in 0..3 {
            redraw(cx);
        }

        let (table_block_id, second_cell, second_cell_id) = editor.read_with(cx, |editor, cx| {
            let table_block = editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .clone();
            let second_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.first())
                .and_then(|row| row.get(1))
                .expect("the table has a second-column body cell");
            (
                table_block.entity_id(),
                second_cell.clone(),
                second_cell.read(cx).record.id,
            )
        });
        let second_cell_entity_id = second_cell.entity_id();

        let cell_selector: &'static str =
            Box::leak(format!("table-cell-{second_cell_id}").into_boxed_str());
        let cell_bounds = cx
            .debug_bounds(cell_selector)
            .expect("the second-column body cell lays out");
        cx.simulate_mouse_down(
            cell_bounds.center(),
            MouseButton::Right,
            Modifiers::default(),
        );
        editor.read_with(cx, |editor, _cx| {
            assert_eq!(
                editor.active_entity_id,
                Some(second_cell_entity_id),
                "right-clicking a table cell must make that cell the action target before the popup takes focus"
            );
        });
        // gpui-component builds the popup in a deferred frame so the menu
        // receives the final mouse position before it is painted. While the
        // menu is open it holds keyboard focus for navigation (upstream
        // ContextMenu focuses the PopupMenu); the editor keeps the clicked
        // cell as its action target, and focus is restored to the menu's
        // previous focus (the cell) on dismiss — covered upstream by
        // `action_bubbles_from_trigger_and_focus_restores_on_dismiss`.
        redraw(cx);
        redraw(cx);

        editor.read_with(cx, |editor, _cx| {
            let target = editor
                .context_menu_target
                .table_target
                .expect("right-clicking a table cell should target the exact table cell");
            assert_eq!(target.table_block_id, table_block_id);
            assert_eq!(target.row, 1);
            assert_eq!(target.column, 1);
            assert_eq!(
                editor.active_entity_id,
                Some(second_cell_entity_id),
                "opening the popup must keep the clicked cell as the editor action target"
            );
        });
        cx.update(|window, cx| {
            assert!(
                !second_cell.read(cx).focus_handle.is_focused(window),
                "while the popup menu is open it holds keyboard focus for navigation"
            );
        });
    }

    #[gpui::test]
    async fn table_toolbar_overlays_without_resizing_table(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |".to_string();
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, markdown, None));
        for _ in 0..3 {
            redraw(cx);
        }

        let (table_block, second_cell, table_id) = editor.read_with(cx, |editor, cx| {
            let table_block = editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .clone();
            let second_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.first())
                .and_then(|row| row.get(1))
                .expect("the table has a second-column body cell")
                .clone();
            let table_id = table_block.read(cx).record.id;
            (table_block, second_cell, table_id)
        });

        cx.update(|window, _| window.blur());
        redraw(cx);
        let baseline = cx.debug_bounds("table-root").expect("table root lays out");
        assert!(
            cx.debug_bounds("table-toolbar").is_none(),
            "the toolbar stays hidden while the table is not focused"
        );

        editor.update(cx, |editor, cx| {
            editor.pending_focus = Some(second_cell.entity_id());
            editor.active_entity_id = Some(second_cell.entity_id());
            cx.notify();
        });
        redraw(cx);

        let toolbar_bounds = cx
            .debug_bounds("table-toolbar")
            .expect("focusing a table cell shows the table toolbar");
        assert_eq!(
            cx.debug_bounds("table-root")
                .expect("table root remains laid out"),
            baseline,
            "the table toolbar must overlay the table instead of resizing it"
        );
        assert_eq!(
            toolbar_bounds.origin.y + toolbar_bounds.size.height,
            baseline.origin.y,
            "the toolbar should sit immediately above the table"
        );
        assert!(
            toolbar_bounds.origin.y >= px(0.0),
            "the toolbar must remain inside the editor viewport for a leading table"
        );

        let center_button_selector: &'static str =
            Box::leak(format!("table-toolbar-align-center-{table_id}").into_boxed_str());
        let center_button = cx
            .debug_bounds(center_button_selector)
            .expect("the center-alignment toolbar button lays out");
        cx.simulate_click(center_button.center(), Modifiers::default());
        redraw(cx);

        table_block.read_with(cx, |block, _cx| {
            let alignments = &block
                .record
                .table
                .as_ref()
                .expect("the table record remains available")
                .alignments;
            assert_eq!(
                alignments.first(),
                Some(&TableColumnAlignment::Default),
                "clicking the toolbar must not change another column"
            );
            assert_eq!(
                alignments.get(1),
                Some(&TableColumnAlignment::Center),
                "the toolbar applies alignment to the active column"
            );
        });
        editor.read_with(cx, |editor, cx| {
            assert!(
                editor.table_axis_selection.is_none(),
                "toolbar alignment must not create a column-axis selection"
            );
            assert!(
                table_block.read(cx).table_axis_selection.is_none(),
                "toolbar alignment must not leave the table column selected"
            );
            let focused_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.first())
                .and_then(|row| row.get(1))
                .expect("the aligned column keeps its body cell");
            assert_eq!(
                editor.active_entity_id,
                Some(focused_cell.entity_id()),
                "the alignment action keeps the active cell in the edited column"
            );
        });

        cx.update(|window, _| window.blur());
        redraw(cx);
        assert!(
            cx.debug_bounds("table-toolbar").is_none(),
            "the toolbar hides after focus leaves the table"
        );
        assert_eq!(
            cx.debug_bounds("table-root")
                .expect("table root remains laid out"),
            baseline,
            "hiding the table toolbar must preserve table bounds"
        );
    }

    #[gpui::test]
    async fn moving_table_row_keeps_active_cell_column(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |".to_string();
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, markdown, None));
        for _ in 0..3 {
            redraw(cx);
        }

        let (table_block, active_cell) = editor.read_with(cx, |editor, cx| {
            let table_block = editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .clone();
            let active_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.first())
                .and_then(|row| row.get(1))
                .expect("the table has a second-column body cell")
                .clone();
            (table_block, active_cell)
        });

        editor.update(cx, |editor, cx| {
            editor.pending_focus = Some(active_cell.entity_id());
            editor.active_entity_id = Some(active_cell.entity_id());
            editor.move_table_row(&table_block, 1, 1, cx);
        });

        editor.read_with(cx, |editor, cx| {
            let expected_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.get(1))
                .and_then(|row| row.get(1))
                .expect("the moved row keeps its second-column cell");
            assert_eq!(
                editor.active_entity_id,
                Some(expected_cell.entity_id()),
                "moving a row keeps the active cell in the same column"
            );
        });
    }

    #[gpui::test]
    async fn moving_table_column_keeps_active_cell_row(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let markdown = "| A | B |\n| --- | --- |\n| 1 | 2 |\n| 3 | 4 |".to_string();
        let (editor, cx) =
            cx.add_window_view(|_window, cx| Editor::from_markdown(cx, markdown, None));
        for _ in 0..3 {
            redraw(cx);
        }

        let (table_block, active_cell) = editor.read_with(cx, |editor, cx| {
            let table_block = editor
                .document
                .visible_blocks()
                .into_iter()
                .find(|visible| visible.entity.read(cx).kind() == BlockKind::Table)
                .expect("the markdown table becomes a table block")
                .entity
                .clone();
            let active_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.get(1))
                .and_then(|row| row.first())
                .expect("the table has a second body-row cell")
                .clone();
            (table_block, active_cell)
        });

        editor.update(cx, |editor, cx| {
            editor.pending_focus = Some(active_cell.entity_id());
            editor.active_entity_id = Some(active_cell.entity_id());
            editor.move_table_column(&table_block, 0, 1, cx);
        });

        editor.read_with(cx, |editor, cx| {
            let expected_cell = table_block
                .read(cx)
                .table_runtime
                .as_ref()
                .and_then(|runtime| runtime.rows.get(1))
                .and_then(|row| row.get(1))
                .expect("the moved column keeps its second-body-row cell");
            assert_eq!(
                editor.active_entity_id,
                Some(expected_cell.entity_id()),
                "moving a column keeps the active cell in the same row"
            );
        });
    }
}
