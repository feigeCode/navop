use super::surface::MarkdownSurfaceKey;
use super::{MarkdownEditor, MarkdownEditorError};
use gpui::{Context, ScrollStrategy, Window};
use markdown_source::{BlockMoveDirection, SourceSelection, SourceTransaction, TableCellAddress};
use markdown_source::{InlineFormat, ListFormat};

impl MarkdownEditor {
    pub fn active_block(&self) -> Option<markdown_source::SourceNodeId> {
        self.active_block
    }

    pub fn active_table_cell(&self) -> Option<TableCellAddress> {
        self.active_table_cell
    }

    pub fn focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.active_block.is_none()
            && let Some(block) = self.history.document().blocks.first()
        {
            self.activate_block(block.id, window, cx);
            return;
        }
        let _ = self.focus_surface(self.active_surface_key(), window, cx);
    }

    pub fn activate_block(
        &mut self,
        block_id: markdown_source::SourceNodeId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some((block_index, block)) = self
            .history
            .document()
            .blocks
            .iter()
            .enumerate()
            .find(|(_, block)| block.id == block_id)
        else {
            return false;
        };
        let cursor = block
            .content_range
            .as_ref()
            .map_or(block.source_range.start, |range| range.start);
        self.active_block = Some(block_id);
        self.active_table_cell = None;
        self.sync_projection(cursor, window, cx);
        if self.uses_virtual_layout() {
            self.block_scroll
                .scroll_to_item(block_index, ScrollStrategy::Center);
        }
        self.focus_surface(self.active_surface_key(), window, cx)
    }

    pub fn deactivate_block(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let key = self.active_surface_key();
        if key == MarkdownSurfaceKey::Empty
            && self.active_block.is_none()
            && self.active_table_cell.is_none()
        {
            return;
        }
        let selection = self.surface_selection(key, cx).unwrap_or_default();
        self.pending_newline = None;
        self.collapse_surface_projection(key, selection, window, cx);
        let _ = self.set_active_surface(MarkdownSurfaceKey::Empty);
        self.collapse_surface_projection(MarkdownSurfaceKey::Empty, selection, window, cx);
        window.blur();
        cx.notify();
    }

    pub fn select_all(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.input.update(cx, |input, cx| {
            input.set_selected_range(0..input.value().len(), false, window, cx);
        });
    }

    pub fn toggle_inline_format(
        &mut self,
        format: InlineFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let selection = self.source_selection(cx);
        let range = selection.anchor.min(selection.head)..selection.anchor.max(selection.head);
        let transaction = self
            .history
            .document()
            .toggle_inline_format(range.clone(), format)?;
        let (opening, closing) = match format {
            InlineFormat::Bold => ("**", "**"),
            InlineFormat::Italic => ("_", "_"),
            InlineFormat::Underline => ("<u>", "</u>"),
            InlineFormat::Strike => ("~~", "~~"),
            InlineFormat::Code => ("`", "`"),
        };
        let replacement = &transaction.edits[0].replacement;
        let wrapped = replacement.starts_with(opening) && replacement.ends_with(closing);
        let opening_len = wrapped.then_some(opening.len()).unwrap_or_default();
        let closing_len = wrapped.then_some(closing.len()).unwrap_or_default();
        let selection_after = SourceSelection {
            anchor: range.start + opening_len,
            head: range.start + replacement.len().saturating_sub(closing_len),
        };
        self.apply_editor_transaction(transaction, selection_after, window, cx)
    }

    pub fn set_active_heading(
        &mut self,
        level: Option<u8>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self.history.document().set_block_heading(block_id, level)?;
        let cursor = transaction.edits[0].range.start + transaction.edits[0].replacement.len();
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn toggle_active_list(
        &mut self,
        format: ListFormat,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self
            .history
            .document()
            .toggle_list_format(block_id, format)?;
        let cursor = transaction.edits[0].range.start + transaction.edits[0].replacement.len();
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn duplicate_active_block(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self.history.document().duplicate_block(block_id)?;
        let cursor = transaction.edits[0].range.start + transaction.edits[0].replacement.len();
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn delete_active_block(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self.history.document().delete_block(block_id)?;
        let cursor = transaction.edits[0].range.start;
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn move_active_block(
        &mut self,
        direction: BlockMoveDirection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self.history.document().move_block(block_id, direction)?;
        let edit = &transaction.edits[0];
        let cursor = match direction {
            BlockMoveDirection::Up => edit.range.start,
            BlockMoveDirection::Down => edit.range.start + edit.replacement.len(),
        };
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn toggle_active_blockquote(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self.history.document().toggle_blockquote(block_id)?;
        let cursor = transaction.edits[0].range.start;
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub fn toggle_active_code_fence(
        &mut self,
        language: Option<&str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self
            .history
            .document()
            .toggle_code_fence(block_id, language)?;
        let cursor = transaction.edits[0].range.start;
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    pub(super) fn split_active_block(
        &mut self,
        source_offset: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let Some(block_id) = self.active_block else {
            return Ok(false);
        };
        let transaction = self
            .history
            .document()
            .split_block(block_id, source_offset)?;
        let cursor = source_offset + transaction.edits[0].replacement.len();
        self.apply_block_transaction(transaction, cursor, window, cx)
    }

    fn apply_block_transaction(
        &mut self,
        mut transaction: SourceTransaction,
        cursor: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        transaction.selection_before = self.source_selection(cx);
        transaction.selection_after = SourceSelection {
            anchor: cursor,
            head: cursor,
        };
        self.history.apply(&transaction)?;
        self.dirty = true;
        self.sync_projection(cursor.min(self.source().len()), window, cx);
        self.emit_changed(cx);
        Ok(true)
    }

    fn apply_editor_transaction(
        &mut self,
        mut transaction: SourceTransaction,
        selection_after: SourceSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        transaction.selection_before = self.source_selection(cx);
        transaction.selection_after = selection_after;
        self.history.apply(&transaction)?;
        self.dirty = true;
        self.sync_selection(selection_after, window, cx);
        self.emit_changed(cx);
        Ok(true)
    }
}
