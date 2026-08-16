//! Editor window rendering: centered scrollable block column,
//! unsaved-changes overlay dialog, custom scrollbar, and deferred
//! operations (focus, scroll, save, window title).

use std::time::{Duration, Instant};

use gpui::*;
use gpui_component::ElementExt as _;
use rust_i18n::t;

use super::{Editor, MountedRun};
use crate::components::Block;
use crate::components::CalloutVariant;
use crate::theme::{Theme, ThemeDimensions};

/// Rows within this many pixels of the viewport stay mounted, so a fast flick
/// paints them before they scroll in instead of showing a blank edge.
const RENDER_OVERDRAW_PX: f32 = 800.0;

/// Scroll-handle bounds are populated during layout, so an embedded editor can
/// observe a zero-sized viewport on its first render. Falling back per axis
/// keeps the initial virtualization window large enough to paint everything
/// visible instead of waiting for the first scroll event to reveal more rows.
fn effective_editor_viewport_size(
    measured: Size<Pixels>,
    editor_root: Size<Pixels>,
    window: Size<Pixels>,
) -> Size<Pixels> {
    let fallback_width = if editor_root.width > px(0.0) {
        editor_root.width
    } else {
        window.width
    };
    let fallback_height = if editor_root.height > px(0.0) {
        editor_root.height
    } else {
        window.height
    };

    Size {
        width: if measured.width > px(0.0) {
            measured.width
        } else {
            fallback_width.max(px(1.0))
        },
        height: if measured.height > px(0.0) {
            measured.height
        } else {
            fallback_height.max(px(1.0))
        },
    }
}

fn editor_text_font() -> Font {
    // FontFallbacks is internally `Arc<Vec<String>>` — building it once
    // per process and Arc-cloning per render is the right shape, since
    // editor_text_font() is called from Editor::render on every frame.
    static FALLBACKS: std::sync::OnceLock<FontFallbacks> = std::sync::OnceLock::new();
    let fallbacks = FALLBACKS
        .get_or_init(|| {
            FontFallbacks::from_fonts(tibetan_font_fallbacks_for_target_os(std::env::consts::OS))
        })
        .clone();
    let mut font = font(".SystemUIFont");
    font.fallbacks = Some(fallbacks);
    font
}

fn tibetan_font_fallbacks_for_target_os(target_os: &str) -> Vec<String> {
    let families = match target_os {
        "windows" => &[
            "Microsoft Himalaya",
            "Noto Serif Tibetan",
            "Noto Sans Tibetan",
            "BabelStone Tibetan",
        ][..],
        "macos" => &["Kailasa", "Noto Serif Tibetan", "Noto Sans Tibetan"][..],
        _ => &[
            "Noto Serif Tibetan",
            "Noto Sans Tibetan",
            "Microsoft Himalaya",
            "Kailasa",
            "BabelStone Tibetan",
        ][..],
    };
    families
        .iter()
        .map(|family| (*family).to_string())
        .collect()
}

/// Adjacent-row metadata used to collapse spacing inside visual groups.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct RenderedRowSpacingInfo {
    quote_group_anchor: Option<uuid::Uuid>,
    visible_quote_group_anchor: Option<uuid::Uuid>,
    callout_anchor: Option<uuid::Uuid>,
    callout_variant: Option<CalloutVariant>,
    is_callout_header: bool,
    footnote_anchor: Option<uuid::Uuid>,
    is_footnote_header: bool,
}

impl RenderedRowSpacingInfo {
    fn from_block(block: &Block) -> Self {
        Self {
            quote_group_anchor: block.quote_group_anchor,
            visible_quote_group_anchor: block.visible_quote_group_anchor,
            callout_anchor: block.callout_anchor,
            callout_variant: block.callout_variant,
            is_callout_header: block.kind().is_callout(),
            footnote_anchor: block.footnote_anchor,
            is_footnote_header: block.kind().is_footnote_definition(),
        }
    }
}

