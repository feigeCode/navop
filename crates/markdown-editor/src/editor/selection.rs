//! Editor-level selection spanning multiple rendered blocks.

use std::collections::HashMap;
use std::ops::Range;

use gpui::*;

use super::{
    CrossBlockDrag, CrossBlockSelection, CrossBlockSelectionEndpoint, Editor, SourceTargetMapping,
    UndoSelectionSnapshot, ViewMode,
};
use crate::components::{
    Block, BlockKind, Copy, Cut, Delete, DeleteBack, UndoCaptureKind,
    serialize_table_markdown_lines,
};

/// Cross-block selection with endpoints ordered by visible block position.
#[derive(Clone, Copy)]
struct NormalizedCrossBlockSelection {
    start: CrossBlockSelectionEndpoint,
    end: CrossBlockSelectionEndpoint,
    start_index: usize,
    end_index: usize,
    reversed: bool,
}

impl Editor {
    fn clear_cross_block_selection_visuals(&mut self, cx: &mut Context<Self>) -> bool {
        let mut changed = false;
        for visible in self.document.visible_blocks().to_vec() {
            visible.entity.update(cx, |block, cx| {
                if block.editor_selection_range.take().is_some() {
                    changed = true;
                    cx.notify();
                }
            });
        }
        changed
    }

    pub(super) fn clear_cross_block_selection(&mut self, cx: &mut Context<Self>) {
        let had_selection = self.cross_block_selection.take().is_some();
        self.cross_block_drag = None;
        let changed_visuals = self.clear_cross_block_selection_visuals(cx);
        let changed = had_selection || changed_visuals;
        if changed {
            cx.notify();
        }
    }

    fn begin_cross_block_drag_at_point(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let had_selection = self.cross_block_selection.take().is_some();
        let changed_visuals = self.clear_cross_block_selection_visuals(cx);
        let changed = had_selection || changed_visuals;
        self.cross_block_drag = self
            .cross_block_endpoint_for_point(position, cx)
            .map(|anchor| CrossBlockDrag { anchor });
        if changed {
            cx.notify();
        }
    }

    pub(super) fn on_editor_capture_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if event.button != MouseButton::Left {
            cx.propagate();
            return;
        }

        if self.view_mode != ViewMode::Rendered {
            cx.propagate();
            return;
        }

