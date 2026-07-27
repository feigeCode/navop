use super::MarkdownEditor;
use crate::MarkdownEditorError;
use gpui::{Context, Window};
use markdown_source::{SourceNodeId, SourceSelection};
use std::ops::Range;

impl MarkdownEditor {
    pub fn set_code_fence_language(
        &mut self,
        block_id: SourceNodeId,
        language: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Result<bool, MarkdownEditorError> {
        let selection = self.source_selection(cx);
        let transaction = self
            .history
            .document()
            .set_code_fence_language(block_id, language)?;
        let edit = &transaction.edits[0];
        if self.history.document().source[edit.range.clone()] == edit.replacement {
            return Ok(false);
        }
        let selection_after = SourceSelection {
            anchor: offset_after_edit(selection.anchor, &edit.range, edit.replacement.len()),
            head: offset_after_edit(selection.head, &edit.range, edit.replacement.len()),
        };
        self.apply_editor_transaction(transaction, selection_after, window, cx)
    }
}

fn offset_after_edit(offset: usize, range: &Range<usize>, replacement_len: usize) -> usize {
    if offset < range.start || (offset == range.start && !range.is_empty()) {
        return offset;
    }
    if offset >= range.end {
        return range.start + replacement_len + offset - range.end;
    }
    range.start + replacement_len
}