fn rendered_row_top_gap(
    previous: Option<RenderedRowSpacingInfo>,
    current: RenderedRowSpacingInfo,
    default_gap: f32,
) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };

    if previous.quote_group_anchor.is_some()
        && previous.quote_group_anchor == current.quote_group_anchor
    {
        0.0
    } else {
        default_gap
    }
}

fn callout_colors(variant: CalloutVariant, theme: &Theme) -> (Hsla, Hsla) {
    let c = &theme.colors;
    match variant {
        CalloutVariant::Note => (c.callout_note_border, c.callout_note_bg),
        CalloutVariant::Tip => (c.callout_tip_border, c.callout_tip_bg),
        CalloutVariant::Important => (c.callout_important_border, c.callout_important_bg),
        CalloutVariant::Warning => (c.callout_warning_border, c.callout_warning_bg),
        CalloutVariant::Caution => (c.callout_caution_border, c.callout_caution_bg),
    }
}

fn callout_row_top_gap(
    previous: Option<RenderedRowSpacingInfo>,
    current: RenderedRowSpacingInfo,
    dimensions: &ThemeDimensions,
) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };

    if previous.visible_quote_group_anchor.is_some()
        && previous.visible_quote_group_anchor == current.visible_quote_group_anchor
    {
        return 0.0;
    }

    if previous.is_callout_header {
        dimensions.callout_header_margin_bottom
    } else {
        dimensions.callout_body_gap
    }
}

fn footnote_row_top_gap(previous: Option<RenderedRowSpacingInfo>, default_gap: f32) -> f32 {
    let Some(previous) = previous else {
        return 0.0;
    };

    if previous.is_footnote_header {
        default_gap * 0.75
    } else {
        default_gap
    }
}

fn footnote_group_shell(
    children: Vec<AnyElement>,
    theme: &Theme,
    dimensions: &ThemeDimensions,
) -> AnyElement {
    div()
        .w_full()
        .flex_shrink_0()
        .flex()
        .flex_col()
        .gap(px(0.0))
        .px(px(dimensions.footnote_padding_x))
        .py(px(dimensions.footnote_padding_y))
        .rounded(px(dimensions.footnote_radius))
        .border(px(1.0))
        .border_color(theme.colors.footnote_border)
        .bg(theme.colors.footnote_bg)
        .children(children)
        .into_any_element()
}