        self.rendered_select_all_cycle = None;
        self.begin_cross_block_drag_at_point(event.position, cx);
        cx.propagate();
    }

    pub(super) fn on_editor_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !event.dragging() {
            return;
        }
        let Some(drag) = self.cross_block_drag else {
            return;
        };
        let Some(focus) = self.cross_block_endpoint_for_point(event.position, cx) else {
            return;
        };

        if self.cross_block_selection.is_none() && drag.anchor.entity_id == focus.entity_id {
            return;
        }

        let selection = CrossBlockSelection {
            anchor: drag.anchor,
            focus,
        };
        if self.cross_block_selection_is_empty(selection) {
            self.cross_block_selection = None;
        } else {
            self.cross_block_selection = Some(selection);
        }
        self.sync_cross_block_selection_visuals(cx);
        cx.notify();
    }

    pub(super) fn on_editor_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cross_block_drag = None;
        self.end_block_pointer_selection_sessions(cx);
    }

    pub(super) fn on_copy_capture(
        &mut self,
        _: &Copy,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(markdown) = self.cross_block_selected_markdown(cx) else {
            cx.propagate();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(markdown));
        cx.stop_propagation();
    }

    pub(super) fn on_cut_capture(&mut self, _: &Cut, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(markdown) = self.cross_block_selected_markdown(cx) else {
            cx.propagate();
            return;
        };
        cx.write_to_clipboard(ClipboardItem::new_string(markdown));
        self.delete_cross_block_selection(cx);
        cx.stop_propagation();
    }

    pub(super) fn on_delete_capture(
        &mut self,
        _: &Delete,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.delete_cross_block_selection(cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
    }

    pub(super) fn on_delete_back_capture(
        &mut self,
        _: &DeleteBack,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.delete_cross_block_selection(cx) {
            cx.propagate();
            return;
        }
        cx.stop_propagation();
    }

    fn rendered_document_is_fully_selected(&self, cx: &App) -> bool {
        let visible = self.document.visible_blocks().to_vec();
        let Some(first) = visible.first() else {
            return false;
        };
        let Some(last) = visible.last() else {
            return false;
        };
        let Some(selection) = self.cross_block_selection else {
            return false;
        };
        let last_len = last.entity.read(cx).visible_len();
        selection.anchor
            == CrossBlockSelectionEndpoint {
                entity_id: first.entity.entity_id(),
                offset: 0,
            }
            && selection.focus
                == CrossBlockSelectionEndpoint {
                    entity_id: last.entity.entity_id(),
                    offset: last_len,
                }
    }

    fn select_focused_block_text_for_rendered_select_all(
        &mut self,
        block: Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        self.clear_cross_block_selection(cx);
        self.end_block_pointer_selection_sessions(cx);
        self.clear_table_axis_preview(cx);
        self.clear_table_axis_selection(cx);
        block.update(cx, |block, cx| {
            let len = block.visible_len();
            block.selected_range = 0..len;
            block.selection_reversed = false;
            block.marked_range = None;
            block.vertical_motion_x = None;
            block.cursor_blink_epoch = std::time::Instant::now();
            cx.notify();
        });
        self.active_entity_id = Some(block.entity_id());
        cx.notify();
    }

    fn select_all_rendered_document(&mut self, cx: &mut Context<Self>) {
        if self.rendered_document_is_fully_selected(cx) {
            return;
        }

        let visible = self.document.visible_blocks().to_vec();
        let Some(first) = visible.first() else {
            return;
        };
        let Some(last) = visible.last() else {
            return;
        };
        let first_id = first.entity.entity_id();
        let last_id = last.entity.entity_id();
        let last_len = last.entity.read(cx).visible_len();

        self.end_block_pointer_selection_sessions(cx);
        self.dismiss_contextual_overlays(cx);
        self.clear_table_axis_preview(cx);
        self.clear_table_axis_selection(cx);
        for visible in &visible {
            visible.entity.update(cx, |block, cx| {
                let cursor = block.cursor_offset();
                let collapsed = cursor..cursor;
                if block.selected_range != collapsed {
                    block.selected_range = collapsed;
                    cx.notify();
                }
            });
        }

        self.cross_block_drag = None;
        self.cross_block_selection = Some(CrossBlockSelection {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: first_id,
                offset: 0,
            },
            focus: CrossBlockSelectionEndpoint {
                entity_id: last_id,
                offset: last_len,
            },
        });
        self.sync_cross_block_selection_visuals(cx);
        cx.notify();
    }

    pub(super) fn on_rendered_select_all_press(
        &mut self,
        block: Entity<Block>,
        cx: &mut Context<Self>,
    ) {
        if self.view_mode != ViewMode::Rendered {
            self.rendered_select_all_cycle = None;
            return;
        }

        let now = std::time::Instant::now();
        let block_id = block.entity_id();
        let count = match self.rendered_select_all_cycle {
            Some(cycle)
                if cycle.entity_id == block_id
                    && now.duration_since(cycle.last_pressed_at)
                        <= Self::RENDERED_SELECT_ALL_CYCLE_WINDOW =>
            {
                cycle.count.saturating_add(1)
            }
            _ => 1,
        }
        .min(3);

        self.rendered_select_all_cycle = Some(super::RenderedSelectAllCycle {
            entity_id: block_id,
            count,
            last_pressed_at: now,
        });

        if count == 1 {
            self.select_focused_block_text_for_rendered_select_all(block, cx);
        } else {
            self.select_all_rendered_document(cx);
        }
    }

    pub(super) fn cross_block_source_selection_snapshot(
        &self,
        cx: &App,
    ) -> Option<UndoSelectionSnapshot> {
        let normalized = self.normalized_cross_block_selection(cx)?;
        let range = self.cross_block_source_range_for_normalized(normalized, cx)?;
        Some(UndoSelectionSnapshot {
            range,
            reversed: normalized.reversed,
        })
    }

    pub(super) fn apply_cross_block_selection_snapshot_if_possible(
        &mut self,
        snapshot: &UndoSelectionSnapshot,
        cx: &mut Context<Self>,
    ) -> bool {
        if snapshot.range.is_empty() {
            return false;
        }

        let mappings = self.build_source_target_mappings(cx);
        let Some(start) = self.endpoint_for_source_offset(snapshot.range.start, &mappings, cx)
        else {
            return false;
        };
        let Some(end) = self.endpoint_for_source_offset(snapshot.range.end, &mappings, cx) else {
            return false;
        };
        let Some(start_index) = self.document.visible_index_for_entity_id(start.entity_id) else {
            return false;
        };
        let Some(end_index) = self.document.visible_index_for_entity_id(end.entity_id) else {
            return false;
        };
        if start_index == end_index {
            return false;
        }

        self.cross_block_selection = Some(if snapshot.reversed {
            CrossBlockSelection {
                anchor: end,
                focus: start,
            }
        } else {
            CrossBlockSelection {
                anchor: start,
                focus: end,
            }
        });
        self.cross_block_drag = None;
        self.sync_cross_block_selection_visuals(cx);
        let focus = if snapshot.reversed { start } else { end };
        self.focus_block(focus.entity_id);
        cx.notify();
        true
    }

    fn cross_block_endpoint_for_point(
        &self,
        position: Point<Pixels>,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let mut previous: Option<(Entity<Block>, Bounds<Pixels>)> = None;
        for visible in self.document.visible_blocks() {
            let entity = visible.entity.clone();
            let bounds = entity.read(cx).last_bounds;
            let Some(bounds) = bounds else {
                continue;
            };

            if position.y < bounds.top() {
                if let Some((previous, _)) = previous {
                    let offset = previous.read(cx).visible_len();
                    return Some(CrossBlockSelectionEndpoint {
                        entity_id: previous.entity_id(),
                        offset,
                    });
                }
                return Some(CrossBlockSelectionEndpoint {
                    entity_id: entity.entity_id(),
                    offset: 0,
                });
            }

            if position.y <= bounds.bottom() {
                let offset = entity.read(cx).index_for_mouse_position(position);
                return Some(CrossBlockSelectionEndpoint {
                    entity_id: entity.entity_id(),
                    offset,
                });
            }

            previous = Some((entity, bounds));
        }

        previous.map(|(entity, _)| CrossBlockSelectionEndpoint {
            entity_id: entity.entity_id(),
            offset: entity.read(cx).visible_len(),
        })
    }

    fn cross_block_selection_is_empty(&self, selection: CrossBlockSelection) -> bool {
        let Some(anchor_index) = self
            .document
            .visible_index_for_entity_id(selection.anchor.entity_id)
        else {
            return true;
        };
        let Some(focus_index) = self
            .document
            .visible_index_for_entity_id(selection.focus.entity_id)
        else {
            return true;
        };
        anchor_index == focus_index && selection.anchor.offset == selection.focus.offset
    }

    fn normalized_cross_block_selection(&self, cx: &App) -> Option<NormalizedCrossBlockSelection> {
        let selection = self.cross_block_selection?;
        let anchor = self.clamp_cross_block_endpoint(selection.anchor, cx)?;
        let focus = self.clamp_cross_block_endpoint(selection.focus, cx)?;
        let anchor_index = self
            .document
            .visible_index_for_entity_id(anchor.entity_id)?;
        let focus_index = self.document.visible_index_for_entity_id(focus.entity_id)?;
        let reversed = focus_index < anchor_index
            || (focus_index == anchor_index && focus.offset < anchor.offset);
        let (start, end, start_index, end_index) = if reversed {
            (focus, anchor, focus_index, anchor_index)
        } else {
            (anchor, focus, anchor_index, focus_index)
        };
        if start_index == end_index && start.offset == end.offset {
            return None;
        }
        Some(NormalizedCrossBlockSelection {
            start,
            end,
            start_index,
            end_index,
            reversed,
        })
    }

    fn clamp_cross_block_endpoint(
        &self,
        endpoint: CrossBlockSelectionEndpoint,
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let entity = self.document.block_entity_by_id(endpoint.entity_id)?;
        let len = entity.read(cx).visible_len();
        Some(CrossBlockSelectionEndpoint {
            entity_id: endpoint.entity_id,
            offset: endpoint.offset.min(len),
        })
    }

    fn sync_cross_block_selection_visuals(&mut self, cx: &mut Context<Self>) {
        let normalized = self.normalized_cross_block_selection(cx);
        let visible_blocks = self.document.visible_blocks().to_vec();
        for (index, visible) in visible_blocks.into_iter().enumerate() {
            let next_range = normalized.and_then(|selection| {
                if index < selection.start_index || index > selection.end_index {
                    return None;
                }
                let block = visible.entity.read(cx);
                let len = block.visible_len();
                let range = if selection.start_index == selection.end_index {
                    selection.start.offset.min(len)..selection.end.offset.min(len)
                } else if index == selection.start_index {
                    selection.start.offset.min(len)..len
                } else if index == selection.end_index {
                    0..selection.end.offset.min(len)
                } else {
                    0..len
                };
                (!range.is_empty()).then_some(range)
            });

            visible.entity.update(cx, |block, cx| {
                if block.editor_selection_range != next_range {
                    block.editor_selection_range = next_range.clone();
                    cx.notify();
                }
            });
        }
    }

    fn source_mapping_by_entity_id(&self, cx: &App) -> HashMap<EntityId, SourceTargetMapping> {
        self.build_source_target_mappings(cx)
            .into_iter()
            .map(|mapping| (mapping.entity.entity_id(), mapping))
            .collect()
    }

    fn endpoint_source_offset(
        &self,
        endpoint: CrossBlockSelectionEndpoint,
        mappings: &HashMap<EntityId, SourceTargetMapping>,
        cx: &App,
    ) -> Option<usize> {
        let mapping = mappings.get(&endpoint.entity_id)?;
        let block = mapping.entity.read(cx);
        let visible_len = block.visible_len();
        if endpoint.offset == 0 {
            return Some(mapping.full_source_range.start);
        }
        if endpoint.offset >= visible_len {
            return Some(mapping.full_source_range.end);
        }
        let markdown_offset = block
            .current_range_to_markdown_range(endpoint.offset..endpoint.offset)
            .start;
        let max_content = mapping.content_to_source.len().saturating_sub(1);
        Some(
            mapping.full_source_range.start
                + mapping.content_to_source[markdown_offset.min(max_content)],
        )
    }

    fn endpoint_for_source_offset(
        &self,
        offset: usize,
        mappings: &[SourceTargetMapping],
        cx: &App,
    ) -> Option<CrossBlockSelectionEndpoint> {
        let mapping = mappings.iter().min_by_key(|mapping| {
            Self::source_offset_distance(&mapping.full_source_range, offset)
        })?;
        let local = if offset <= mapping.full_source_range.start {
            0
        } else if offset >= mapping.full_source_range.end {
            mapping.full_source_range.len()
        } else {
            offset - mapping.full_source_range.start
        };
        let content_offset =
            mapping.source_to_content[local.min(mapping.source_to_content.len().saturating_sub(1))];
        let block = mapping.entity.read(cx);
        Some(CrossBlockSelectionEndpoint {
            entity_id: mapping.entity.entity_id(),
            offset: block.markdown_offset_to_current_offset(content_offset),
        })
    }

    fn cross_block_source_range_for_normalized(
        &self,
        selection: NormalizedCrossBlockSelection,
        cx: &App,
    ) -> Option<Range<usize>> {
        let (mapping_list, block_ranges) = self.build_source_target_mappings_with_block_ranges(cx);
        let mappings: HashMap<EntityId, SourceTargetMapping> = mapping_list
            .into_iter()
            .map(|mapping| (mapping.entity.entity_id(), mapping))
            .collect();
        let visible = self.document.visible_blocks();

        // Resolve an endpoint to a source offset. Atomic blocks (tables, etc.)
        // carry no per-block text mapping, so endpoint_source_offset returns
        // None for them; fall back to the block's own source span, picking the
        // side that keeps the block inside the selection.
        let endpoint_offset =
            |endpoint: CrossBlockSelectionEndpoint, index: usize, at_end: bool| -> Option<usize> {
                if let Some(offset) = self.endpoint_source_offset(endpoint, &mappings, cx) {
                    return Some(offset);
                }
                let entity = visible.get(index)?.entity.clone();
                let range = block_ranges.get(&entity.entity_id())?;
                Some(if at_end { range.end } else { range.start })
            };

        let start = endpoint_offset(selection.start, selection.start_index, false)?;
        let end = endpoint_offset(selection.end, selection.end_index, true)?;
        let (mut lo, mut hi) = (start.min(end), start.max(end));

        // Endpoint offsets can never point *after* a zero-visible-len (atomic)
        // block, so a table at the trailing boundary of the selection would be
        // left behind. Union in the full source range of every atomic block
        // whose visible index falls inside the selection so it is removed whole.
        for index in selection.start_index..=selection.end_index {
            let entity = visible.get(index)?.entity.clone();
            if entity.read(cx).visible_len() == 0 {
                if let Some(range) = block_ranges.get(&entity.entity_id()) {
                    lo = lo.min(range.start);
                    hi = hi.max(range.end);
                }
            }
        }
        Some(lo..hi)
    }

    fn rebuild_after_cross_block_source_edit(&mut self, source: String, cx: &mut Context<Self>) {
        match self.view_mode {
            ViewMode::Rendered => {
                let roots = Self::build_rendered_roots(cx, &source);
                self.document.replace_roots(roots, cx);
                self.rebuild_table_runtimes(cx);
                self.rebuild_image_runtimes(cx);
            }
            ViewMode::Source => {
                let block = Self::new_block(
                    cx,
                    crate::components::BlockRecord::paragraph(source.clone()),
                );
                block.update(cx, |block, _cx| block.set_source_document_mode());
                self.document.replace_roots(vec![block], cx);
                self.table_cells.clear();
            }
        }
    }

    fn apply_marked_source_range(&mut self, source_range: Range<usize>, cx: &mut Context<Self>) {
        if source_range.is_empty() {
            return;
        }
        let mappings = self.build_source_target_mappings(cx);
        let Some(start) = self.endpoint_for_source_offset(source_range.start, &mappings, cx) else {
            return;
        };
        let Some(end) = self.endpoint_for_source_offset(source_range.end, &mappings, cx) else {
            return;
        };
        if start.entity_id != end.entity_id {
            return;
        }
        let Some(block) = self.focusable_entity_by_id(start.entity_id) else {
            return;
        };
        block.update(cx, |block, cx| {
            block.marked_range = Some(start.offset.min(end.offset)..start.offset.max(end.offset));
            cx.notify();
        });
    }

    pub(super) fn replace_cross_block_selection_with_text(
        &mut self,
        new_text: &str,
        selected_range_relative: Option<Range<usize>>,
        mark_inserted_text: bool,
        undo_kind: UndoCaptureKind,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(selection) = self.normalized_cross_block_selection(cx) else {
            return false;
        };
        let Some(source_range) = self.cross_block_source_range_for_normalized(selection, cx) else {
            return false;
        };

        self.prepare_undo_capture(undo_kind, cx);
        let mut source = self.current_document_source(cx);
        let start = source_range.start.min(source.len());
        let end = source_range.end.min(source.len());
        source.replace_range(start..end, new_text);
        self.cross_block_selection = None;
        self.cross_block_drag = None;

        let inserted_start = start;
        let inserted_end = inserted_start + new_text.len();
        let selected_source_range = selected_range_relative
            .map(|relative| {
                inserted_start + relative.start.min(new_text.len())
                    ..inserted_start + relative.end.min(new_text.len())
            })
            .unwrap_or(inserted_end..inserted_end);
        let marked_source_range =
            (mark_inserted_text && !new_text.is_empty()).then_some(inserted_start..inserted_end);

        self.rebuild_after_cross_block_source_edit(source, cx);
        self.apply_selection_snapshot_in_current_mode(
            &UndoSelectionSnapshot {
                range: selected_source_range,
                reversed: false,
            },
            cx,
        );
        if let Some(marked_source_range) = marked_source_range {
            self.apply_marked_source_range(marked_source_range, cx);
        }
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        self.sync_cross_block_selection_visuals(cx);
        self.request_active_block_scroll_into_view(cx);
        cx.notify();
        true
    }

    pub(super) fn cross_block_selected_markdown(&self, cx: &App) -> Option<String> {
        let selection = self.normalized_cross_block_selection(cx)?;
        let source = self.current_document_source(cx);
        let mappings = self.source_mapping_by_entity_id(cx);
        let visible = self.document.visible_blocks();

        // Join blocks with the same spacing the document serializer uses
        // (collect_root_markdown_lines): a blank line between blocks, but tight
        // list items stay on consecutive lines. A flat single-newline join used
        // to silently fuse separate paragraphs on paste, and once setext pairs
        // are recognized it could even fabricate a heading from two paragraphs.
        let mut result = String::new();
        let mut wrote_chunk = false;
        let mut pending_empty = 0usize;
        let mut previous_was_list_item = false;

        for index in selection.start_index..=selection.end_index {
            let entity = visible.get(index)?.entity.clone();
            let block = entity.read(cx);
            let len = block.visible_len();
            let range = if selection.start_index == selection.end_index {
                selection.start.offset.min(len)..selection.end.offset.min(len)
            } else if index == selection.start_index {
                selection.start.offset.min(len)..len
            } else if index == selection.end_index {
                0..selection.end.offset.min(len)
            } else {
                0..len
            };
            let full_block = range.start == 0
                && range.end == len
                && (selection.start_index != selection.end_index || len > 0);
            // Cut deletes any atomic block covered by a multi-block selection
            // (see cross_block_source_range_for_normalized), so the clipboard
            // must serialize those blocks too, including boundary ones, not
            // just interior. Otherwise cut would drop a table from the clipboard
            // that it nonetheless removed from the document.
            let include_atomic = len == 0 && selection.start_index != selection.end_index;
            if range.is_empty() && !include_atomic {
                continue;
            }

            // Empty paragraphs are blank-line separators, not content: defer
            // them so the gap between real blocks is reproduced as a blank line
            // rather than collapsed. Atomic content (tables, separators, images)
            // is len 0 too but is not an empty paragraph, so it still serializes.
            if (full_block || include_atomic) && Editor::is_empty_root_paragraph(block) {
                pending_empty += 1;
                continue;
            }

            let current_is_list_item = block.kind().is_list_item();
            if wrote_chunk {
                let separator_lines = if previous_was_list_item && current_is_list_item {
                    pending_empty
                } else {
                    pending_empty + 1
                };
                result.push_str(&"\n".repeat(separator_lines + 1));
            }
            result.push_str(&self.markdown_chunk_for_block(
                &entity,
                range,
                full_block || include_atomic,
                &source,
                &mappings,
                cx,
            ));
            wrote_chunk = true;
            pending_empty = 0;
            previous_was_list_item = current_is_list_item;
        }

        Some(result)
    }

    fn markdown_chunk_for_block(
        &self,
        entity: &Entity<Block>,
        range: Range<usize>,
        full_block: bool,
        source: &str,
        mappings: &HashMap<EntityId, SourceTargetMapping>,
        cx: &App,
    ) -> String {
        if let Some(mapping) = mappings.get(&entity.entity_id()) {
            if full_block {
                return source[mapping.full_source_range.clone()].to_string();
            }

            let start = self
                .endpoint_source_offset(
                    CrossBlockSelectionEndpoint {
                        entity_id: entity.entity_id(),
                        offset: range.start,
                    },
                    mappings,
                    cx,
                )
                .unwrap_or(mapping.full_source_range.start);
            let end = self
                .endpoint_source_offset(
                    CrossBlockSelectionEndpoint {
                        entity_id: entity.entity_id(),
                        offset: range.end,
                    },
                    mappings,
                    cx,
                )
                .unwrap_or(mapping.full_source_range.end);
            return source[start.min(end)..start.max(end)].to_string();
        }

        let block = entity.read(cx);
        if full_block {
            return match block.kind() {
                BlockKind::Table => block
                    .record
                    .table
                    .as_ref()
                    .map(serialize_table_markdown_lines)
                    .map(|lines| lines.join("\n"))
                    .unwrap_or_default(),
                _ => block
                    .record
                    .markdown_line(block.render_depth, block.list_ordinal),
            };
        }

        let markdown = block.record.title.serialize_markdown();
        let markdown_range = block.current_range_to_markdown_range(range);
        markdown
            .get(markdown_range)
            .map(ToOwned::to_owned)
            .unwrap_or_default()
    }

    fn delete_cross_block_selection(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(selection) = self.normalized_cross_block_selection(cx) else {
            return false;
        };
        let Some(source_range) = self.cross_block_source_range_for_normalized(selection, cx) else {
            return false;
        };
        if source_range.is_empty() {
            return false;
        }

        self.prepare_undo_capture(UndoCaptureKind::NonCoalescible, cx);
        let mut source = self.current_document_source(cx);
        let start = source_range.start.min(source.len());
        let end = source_range.end.min(source.len());
        source.replace_range(start..end, "");
        self.cross_block_selection = None;
        self.cross_block_drag = None;

        self.rebuild_after_cross_block_source_edit(source, cx);

        self.apply_selection_snapshot_in_current_mode(
            &UndoSelectionSnapshot {
                range: start..start,
                reversed: false,
            },
            cx,
        );
        self.mark_dirty(cx);
        self.finalize_pending_undo_capture(cx);
        self.sync_table_axis_visuals(cx);
        self.dismiss_contextual_overlays(cx);
        self.sync_cross_block_selection_visuals(cx);
        cx.notify();
        true
    }
}

