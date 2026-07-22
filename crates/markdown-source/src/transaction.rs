use crate::{PatchError, SourceMarkdownDocument, SourceSelection};
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEdit {
    pub range: Range<usize>,
    pub replacement: String,
    pub expected_revision: u64,
}

impl SourceEdit {
    pub fn new(
        range: Range<usize>,
        replacement: impl Into<String>,
        expected_revision: u64,
    ) -> Self {
        Self {
            range,
            replacement: replacement.into(),
            expected_revision,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceEditOrigin {
    SourceEditor,
    RichTextTyping,
    Formatting,
    InsertBlock,
    DeleteBlock,
    MoveBlock,
    TableCellEdit,
    ImageEdit,
    Paste,
    Undo,
    Redo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTransaction {
    pub edits: Vec<SourceEdit>,
    pub origin: SourceEditOrigin,
    pub allowed_ranges: Vec<Range<usize>>,
    pub selection_before: SourceSelection,
    pub selection_after: SourceSelection,
}

pub(crate) fn apply_transaction(
    document: &SourceMarkdownDocument,
    transaction: &SourceTransaction,
) -> Result<crate::SourceEditTransaction, PatchError> {
    validate_revisions(document, transaction)?;
    validate_allowed_ranges(transaction)?;
    let source = crate::apply_edits(&document.source, &transaction.edits)?;
    let inverse_edits = inverse_edits(document, &transaction.edits)?;
    let next_revision = document.revision.saturating_add(1);
    let (next, parse_scope) =
        crate::parser::reparse_after_edits(document, source, &transaction.edits, next_revision)
            .map_err(|error| PatchError::Parse(error.to_string()))?;
    Ok(crate::SourceEditTransaction {
        document: next,
        forward_edits: transaction.edits.clone(),
        inverse_edits,
        origin: transaction.origin,
        selection_before: transaction.selection_before,
        selection_after: transaction.selection_after,
        parse_scope,
    })
}

fn validate_revisions(
    document: &SourceMarkdownDocument,
    transaction: &SourceTransaction,
) -> Result<(), PatchError> {
    transaction
        .edits
        .iter()
        .all(|edit| edit.expected_revision == document.revision)
        .then_some(())
        .ok_or(PatchError::StaleRevision)
}

fn validate_allowed_ranges(transaction: &SourceTransaction) -> Result<(), PatchError> {
    if transaction.edits.is_empty() {
        return Ok(());
    }
    let valid = transaction.edits.iter().all(|edit| {
        transaction
            .allowed_ranges
            .iter()
            .any(|allowed| allowed.start <= edit.range.start && edit.range.end <= allowed.end)
    });
    valid.then_some(()).ok_or(PatchError::OutsideAllowedRanges)
}

fn inverse_edits(
    document: &SourceMarkdownDocument,
    edits: &[SourceEdit],
) -> Result<Vec<SourceEdit>, PatchError> {
    let mut ordered = edits.to_vec();
    ordered.sort_by_key(|edit| edit.range.start);
    let mut shift: isize = 0;
    let mut inverse = Vec::with_capacity(ordered.len());
    for edit in ordered {
        let start = edit
            .range
            .start
            .checked_add_signed(shift)
            .ok_or(PatchError::InvalidRange)?;
        let end = start + edit.replacement.len();
        inverse.push(SourceEdit::new(
            start..end,
            &document.source[edit.range.clone()],
            document.revision.saturating_add(1),
        ));
        shift += edit.replacement.len() as isize - edit.range.len() as isize;
    }
    Ok(inverse)
}