impl Editor {
    fn apply_pending_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(entity_id) = self.pending_focus.take()
            && let Some(block) = self.focusable_entity_by_id(entity_id)
        {
            let focus_handle = block.read(cx).focus_handle.clone();
            focus_handle.focus(window, cx);
        }
    }

    fn ensure_focused_caret_visible(&mut self, window: &Window, cx: &App) -> bool {
        let Some(focused_block) = self.focused_edit_target(window, cx) else {
            return false;
        };
        let Some(active_bounds) =
            focused_block.read_with(cx, |block, _cx| block.active_range_or_cursor_bounds())
        else {
            return false;
        };

        let viewport = self.scroll_handle.bounds();
        let padding = px(20.0);
        let top_limit = viewport.top() + padding;
        let bottom_limit = viewport.bottom() - padding;
        let mut offset = self.scroll_handle.offset();
        let mut changed = false;

        if active_bounds.top() < top_limit {
            offset.y += top_limit - active_bounds.top();
            changed = true;
        } else if active_bounds.bottom() > bottom_limit {
            offset.y -= active_bounds.bottom() - bottom_limit;
            changed = true;
        }

        if changed {
            let max_offset_y = self.scroll_handle.max_offset().y.max(px(0.0));
            offset.y = offset.y.min(px(0.0)).max(-max_offset_y);
            self.scroll_handle.set_offset(offset);
        }

        true
    }

    fn apply_pending_scroll_into_view(&mut self, window: &Window, cx: &mut Context<Self>) {
        if self.scrollbar_drag.is_some() {
            return;
        }

        if !self.pending_scroll_active_block_into_view {
            return;
        }

        // scroll_to_item indexed children by position, which the spacers break;
        // the focused block is always mounted, so pixel math on its bounds works.
        let has_bounds = self.ensure_focused_caret_visible(window, cx);
        if self.pending_scroll_recheck_after_layout {
            self.pending_scroll_recheck_after_layout = false;
            self.schedule_scroll_recheck(cx);
            return;
        }

        if !has_bounds {
            self.schedule_scroll_recheck(cx);
            return;
        }

        self.pending_scroll_active_block_into_view = false;
        self.scroll_recheck_task = None;
    }

    /// Requests a repaint one frame out so a still-pending scroll-into-view can
    /// retry once the target block has been laid out. `cx.notify()` is swallowed
    /// when called from within `render`, so without this the retry would wait
    /// for the next external notify (e.g. the cursor blink, ~0.5s later).
    fn schedule_scroll_recheck(&mut self, cx: &mut Context<Self>) {
        self.scroll_recheck_task = Some(cx.spawn(async move |this: WeakEntity<Self>, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(16))
                .await;
            let _ = this.update(cx, |_this, cx| cx.notify());
        }));
    }

    fn sync_pending_open_link(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(link) = self.pending_open_link.take() else {
            return;
        };

        let open = t!("MarkdownEditor.open_link_open").to_string();
        let cancel = t!("MarkdownEditor.open_link_cancel").to_string();
        let title = t!("MarkdownEditor.open_link_title").to_string();
        let buttons = [open.as_str(), cancel.as_str()];
        let prompt = window.prompt(
            PromptLevel::Info,
            &title,
            Some(&link.prompt_target),
            &buttons,
            cx,
        );
        let window_handle = window.window_handle();
        cx.spawn(async move |_this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let Ok(choice) = prompt.await else {
                return;
            };
            if choice == 0 {
                let _ = cx.update_window(window_handle, |_view: AnyView, _window, cx| {
                    cx.open_url(&link.open_target);
                });
            }
        })
        .detach();
    }

    fn sync_scroll_viewport(&mut self, viewport_size: Size<Pixels>, cx: &mut Context<Self>) {
        match self.last_scroll_viewport_size {
            Some(previous) if Self::viewport_size_changed(previous, viewport_size) => {
                self.last_scroll_viewport_size = Some(viewport_size);
                self.request_active_block_scroll_into_view(cx);
            }
            Some(_) => {}
            None => {
                self.last_scroll_viewport_size = Some(viewport_size);
            }
        }
    }
}

impl Render for Editor {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let explicit_scroll_in_progress =
            self.pending_scroll_active_block_into_view || self.scrollbar_drag.is_some();
        self.apply_pending_focus(window, cx);
        self.apply_pending_scroll_into_view(window, cx);
        self.last_selection_snapshot = self.capture_source_selection_snapshot(cx);
        self.sync_pending_open_link(window, cx);

        let viewport_bounds = self.scroll_handle.bounds();
        let viewport_size = effective_editor_viewport_size(
            viewport_bounds.size,
            self.root_bounds.size,
            window.viewport_size(),
        );
        self.sync_scroll_viewport(viewport_size, cx);

        let theme = self.effective_theme(cx);
        let d = &theme.dimensions;
        let visible_blocks = self.document.visible_blocks().to_vec();
        let editor = cx.entity().downgrade();
        let scroll_trigger_padding = (d.block_min_height * 0.75).max(16.0);
        let max_scroll_y = f32::from(self.scroll_handle.max_offset().y.max(px(0.0)));
        let viewport_height = f32::from(viewport_size.height);
        // Extra room below the last block so the lowest line can be scrolled up
        // to the viewport center instead of being pinned to the bottom edge.
        let scroll_beyond_bottom = viewport_height * 0.5;
        let viewport_width = f32::from(viewport_size.width);
        let has_overflow = max_scroll_y > 0.5;