#[cfg(test)]
mod tests {
    use gpui::{AppContext, Bounds, Context, TestAppContext, point, px, size};

    use super::{CrossBlockSelection, CrossBlockSelectionEndpoint, Editor};
    use crate::components::{Cut, Delete, DeleteBack, Undo, UndoCaptureKind};
    use crate::theme::ThemeManager;

    fn init_editor_test_app(cx: &mut TestAppContext) {
        cx.update(|cx| {
            ThemeManager::init(cx);
            crate::components::init(cx);
        });
    }

    fn redraw(cx: &mut gpui::VisualTestContext) {
        cx.update(|window, cx| window.draw(cx).clear(cx));
        cx.run_until_parked();
    }

    fn set_selection(
        editor: &mut Editor,
        start_index: usize,
        start_offset: usize,
        end_index: usize,
        end_offset: usize,
        cx: &mut Context<Editor>,
    ) {
        let visible = editor.document.visible_blocks().to_vec();
        let start = visible[start_index].entity.entity_id();
        let end = visible[end_index].entity.entity_id();
        editor.cross_block_selection = Some(CrossBlockSelection {
            anchor: CrossBlockSelectionEndpoint {
                entity_id: start,
                offset: start_offset,
            },
            focus: CrossBlockSelectionEndpoint {
                entity_id: end,
                offset: end_offset,
            },
        });
        editor.sync_cross_block_selection_visuals(cx);
    }

