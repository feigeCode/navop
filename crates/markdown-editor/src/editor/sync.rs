use super::MarkdownEditor;
use super::projection_styles::projection_highlights;
use super::surface::{MarkdownSurfaceKey, SurfaceProjectionUpdate, projection_for};
use crate::{MarkdownEditorEvent, MarkdownProjection};
use gpui::{App, Context, Window};
#[cfg(test)]
use gpui_component::input::Position;
use markdown_source::{
    SourceBlockKind, SourceInlineKind, SourceNodeId, SourceSelection, TableCellAddress,
};

mod events;

impl MarkdownEditor {
    pub(super) fn refresh_projection_highlights(&self, cx: &mut Context<Self>) {
        let surfaces = self
            .surfaces
            .values()
            .map(|surface| (surface.input.clone(), surface.projection.clone()))
            .collect::<Vec<_>>();
        for (input, projection) in surfaces {
            let highlights = projection_highlights(&projection, &self.theme);
            input.update(cx, |input, cx| {
                input.set_text_highlights(highlights, cx);
            });
        }
    }

    pub(super) fn sync_projection(
        &mut self,
        source_cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (active_block, key) = self.block_surface_at(source_cursor);
        self.empty_surface_range = if key == MarkdownSurfaceKey::Empty {
            source_cursor..source_cursor
        } else {
            0..self.history.document().source.len()
        };
        self.active_block = active_block;
        self.active_table_cell = None;
        self.active_surface = Some(key);
        self.reconcile_surfaces(window, cx);
        self.sync_surface_selection(
            key,
            SourceSelection {
                anchor: source_cursor,
                head: source_cursor,
            },
            window,
            cx,
        );
        self.sync_image_property_inputs(window, cx);
    }

    pub(super) fn sync_table_cell(
        &mut self,
        address: TableCellAddress,
        source_cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.history.document().table_cell(address).is_err() {
            self.sync_projection(source_cursor, window, cx);
            return;
        }
        let key = MarkdownSurfaceKey::table_cell(address);
        self.empty_surface_range = 0..self.history.document().source.len();
        self.active_block = Some(address.block_id);
        self.active_table_cell = Some(address);
        self.active_surface = Some(key);
        self.reconcile_surfaces(window, cx);
        self.sync_surface_selection(
            key,
            SourceSelection {
                anchor: source_cursor,
                head: source_cursor,
            },
            window,
            cx,
        );
        self.sync_image_property_inputs(window, cx);
    }

    pub(super) fn resync_surface(
        &mut self,
        key: MarkdownSurfaceKey,
        source_cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(address) = key.table_address() {
            self.sync_table_cell(address, source_cursor, window, cx);
        } else {
            self.sync_projection(source_cursor, window, cx);
        }
    }

    pub(super) fn surface_selection(
        &self,
        key: MarkdownSurfaceKey,
        cx: &App,
    ) -> Option<SourceSelection> {
        let surface = self.surface(key)?;
        let range = surface.input.read(cx).selected_range();
        Some(SourceSelection {
            anchor: surface.projection.display_to_source(range.start),
            head: surface.projection.display_end_to_source(range.end),
        })
    }

    pub(super) fn active_inline_at_display(
        &self,
        key: MarkdownSurfaceKey,
        display_offset: usize,
    ) -> Option<SourceNodeId> {
        let projection = &self.surface(key)?.projection;
        let source_offset = projection.display_to_source(display_offset);
        self.inline_at_source(projection, source_offset)
            .or_else(|| {
                let end = projection.display_end_to_source(display_offset);
                previous_char_offset(&self.history.document().source, end)
                    .and_then(|offset| self.inline_at_source(projection, offset))
            })
    }

    pub(super) fn sync_surface_selection(
        &mut self,
        key: MarkdownSurfaceKey,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(base) = projection_for(
            self.history.document(),
            key,
            None,
            self.empty_surface_range.clone(),
        ) else {
            return;
        };
        let active_inline = self.inline_at_source(&base, selection.head);
        let Some(projection) = projection_for(
            self.history.document(),
            key,
            active_inline,
            self.empty_surface_range.clone(),
        ) else {
            return;
        };
        self.update_surface_projection(
            SurfaceProjectionUpdate {
                key,
                projection,
                selection: Some(selection),
            },
            window,
            cx,
        );
        cx.notify();
    }

    pub(super) fn collapse_surface_projection(
        &mut self,
        key: MarkdownSurfaceKey,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(projection) = projection_for(
            self.history.document(),
            key,
            None,
            self.empty_surface_range.clone(),
        ) else {
            return;
        };
        self.update_surface_projection(
            SurfaceProjectionUpdate {
                key,
                projection,
                selection: Some(selection),
            },
            window,
            cx,
        );
        cx.notify();
    }

    pub(super) fn sync_image_property_inputs(&self, window: &mut Window, cx: &mut Context<Self>) {
        let (alt, destination) = self.active_image_properties().unwrap_or_default();
        self.image_alt_input
            .update(cx, |input, cx| input.set_value(alt, window, cx));
        self.image_destination_input.update(cx, |input, cx| {
            input.set_value(destination, window, cx);
        });
    }

    pub(super) fn source_selection(&self, cx: &App) -> SourceSelection {
        self.surface_selection(self.active_surface_key(), cx)
            .unwrap_or_default()
    }

    pub(super) fn sync_selection(
        &mut self,
        selection: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(address) = self.active_table_cell {
            self.sync_table_cell(address, selection.head, window, cx);
        } else {
            self.sync_projection(selection.head, window, cx);
        }
        self.sync_surface_selection(self.active_surface_key(), selection, window, cx);
    }

    pub(super) fn emit_changed(&self, cx: &mut Context<Self>) {
        cx.emit(MarkdownEditorEvent::Changed {
            revision: self.revision(),
        });
    }

    fn block_surface_at(&self, source_cursor: usize) -> (Option<SourceNodeId>, MarkdownSurfaceKey) {
        let document = self.history.document();
        let block = document.block_at(source_cursor).or_else(|| {
            previous_char_offset(&document.source, source_cursor)
                .and_then(|offset| document.block_at(offset))
        });
        let block_id = block.map(|block| block.id);
        let key = block
            .as_ref()
            .filter(|block| !matches!(block.kind, SourceBlockKind::Table(_)))
            .map(|block| MarkdownSurfaceKey::block(block.id))
            .unwrap_or(MarkdownSurfaceKey::Empty);
        (block_id, key)
    }

    fn inline_at_source(
        &self,
        projection: &MarkdownProjection,
        source_offset: usize,
    ) -> Option<SourceNodeId> {
        self.history
            .document()
            .inline_node_at(source_offset)
            .filter(|node| !matches!(node.kind, SourceInlineKind::RawMarkdown))
            .filter(|node| {
                projection.source_range.start <= node.source_range.start
                    && node.source_range.end <= projection.source_range.end
            })
            .map(|node| node.id)
    }
}

#[cfg(test)]
fn position_for_offset(value: &str, offset: usize) -> Position {
    let prefix = &value[..crate::projection::floor_char_boundary(value, offset)];
    let line = prefix.bytes().filter(|byte| *byte == b'\n').count();
    let line_start = prefix.rfind('\n').map_or(0, |index| index + 1);
    Position::new(line as u32, prefix[line_start..].chars().count() as u32)
}

fn previous_char_offset(value: &str, offset: usize) -> Option<usize> {
    value
        .get(..offset)?
        .char_indices()
        .next_back()
        .map(|(index, _)| index)
}

#[cfg(test)]
mod tests;