        let centered_width = Self::centered_column_width(viewport_width, &theme.dimensions);
        let mut current_scroll_y =
            (-f32::from(self.scroll_handle.offset().y)).clamp(0.0, max_scroll_y);
        let scrollbar_geometry =
            Self::scrollbar_geometry(viewport_height, max_scroll_y, current_scroll_y);
        let track_height = scrollbar_geometry.track_height;
        let thumb_height = scrollbar_geometry.thumb_height;
        let thumb_top = scrollbar_geometry.thumb_top;

        let show_custom_scrollbar = has_overflow
            && (self.scrollbar_drag.is_some()
                || self.scrollbar_hovered
                || Instant::now() <= self.scrollbar_visible_until);

        // Spacing metadata is read on demand instead of pre-collected into a
        // Vec<RenderedRowSpacingInfo> sized to all visible blocks. For long
        // documents this skips a ~tens-of-KB allocation per frame; per-block
        // entity.read_with is a cheap immutable lock + 7-field struct copy.
        let spacing_for = |index: usize| -> RenderedRowSpacingInfo {
            visible_blocks[index]
                .entity
                .read_with(cx, |block, _cx| RenderedRowSpacingInfo::from_block(block))
        };
        let mut previous_row_spacing = None;
        // One entry per render row; off-screen rows are dropped after windowing.
        let mut row_elements: Vec<AnyElement> = Vec::new();
        let mut row_starts: Vec<usize> = Vec::new();
        // Each row's leading `mt` gap; the top spacer subtracts the first mounted
        // row's, since that row re-applies it.
        let mut row_top_gaps: Vec<f32> = Vec::new();
        let mut index = 0usize;
        while index < visible_blocks.len() {
            let first_visible = visible_blocks[index].clone();
            let first_spacing = spacing_for(index);
            let top_gap = rendered_row_top_gap(previous_row_spacing, first_spacing, d.block_gap);

            if let (Some(callout_anchor), Some(callout_variant)) =
                (first_spacing.callout_anchor, first_spacing.callout_variant)
            {
                let mut group_children = Vec::new();
                let mut group_end = index;
                let mut previous_callout_row = None;
                while group_end < visible_blocks.len()
                    && spacing_for(group_end).callout_anchor == Some(callout_anchor)
                {
                    let row_spacing = spacing_for(group_end);
                    if let Some(footnote_anchor) = row_spacing.footnote_anchor {
                        let mut footnote_children = Vec::new();
                        let mut footnote_end = group_end;
                        let mut previous_footnote_row = None;
                        while footnote_end < visible_blocks.len()
                            && spacing_for(footnote_end).callout_anchor == Some(callout_anchor)
                            && spacing_for(footnote_end).footnote_anchor == Some(footnote_anchor)
                        {
                            let footnote_spacing = spacing_for(footnote_end);
                            let entity = visible_blocks[footnote_end].entity.clone();
                            let row = div()
                                .w_full()
                                .flex_shrink_0()
                                .mt(px(footnote_row_top_gap(previous_footnote_row, d.block_gap)))
                                .child(entity.clone());
                            let row_editor = editor.clone();
                            let entity_id = entity.entity_id();
                            let row =
                                row.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                                    let _ = row_editor.update(cx, |editor, cx| {
                                        editor.on_block_context_menu_mouse_down(
                                            entity_id, event, window, cx,
                                        );
                                    });
                                });
                            footnote_children.push(row.into_any_element());
                            previous_footnote_row = Some(footnote_spacing);
                            footnote_end += 1;
                        }

                        group_children.push(
                            div()
                                .w_full()
                                .flex_shrink_0()
                                .mt(px(callout_row_top_gap(
                                    previous_callout_row,
                                    row_spacing,
                                    d,
                                )))
                                .child(footnote_group_shell(footnote_children, &theme, d))
                                .into_any_element(),
                        );
                        previous_callout_row = Some(spacing_for(footnote_end - 1));
                        group_end = footnote_end;
                        continue;
                    }

                    let entity = visible_blocks[group_end].entity.clone();
                    let row = div()
                        .w_full()
                        .flex_shrink_0()
                        .mt(px(callout_row_top_gap(
                            previous_callout_row,
                            row_spacing,
                            d,
                        )))
                        .child(entity.clone());
                    let row_editor = editor.clone();
                    let entity_id = entity.entity_id();
                    let row = row.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        let _ = row_editor.update(cx, |editor, cx| {
                            editor.on_block_context_menu_mouse_down(entity_id, event, window, cx);
                        });
                    });
                    group_children.push(row.into_any_element());
                    previous_callout_row = Some(row_spacing);
                    group_end += 1;
                }

                let (accent, background) = callout_colors(callout_variant, &theme);
                row_starts.push(index);
                row_top_gaps.push(top_gap);
                row_elements.push(
                    div()
                        .w(px(centered_width))
                        .max_w(relative(1.0))
                        .flex_shrink_0()
                        .mt(px(top_gap))
                        .flex()
                        .flex_col()
                        .gap(px(0.0))
                        .px(px(d.callout_padding_x))
                        .py(px(d.callout_padding_y))
                        .rounded(px(d.callout_radius))
                        .border_l(px(d.callout_border_width))
                        .border_color(accent)
                        .bg(background)
                        .children(group_children)
                        .into_any_element(),
                );
                previous_row_spacing = Some(spacing_for(group_end - 1));
                index = group_end;
                continue;
            }

            if let Some(footnote_anchor) = first_spacing.footnote_anchor {
                let mut group_children = Vec::new();
                let mut group_end = index;
                let mut previous_footnote_row = None;
                while group_end < visible_blocks.len()
                    && spacing_for(group_end).footnote_anchor == Some(footnote_anchor)
                {
                    let row_spacing = spacing_for(group_end);
                    let entity = visible_blocks[group_end].entity.clone();
                    let row = div()
                        .w_full()
                        .flex_shrink_0()
                        .mt(px(footnote_row_top_gap(previous_footnote_row, d.block_gap)))
                        .child(entity.clone());
                    let row_editor = editor.clone();
                    let entity_id = entity.entity_id();
                    let row = row.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                        let _ = row_editor.update(cx, |editor, cx| {
                            editor.on_block_context_menu_mouse_down(entity_id, event, window, cx);
                        });
                    });
                    group_children.push(row.into_any_element());
                    previous_footnote_row = Some(row_spacing);
                    group_end += 1;
                }

                row_starts.push(index);
                row_top_gaps.push(top_gap);
                row_elements.push(
                    div()
                        .w(px(centered_width))
                        .max_w(relative(1.0))
                        .flex_shrink_0()
                        .mt(px(top_gap))
                        .child(footnote_group_shell(group_children, &theme, d))
                        .into_any_element(),
                );
                previous_row_spacing = Some(spacing_for(group_end - 1));
                index = group_end;
                continue;
            }

            let entity = first_visible.entity.clone();
            let row = div()
                .w(px(centered_width))
                .max_w(relative(1.0))
                .flex_shrink_0()
                .mt(px(top_gap))
                .child(entity.clone());
            let row_editor = editor.clone();
            let entity_id = entity.entity_id();
            let row = row.on_mouse_down(MouseButton::Right, move |event, window, cx| {
                let _ = row_editor.update(cx, |editor, cx| {
                    editor.on_block_context_menu_mouse_down(entity_id, event, window, cx);
                });
            });
            row_starts.push(index);
            row_top_gaps.push(top_gap);
            row_elements.push(row.into_any_element());
            previous_row_spacing = Some(first_spacing);
            index += 1;
        }

        // The focused row is always kept mounted so its caret is not blurred; a
        // table cell maps to its containing table block's row.
        let focus_row = self
            .focused_edit_target_entity_id(window, cx)
            .and_then(|id| {
                self.document.visible_index_for_entity_id(id).or_else(|| {
                    self.table_cell_binding(id).and_then(|binding| {
                        self.document
                            .visible_index_for_entity_id(binding.table_block.entity_id())
                    })
                })
            })
            .map(|visible_index| {
                row_starts
                    .partition_point(|&start| start <= visible_index)
                    .saturating_sub(1)
            });

        // A row's first block keys its cached footprint.
        let row_first_ids: Vec<EntityId> = row_starts
            .iter()
            .map(|&start| visible_blocks[start].entity.entity_id())
            .collect();

        // On a structural edit the row indices no longer match last frame, so the
        // cache refresh below is skipped; its block-keyed entries still hold.
        let structural_change = visible_blocks.len() != self.prev_visible_block_ids.len()
            || visible_blocks
                .iter()
                .zip(&self.prev_visible_block_ids)
                .any(|(visible, prev)| visible.entity.entity_id() != *prev);
        if structural_change {
            self.prev_visible_block_ids = visible_blocks
                .iter()
                .map(|v| v.entity.entity_id())
                .collect();
        }

        // Snapshot the estimates before incorporating last frame's measured
        // row tops. If heights above the visible anchor settle this frame, the
        // virtual top spacer changes by the same amount; compensate the scroll
        // offset so content already under the pointer does not jump.
        let estimate = d.block_min_height.max(1.0);
        let old_strides: Vec<f32> = row_first_ids
            .iter()
            .map(|id| self.row_stride_cache.get(id).copied().unwrap_or(estimate))
            .collect();

        // A footprint only holds for the column it was measured at. The first
        // frame has no scroll bounds yet, so the column collapses to its 1px
        // floor and every block wraps a character per line; keeping those
        // measurements would leave the document permanently mis-sized.
        let width_changed = self.row_stride_width != Some(centered_width);
        if width_changed {
            self.row_stride_cache.clear();
            self.row_stride_width = Some(centered_width);
        }

        // The scroll container records every mounted child's layout bounds, so
        // adjacent tops differ by exactly one row's footprint whatever the row
        // holds. Caching those differences, not raw positions, keeps the window
        // stable while scrolling.
        if !structural_change && !width_changed {
            if let Some(prev) = self
                .prev_mounted_run
                .filter(|prev| self.mounted_run_is_addressable(*prev))
            {
                let prev_end = prev.row_end.min(row_first_ids.len());
                for row in prev.row_start..prev_end.saturating_sub(1) {
                    let child = prev.child_base + row - prev.row_start;
                    if let (Some(bounds), Some(next_bounds)) = (
                        self.scroll_handle.bounds_for_item(child),
                        self.scroll_handle.bounds_for_item(child + 1),
                    ) {
                        let stride = f32::from(next_bounds.top() - bounds.top());
                        if stride > 0.0 && stride.is_finite() {
                            self.row_stride_cache.insert(row_first_ids[row], stride);
                        }
                    }
                }
            }
        }

        // Unmeasured rows use the minimum block height: a lower bound, so the
        // window over-mounts rather than ever landing on a spacer.
        let strides: Vec<f32> = row_first_ids
            .iter()
            .map(|id| self.row_stride_cache.get(id).copied().unwrap_or(estimate))
            .collect();

        if !structural_change && !explicit_scroll_in_progress {
            let anchor_delta =
                Self::row_stride_anchor_delta(&old_strides, &strides, current_scroll_y);
            if anchor_delta.is_finite() && anchor_delta.abs() > 0.5 {
                current_scroll_y = (current_scroll_y + anchor_delta).max(0.0);
                let mut offset = self.scroll_handle.offset();
                offset.y = px(-current_scroll_y);
                self.scroll_handle.set_offset(offset);
            }
        }

        // Bound the cache against block churn, only when it outgrows the live rows.
        if self.row_stride_cache.len() > row_first_ids.len().saturating_mul(2) {
            let live: std::collections::HashSet<EntityId> = row_first_ids.iter().copied().collect();
            self.row_stride_cache.retain(|id, _| live.contains(id));
        }

        let render_window = Self::rendered_window(
            &strides,
            current_scroll_y,
            viewport_height,
            RENDER_OVERDRAW_PX,
            focus_row,
        );
        let island = render_window.focus_island;
        let island_before_run = island.is_some_and(|island| island.row < render_window.run_start);
        // A mounted row re-applies its own `mt`, which the preceding stride
        // already covered, so every spacer sheds the gap of the row it precedes.
        let spacer_before = |row: usize, height: f32| -> f32 {
            match row_top_gaps.get(row) {
                Some(gap) => (height - gap).max(0.0),
                None => height,
            }
        };
        let mut block_rows: Vec<AnyElement> =
            Vec::with_capacity(render_window.run_end - render_window.run_start + 4);
        let push_spacer = |rows: &mut Vec<AnyElement>, height: f32| {
            if height > 0.5 {
                rows.push(
                    div()
                        .w_full()
                        .flex_shrink_0()
                        .h(px(height))
                        .into_any_element(),
                );
            }
        };

        let mut row_elements: Vec<Option<AnyElement>> =
            row_elements.into_iter().map(Some).collect();
        let mut take_row = |rows: &mut Vec<AnyElement>, row: usize| {
            if let Some(element) = row_elements.get_mut(row).and_then(Option::take) {
                rows.push(element);
            }
        };

        if let Some(island) = island.filter(|_| island_before_run) {
            push_spacer(&mut block_rows, spacer_before(island.row, island.lead_h));
            take_row(&mut block_rows, island.row);
        }
        push_spacer(
            &mut block_rows,
            spacer_before(render_window.run_start, render_window.top_h),
        );
        let run_child_base = block_rows.len();
        for row in render_window.run_start..render_window.run_end {
            take_row(&mut block_rows, row);
        }
        if let Some(island) = island.filter(|_| !island_before_run) {
            push_spacer(&mut block_rows, spacer_before(island.row, island.lead_h));
            take_row(&mut block_rows, island.row);
        }
        push_spacer(&mut block_rows, render_window.bottom_h);
        // Next frame reads the run's footprints back at these child indices, and
        // re-checks `child_count` before trusting them.
        self.prev_mounted_run = Some(MountedRun {
            row_start: render_window.run_start,
            row_end: render_window.run_end,
            child_base: run_child_base,
            child_count: block_rows.len(),
        });

        let scroll_content = div()
            .id("editor-scroll-inner")
            .size_full()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .flex()
            .flex_col()
            .items_center()
            .bg(theme.colors.editor_background)
            .overflow_y_scroll()
            .scrollbar_width(px(0.0))
            .track_scroll(&self.scroll_handle)
            .on_hover(cx.listener(Self::on_editor_hover))
            .capture_any_mouse_down(cx.listener(Self::on_editor_capture_mouse_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_editor_mouse_down))
            .on_mouse_move(cx.listener(Self::on_editor_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_editor_mouse_up))
            .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_editor_mouse_up))
            .on_scroll_wheel(cx.listener(Self::on_editor_scroll_wheel))
            .p(px(d.editor_padding))
            .pb(px(d.editor_padding
                + scroll_trigger_padding
                + scroll_beyond_bottom))
            .children(block_rows);
        let scroll_content = scroll_content.on_mouse_down(
            MouseButton::Right,
            cx.listener(Self::on_editor_context_menu_mouse_down),
        );

        let content_area = div()
            .id("editor-scroll")
            .w_full()
            .h_full()
            .flex_1()
            .min_w(px(0.0))
            .min_h(px(0.0))
            .overflow_hidden()
            .bg(theme.colors.editor_background)
            .relative()
            .child(scroll_content);

        let content_area = if show_custom_scrollbar {
            let scrollbar_editor = editor.clone();
            let track_origin_y = f32::from(viewport_bounds.origin.y);
            content_area.child(
                div()
                    .id("editor-scrollbar-thumb")
                    .absolute()
                    .occlude()
                    .top(px(thumb_top))
                    .right(px(d.scrollbar_right))
                    .w(px(d.scrollbar_width))
                    .h(px(thumb_height))
                    .rounded(px(999.0))
                    .bg(theme.colors.scrollbar_thumb)
                    .cursor_pointer()
                    .on_hover(cx.listener(Self::on_editor_hover))
                    .on_mouse_down(MouseButton::Left, move |event, _window, cx| {
                        let pointer_offset_y =
                            f32::from(event.position.y) - track_origin_y - thumb_top;
                        let _ = scrollbar_editor.update(cx, |editor, cx| {
                            cx.stop_propagation();
                            editor.start_scrollbar_drag(
                                pointer_offset_y,
                                track_height,
                                thumb_height,
                                max_scroll_y,
                                cx,
                            );
                        });
                    })
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |_thumb_bounds, _, window, _| {
                                window.on_mouse_event({
                                    let editor = editor.clone();
                                    move |_event: &MouseUpEvent, phase, _window, cx| {
                                        if !phase.bubble() {
                                            return;
                                        }
                                        let _ = editor.update(cx, |editor, cx| {
                                            editor.end_scrollbar_drag(cx);
                                        });
                                    }
                                });

                                window.on_mouse_event({
                                    let editor = editor.clone();
                                    move |event: &MouseMoveEvent, phase, _window, cx| {
                                        if !phase.bubble() || !event.dragging() {
                                            return;
                                        }

                                        let pointer_y_in_track =
                                            f32::from(event.position.y) - track_origin_y;
                                        let _ = editor.update(cx, |editor, cx| {
                                            editor.update_scrollbar_drag(pointer_y_in_track, cx);
                                        });
                                    }
                                });
                            },
                        )
                        .size_full(),
                    ),
            )
        } else {
            content_area
        };

        // Repaint when the Cmd/Ctrl follow modifier toggles so a hovered link's
        // hand cursor updates without moving the pointer. `ModifiersChanged` is
        // dispatched along the focused element's path to the root, and this root
        // is an ancestor of every block, so one listener here covers a link in any
        // block while editing. Gated to the secondary modifier so Shift during
        // selection does not repaint.
        let follow_modifier_active = window.modifiers().secondary();
        let bounds_editor = cx.entity().downgrade();

        let base = div()
            .w_full()
            .min_w(px(0.0))
            .h_full()
            .flex()
            .flex_col()
            .relative()
            .overflow_hidden()
            .bg(theme.colors.editor_background)
            .font(editor_text_font())
            .on_prepaint(move |bounds, _window, cx| {
                let _ = bounds_editor.update(cx, |editor, _cx| {
                    editor.root_bounds = bounds;
                });
            })
            .on_modifiers_changed(move |event, window, _| {
                if event.modifiers.secondary() != follow_modifier_active {
                    window.refresh();
                }
            })
            .capture_action(cx.listener(Self::on_copy_capture))
            .capture_action(cx.listener(Self::on_cut_capture))
            .capture_action(cx.listener(Self::on_delete_capture))
            .capture_action(cx.listener(Self::on_delete_back_capture))
            .on_action(cx.listener(Self::on_undo))
            .on_action(cx.listener(Self::on_redo))
            .on_action(cx.listener(Self::on_toggle_view_mode_action))
            .on_action(cx.listener(Self::on_page_up))
            .on_action(cx.listener(Self::on_page_down))
            .on_action(cx.listener(Self::on_jump_to_top))
            .on_action(cx.listener(Self::on_jump_to_bottom))
            .on_action(cx.listener(Self::on_dismiss_transient_ui));
        let base = base.child(content_area);
        let mut base = if let Some(context_menu) = self.render_context_menu_overlay(&theme, cx) {
            base.child(context_menu)
        } else {
            base
        };
        if let Some(table_dialog) = self.render_table_insert_dialog_overlay(&theme, cx) {
            base = base.child(table_dialog);
        }
        if let Some(enlarged) = self.render_enlarged_block_overlay(&theme, window, cx) {
            base = base.child(enlarged);
        }
        base
    }
}
