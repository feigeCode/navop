use super::surface::MarkdownSurfaceKey;
use super::{MarkdownEditor, MarkdownEditorError};
use gpui::{Context, Window};
use markdown_source::{
    SourceBlockKind, SourceSelection, SourceTransaction, TableAlignment, TableCellAddress,
    TableInsertPosition,
};

impl MarkdownEditor {
    pub fn insert_active_table_row(
        &mut self,
        position: TableInsertPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let visible_row = address.row.saturating_sub(1);
        let target_row = visible_row + usize::from(position == TableInsertPosition::After);
        let transaction = self
            .history
            .document()
            .insert_table_row(address, position)?;
        self.apply_table_structure(transaction, target_row, address.column, window, cx)
    }

    pub fn delete_active_table_row(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let target_row = address.row.saturating_sub(1);
        let transaction = self.history.document().delete_table_row(address)?;
        self.apply_table_structure(transaction, target_row, address.column, window, cx)
    }

    pub fn insert_active_table_column(
        &mut self,
        position: TableInsertPosition,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let target_column = address.column + usize::from(position == TableInsertPosition::After);
        let transaction = self
            .history
            .document()
            .insert_table_column(address, position)?;
        self.apply_table_structure(
            transaction,
            address.row.saturating_sub(1),
            target_column,
            window,
            cx,
        )
    }

    pub fn delete_active_table_column(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let transaction = self.history.document().delete_table_column(address)?;
        self.apply_table_structure(
            transaction,
            address.row.saturating_sub(1),
            address.column,
            window,
            cx,
        )
    }

    pub fn align_active_table_column(
        &mut self,
        alignment: TableAlignment,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let transaction = self
            .history
            .document()
            .set_table_column_alignment(address, alignment)?;
        self.apply_table_structure(
            transaction,
            address.row.saturating_sub(1),
            address.column,
            window,
            cx,
        )
    }

    pub fn resize_active_table(
        &mut self,
        visible_rows: usize,
        columns: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(address) = self.active_table_cell else {
            return Ok(false);
        };
        let transaction =
            self.history
                .document()
                .resize_table(address.block_id, visible_rows, columns)?;
        self.apply_table_structure(
            transaction,
            address.row.saturating_sub(1).min(visible_rows - 1),
            address.column.min(columns - 1),
            window,
            cx,
        )
    }

    fn apply_table_structure(
        &mut self,
        mut transaction: SourceTransaction,
        target_row: usize,
        target_column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let table_start = transaction.edits[0].range.start;
        transaction.selection_before = self.source_selection(cx);
        transaction.selection_after = SourceSelection {
            anchor: table_start,
            head: table_start,
        };
        self.history.apply(&transaction)?;
        self.dirty = true;
        self.reactivate_table_cell(table_start, target_row, target_column, window, cx);
        self.emit_changed(cx);
        Ok(true)
    }

    fn reactivate_table_cell(
        &mut self,
        table_start: usize,
        target_row: usize,
        target_column: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document = self.history.document();
        let Some(block) = document.block_at(table_start) else {
            self.sync_projection(table_start, window, cx);
            return;
        };
        let SourceBlockKind::Table(table) = &block.kind else {
            self.sync_projection(table_start, window, cx);
            return;
        };
        let visible_rows = table.rows.len().saturating_sub(1).max(1);
        let row = target_row.min(visible_rows - 1);
        let source_row = if row == 0 { 0 } else { row + 1 };
        let columns = table
            .rows
            .get(source_row)
            .map(|row| row.cells.len())
            .unwrap_or(1)
            .max(1);
        let address = TableCellAddress {
            block_id: block.id,
            row: source_row,
            column: target_column.min(columns - 1),
        };
        let cursor = table.rows[source_row].cells[address.column]
            .content_range
            .end;
        self.sync_table_cell(address, cursor, window, cx);
        let _ = self.focus_surface(MarkdownSurfaceKey::table_cell(address), window, cx);
    }
}
