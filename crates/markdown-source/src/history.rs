use crate::{
    PatchError, SourceEdit, SourceEditOrigin, SourceMarkdownDocument, SourceSelection,
    SourceTransaction,
};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceEditTransaction {
    pub document: SourceMarkdownDocument,
    pub forward_edits: Vec<SourceEdit>,
    pub inverse_edits: Vec<SourceEdit>,
    pub origin: SourceEditOrigin,
    pub selection_before: SourceSelection,
    pub selection_after: SourceSelection,
    pub parse_scope: SourceParseScope,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SourceParseScope {
    SingleBlock,
    FullDocument,
}

#[derive(Debug, Clone)]
pub struct SourceHistory {
    document: SourceMarkdownDocument,
    undo: Vec<SourceHistoryEntry>,
    redo: Vec<SourceHistoryEntry>,
}

#[derive(Debug, Clone)]
struct SourceHistoryEntry {
    forward_edits: Vec<SourceEdit>,
    inverse_edits: Vec<SourceEdit>,
    selection_before: SourceSelection,
    selection_after: SourceSelection,
}

impl SourceHistory {
    pub fn new(document: SourceMarkdownDocument) -> Self {
        Self {
            document,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    pub fn document(&self) -> &SourceMarkdownDocument {
        &self.document
    }

    pub fn apply(&mut self, transaction: &SourceTransaction) -> Result<(), PatchError> {
        let applied = self.document.apply_transaction(transaction)?;
        let SourceEditTransaction {
            document,
            forward_edits,
            inverse_edits,
            selection_before,
            selection_after,
            ..
        } = applied;
        self.document = document;
        self.undo.push(SourceHistoryEntry {
            forward_edits,
            inverse_edits,
            selection_before,
            selection_after,
        });
        self.redo.clear();
        Ok(())
    }

    pub fn undo(&mut self) -> Result<Option<SourceSelection>, PatchError> {
        let Some(previous) = self.undo.pop() else {
            return Ok(None);
        };
        let transaction = transaction_for(
            &previous.inverse_edits,
            self.document.revision,
            SourceEditOrigin::Undo,
        );
        let applied = self.document.apply_transaction(&transaction)?;
        self.document = applied.document;
        let selection = previous.selection_before;
        self.redo.push(previous);
        Ok(Some(selection))
    }

    pub fn redo(&mut self) -> Result<Option<SourceSelection>, PatchError> {
        let Some(next) = self.redo.pop() else {
            return Ok(None);
        };
        let transaction = transaction_for(
            &next.forward_edits,
            self.document.revision,
            SourceEditOrigin::Redo,
        );
        let applied = self.document.apply_transaction(&transaction)?;
        self.document = applied.document;
        let selection = next.selection_after;
        self.undo.push(next);
        Ok(Some(selection))
    }
}

fn transaction_for(
    edits: &[SourceEdit],
    revision: u64,
    origin: SourceEditOrigin,
) -> SourceTransaction {
    let edits = edits
        .iter()
        .cloned()
        .map(|mut edit| {
            edit.expected_revision = revision;
            edit
        })
        .collect::<Vec<_>>();
    SourceTransaction {
        allowed_ranges: edits.iter().map(|edit| edit.range.clone()).collect(),
        edits,
        origin,
        selection_before: SourceSelection::default(),
        selection_after: SourceSelection::default(),
    }
}
