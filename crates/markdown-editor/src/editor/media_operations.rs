use super::{MarkdownEditor, MarkdownEditorError};
use gpui::{Context, Pixels, Point, Window};
use markdown_source::{SourceSelection, TableCellAddress};

impl MarkdownEditor {
    pub fn activate_table_cell(
        &mut self,
        address: TableCellAddress,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Ok(cell) = self.history.document().table_cell(address) else {
            return false;
        };
        self.sync_table_cell(address, cell.content_range.end, window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        true
    }

    pub fn activate_table_cell_at(
        &mut self,
        address: TableCellAddress,
        position: Point<Pixels>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.activate_table_cell(address, window, cx) {
            return false;
        }
        cx.defer_in(window, move |editor, window, cx| {
            if editor.active_table_cell != Some(address) {
                return;
            }
            let offset = editor
                .input
                .read(cx)
                .offset_for_position(position)
                .unwrap_or_else(|| editor.input.read(cx).value().len());
            editor.input.update(cx, |input, cx| {
                input.set_selected_range(offset..offset, false, window, cx);
            });
        });
        true
    }

    pub fn delete_active_image(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(node_id) = self.projection.active_inline else {
            return false;
        };
        let selection_before = self.source_selection(cx);
        let Ok(mut transaction) = self.history.document().delete_image(node_id) else {
            return false;
        };
        let cursor = transaction.edits[0].range.start;
        transaction.selection_before = selection_before;
        transaction.selection_after = SourceSelection {
            anchor: cursor,
            head: cursor,
        };
        if self.history.apply(&transaction).is_err() {
            return false;
        }
        self.dirty = true;
        self.sync_projection(cursor, window, cx);
        self.emit_changed(cx);
        true
    }

    pub fn edit_active_image(
        &mut self,
        alt: impl Into<String>,
        destination: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(node_id) = self.projection.active_inline else {
            return Ok(false);
        };
        let mut transaction = self
            .history
            .document()
            .edit_image(node_id, alt, destination)?;
        let cursor = transaction.edits[0].range.start + transaction.edits[0].replacement.len();
        transaction.selection_before = self.source_selection(cx);
        transaction.selection_after = SourceSelection {
            anchor: cursor,
            head: cursor,
        };
        self.history.apply(&transaction)?;
        self.dirty = true;
        self.sync_projection(cursor, window, cx);
        self.emit_changed(cx);
        Ok(true)
    }

    pub fn active_image_properties(&self) -> Option<(String, String)> {
        let image = self
            .history
            .document()
            .image_map(self.projection.active_inline?)
            .ok()?;
        let source = &self.history.document().source;
        Some((
            source[image.alt_range.clone()].to_owned(),
            source[image.destination_range.clone()].to_owned(),
        ))
    }

    pub fn set_active_image_property_values(
        &self,
        alt: impl Into<String>,
        destination: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.image_alt_input
            .update(cx, |input, cx| input.set_value(alt.into(), window, cx));
        self.image_destination_input.update(cx, |input, cx| {
            input.set_value(destination.into(), window, cx);
        });
    }

    pub fn save_active_image_properties(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let alt = self.image_alt_input.read(cx).value().to_string();
        let destination = self.image_destination_input.read(cx).value().to_string();
        self.edit_active_image(alt, destination, window, cx)
    }

    pub fn edit_table_cell(
        &mut self,
        address: TableCellAddress,
        replacement: impl Into<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<(), MarkdownEditorError> {
        let mut transaction = self
            .history
            .document()
            .edit_table_cell(address, replacement)?;
        let cursor = transaction.edits[0].range.start + transaction.edits[0].replacement.len();
        transaction.selection_before = self.source_selection(cx);
        transaction.selection_after = SourceSelection {
            anchor: cursor,
            head: cursor,
        };
        self.history.apply(&transaction)?;
        self.dirty = true;
        self.sync_projection(cursor, window, cx);
        self.emit_changed(cx);
        Ok(())
    }

    pub fn clear_active_table_cell(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        self.edit_table_cell(address, "", window, cx)?;
        self.activate_table_cell(address, window, cx);
        Ok(true)
    }
}