    fn assign_visible_block_bounds(editor: &mut Editor, cx: &mut Context<Editor>) {
        for (index, visible) in editor
            .document
            .visible_blocks()
            .to_vec()
            .into_iter()
            .enumerate()
        {
            visible.entity.update(cx, move |block, _cx| {
                block.last_bounds = Some(Bounds::new(
                    point(px(0.0), px(index as f32 * 32.0)),
                    size(px(400.0), px(24.0)),
                ));
            });
        }
    }

    #[test]
    fn mouse_down_starts_cross_block_drag_after_clearing_old_selection() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor =
            cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            assign_visible_block_bounds(editor, cx);
            set_selection(editor, 0, 0, 2, 2, cx);
            assert!(editor.cross_block_selection.is_some());
            assert!(
                editor
                    .document
                    .visible_blocks()
                    .iter()
                    .any(|visible| visible.entity.read(cx).editor_selection_range.is_some())
            );

            editor.begin_cross_block_drag_at_point(point(px(8.0), px(4.0)), cx);

            assert!(editor.cross_block_selection.is_none());
            assert!(editor.cross_block_drag.is_some());
            assert!(
                editor
                    .document
                    .visible_blocks()
                    .iter()
                    .all(|visible| visible.entity.read(cx).editor_selection_range.is_none())
            );
        });
        cx.quit();
    }

    #[test]
    fn typing_replaces_cross_block_selection_with_plain_text() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor =
            cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            set_selection(editor, 0, 2, 2, 2, cx);
            assert!(editor.replace_cross_block_selection_with_text(
                "X",
                None,
                false,
                UndoCaptureKind::CoalescibleText,
                cx
            ));

            assert_eq!(editor.document.markdown_text(cx), "alXmma");
            assert!(editor.cross_block_selection.is_none());
            assert!(editor.cross_block_drag.is_none());
            let block = editor.document.visible_blocks()[0].entity.read(cx);
            assert_eq!(block.selected_range, 3..3);
            assert!(block.marked_range.is_none());
        });
        cx.quit();
    }

    #[test]
    fn ime_composition_replaces_cross_block_selection_and_marks_inserted_text() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor =
            cx.new(|cx| Editor::from_markdown(cx, "alpha\n\nbeta\n\ngamma".to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            set_selection(editor, 0, 2, 2, 2, cx);
            assert!(editor.replace_cross_block_selection_with_text(
                "ni",
                Some(2..2),
                true,
                UndoCaptureKind::CoalescibleText,
                cx
            ));

            assert_eq!(editor.document.markdown_text(cx), "alnimma");
            let block = editor.document.visible_blocks()[0].entity.read(cx);
            assert_eq!(block.selected_range, 4..4);
            assert_eq!(block.marked_range, Some(2..4));
            assert!(block.editor_selection_range.is_none());
        });
        cx.quit();
    }

    #[test]
    fn cross_block_selection_marks_visual_ranges_and_copies_markdown() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor = cx.new(|cx| {
            Editor::from_markdown(
                cx,
                "alpha **bold**\n\n- item\n\n![alt](image.png)".to_string(),
                None,
            )
        });

        editor.update(&mut cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let end_len = visible[2].entity.read(cx).visible_len();
            set_selection(editor, 0, 0, 2, end_len, cx);

            assert_eq!(
                editor.cross_block_selected_markdown(cx).as_deref(),
                Some("alpha **bold**\n\n- item\n\n![alt](image.png)")
            );
            for visible in visible {
                let block = visible.entity.read(cx);
                assert_eq!(block.editor_selection_range, Some(0..block.visible_len()));
            }
        });
        cx.quit();
    }

    #[gpui::test]
    fn cross_block_cut_writes_markdown_deletes_range_and_undo_restores(cx: &mut TestAppContext) {
        init_editor_test_app(cx);
        let original = "alpha\n\nbeta\n\ngamma";
        let (editor, cx) = cx.add_window_view({
            let original = original.to_string();
            move |_window, cx| Editor::from_markdown(cx, original.clone(), None)
        });
        redraw(cx);

        editor.update(cx, |editor, cx| {
            set_selection(editor, 0, 2, 2, 2, cx);
            assert_eq!(
                editor.cross_block_selected_markdown(cx).as_deref(),
                Some("pha\n\nbeta\n\nga")
            );
        });
        redraw(cx);

        cx.dispatch_action(Cut);
        redraw(cx);

        assert_eq!(
            cx.read_from_clipboard()
                .and_then(|item| item.text())
                .as_deref(),
            Some("pha\n\nbeta\n\nga")
        );
        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.document.markdown_text(cx)),
            "almma"
        );

        cx.dispatch_action(Undo);
        redraw(cx);

        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.document.markdown_text(cx)),
            original
        );
        editor.read_with(cx, |editor, cx| {
            assert_eq!(
                editor.cross_block_selected_markdown(cx).as_deref(),
                Some("pha\n\nbeta\n\nga")
            );
        });
    }

    enum FootnoteDeleteAction {
        Forward,
        Backward,
    }

    fn assert_unicode_footnote_selection_delete(
        cx: &mut TestAppContext,
        id_prefix: char,
        action: FootnoteDeleteAction,
    ) {
        init_editor_test_app(cx);
        let id = format!("{id_prefix}a");
        let original = format!("[^{id}] tail\n\n[^{id}]: body");
        let (editor, cx) = cx.add_window_view({
            let original = original.clone();
            move |_window, cx| Editor::from_markdown(cx, original, None)
        });
        redraw(cx);

        let reference = editor.update(cx, |editor, cx| {
            let reference = editor.document.visible_blocks()[0].entity.clone();
            reference.update(cx, |block, _cx| {
                assert_eq!(block.display_text(), format!("[^{id}] tail"));
            });
            editor.focus_block(reference.entity_id());
            reference
        });
        redraw(cx);

        reference.update(cx, |block, _cx| {
            let display = block.display_text();
            let selection_start =
                display.find(id_prefix).expect("expanded footnote id") + id_prefix.len_utf8();
            let selection_end = display.find("tail").expect("trailing text");
            block.selected_range = selection_start..selection_end;
        });

        match action {
            FootnoteDeleteAction::Forward => cx.dispatch_action(Delete),
            FootnoteDeleteAction::Backward => cx.dispatch_action(DeleteBack),
        }
        redraw(cx);

        assert_eq!(
            editor.read_with(cx, |editor, cx| editor.document.markdown_text(cx)),
            format!("[\\^{id_prefix}tail\n\n[^{id}]: body")
        );
        editor.read_with(cx, |editor, cx| {
            assert_eq!(
                editor.document.visible_blocks()[0]
                    .entity
                    .read(cx)
                    .display_text(),
                format!("[^{id_prefix}tail")
            );
        });
    }

    #[gpui::test]
    fn delete_back_selection_across_unicode_footnote_run_does_not_panic(cx: &mut TestAppContext) {
        assert_unicode_footnote_selection_delete(cx, '€', FootnoteDeleteAction::Backward);
    }

    #[gpui::test]
    fn delete_selection_across_unicode_footnote_run_does_not_panic(cx: &mut TestAppContext) {
        assert_unicode_footnote_selection_delete(cx, '😀', FootnoteDeleteAction::Forward);
    }

    const TABLE_DOC: &str = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |\n\ngamma";

    #[test]
    fn delete_selection_spanning_table_removes_table() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let end_len = visible[2].entity.read(cx).visible_len();
            // The table sits in the interior of the selection.
            set_selection(editor, 0, 0, 2, end_len, cx);
            assert!(editor.delete_cross_block_selection(cx));

            let text = editor.document.markdown_text(cx);
            assert!(!text.contains('|'), "table should be gone: {text:?}");
            assert!(!text.contains("alpha"));
            assert!(!text.contains("gamma"));
        });
        cx.quit();
    }

    #[test]
    fn delete_selection_with_trailing_table_removes_table() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            assert_eq!(editor.document.visible_blocks().len(), 3);
            // Selection ends at the start of the table block: the previously
            // broken case where the trailing atomic block was left behind.
            set_selection(editor, 0, 0, 1, 0, cx);
            assert!(editor.delete_cross_block_selection(cx));

            // The table is removed in full; only `gamma` survives (deleting from
            // the document start leaves the table's trailing blank line, which
            // reparses to leading empty paragraphs, harmless and trimmable).
            let text = editor.document.markdown_text(cx);
            assert!(
                !text.contains('|'),
                "trailing table should be gone: {text:?}"
            );
            assert_eq!(text.trim(), "gamma");
        });
        cx.quit();
    }

    #[test]
    fn delete_selection_of_only_table_removes_just_the_table() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let alpha_len = visible[0].entity.read(cx).visible_len();
            // Drag from the end of the paragraph above onto the table: only the
            // table is removed, and re-parse normalizes the spacing.
            set_selection(editor, 0, alpha_len, 1, 0, cx);
            assert!(editor.delete_cross_block_selection(cx));

            assert_eq!(editor.document.markdown_text(cx), "alpha\n\ngamma");
        });
        cx.quit();
    }

    #[test]
    fn cut_selection_including_table_serializes_and_deletes_it() {
        // Exercise cut's two halves directly (the clipboard markdown and the
        // deleted source range) rather than dispatching the action, keeping this
        // a focused unit test of the cut logic.
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let editor = cx.new(|cx| Editor::from_markdown(cx, TABLE_DOC.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let end_len = visible[2].entity.read(cx).visible_len();
            set_selection(editor, 0, 0, 2, end_len, cx);

            // The clipboard markdown serializes the full table, matching what
            // delete removes; otherwise cut would drop it from the clipboard.
            let markdown = editor.cross_block_selected_markdown(cx).unwrap();
            assert!(markdown.contains("| a | b |"), "clipboard: {markdown:?}");
            assert!(markdown.contains("| 1 | 2 |"), "clipboard: {markdown:?}");
            assert!(markdown.contains("alpha") && markdown.contains("gamma"));

            assert!(editor.delete_cross_block_selection(cx));
            assert!(
                !editor.document.markdown_text(cx).contains('|'),
                "document should no longer contain the table"
            );
        });
        cx.quit();
    }

    #[test]
    fn delete_selection_spanning_code_block_removes_it() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        // Code blocks edit their raw text, so they are deletable as an ordinary
        // text range; this documents that visible_len-based behavior.
        let doc = "alpha\n\n```\ncode\n```\n\ngamma";
        let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let end_len = visible[2].entity.read(cx).visible_len();
            set_selection(editor, 0, 0, 2, end_len, cx);
            assert!(editor.delete_cross_block_selection(cx));

            let text = editor.document.markdown_text(cx);
            assert!(
                !text.contains("code"),
                "code block should be gone: {text:?}"
            );
        });
        cx.quit();
    }

    #[test]
    fn delete_selection_ending_on_trailing_empty_paragraph_removes_table() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let doc = "alpha\n\n| a | b |\n| --- | --- |\n| 1 | 2 |";
        let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            // Append a trailing empty paragraph, exactly as inserting a table at
            // the end of a document does. Ending the selection on it used to
            // abort deletion because empty roots had no source span.
            let empty =
                Editor::new_block(cx, crate::components::BlockRecord::paragraph(String::new()));
            let index = editor.document.root_count();
            editor
                .document
                .insert_blocks_at(None, index, vec![empty], cx);

            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            let alpha_len = visible[0].entity.read(cx).visible_len();
            // From the end of `alpha` onto the trailing empty paragraph.
            set_selection(editor, 0, alpha_len, 2, 0, cx);
            assert!(editor.delete_cross_block_selection(cx));

            let text = editor.document.markdown_text(cx);
            assert!(!text.contains('|'), "table should be gone: {text:?}");
            assert_eq!(text.trim(), "alpha");
        });
        cx.quit();
    }

    #[test]
    fn delete_selection_starting_on_empty_paragraph_removes_table() {
        let mut cx = TestAppContext::single();
        init_editor_test_app(&mut cx);
        let doc = "| a | b |\n| --- | --- |\n| 1 | 2 |\n\ngamma";
        let editor = cx.new(|cx| Editor::from_markdown(cx, doc.to_string(), None));

        editor.update(&mut cx, |editor, cx| {
            // Prepend a leading empty paragraph; starting the highlight on it used
            // to abort deletion (the user's "drag up from the text below into an
            // empty block above the table" case).
            let empty =
                Editor::new_block(cx, crate::components::BlockRecord::paragraph(String::new()));
            editor.document.insert_blocks_at(None, 0, vec![empty], cx);

            let visible = editor.document.visible_blocks().to_vec();
            assert_eq!(visible.len(), 3);
            // From the empty paragraph (index 0) to the start of `gamma`.
            set_selection(editor, 0, 0, 2, 0, cx);
            assert!(editor.delete_cross_block_selection(cx));

            let text = editor.document.markdown_text(cx);
            assert!(!text.contains('|'), "table should be gone: {text:?}");
            assert_eq!(text.trim(), "gamma");
        });
        cx.quit();
    }
}
