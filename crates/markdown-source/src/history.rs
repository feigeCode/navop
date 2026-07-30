use crate::{
    PatchError, SourceEdit, SourceEditOrigin, SourceMarkdownDocument, SourceSelection,
    SourceTransaction,
};
use std::time::{Duration, Instant};

const TYPING_COALESCE_WINDOW: Duration = Duration::from_millis(750);

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
    origin: SourceEditOrigin,
    single_edit: bool,
    selection_before: SourceSelection,
    selection_after: SourceSelection,
    committed_at: Instant,
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
        let source_before = self.document.source.clone();
        let applied = self.document.apply_transaction(transaction)?;
        let SourceEditTransaction {
            document,
            forward_edits,
            inverse_edits,
            origin,
            selection_before,
            selection_after,
            parse_scope: _,
        } = applied;
        self.document = document;
        let now = Instant::now();
        let next = SourceHistoryEntry {
            single_edit: forward_edits.len() == 1,
            forward_edits,
            inverse_edits,
            origin,
            selection_before,
            selection_after,
            committed_at: now,
        };
        if self
            .undo
            .last()
            .is_some_and(|previous| previous.can_coalesce_with(&next, now))
        {
            let previous = self.undo.last_mut().expect("checked above");
            let source_at_group_start =
                crate::apply_edits(&source_before, &previous.inverse_edits)?;
            let (forward_edits, inverse_edits) = minimal_edits(
                &source_at_group_start,
                &self.document.source,
                self.document.revision,
            );
            previous.forward_edits = forward_edits;
            previous.inverse_edits = inverse_edits;
            previous.selection_after = next.selection_after;
            previous.committed_at = now;
        } else {
            self.undo.push(next);
        }
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

impl SourceHistoryEntry {
    fn can_coalesce_with(&self, next: &Self, now: Instant) -> bool {
        matches!(
            self.origin,
            SourceEditOrigin::SourceEditor | SourceEditOrigin::RichTextTyping
        ) && self.origin == next.origin
            && self.single_edit
            && next.single_edit
            && self.selection_after == next.selection_before
            && now.saturating_duration_since(self.committed_at) <= TYPING_COALESCE_WINDOW
    }
}

fn minimal_edits(before: &str, after: &str, revision: u64) -> (Vec<SourceEdit>, Vec<SourceEdit>) {
    if before == after {
        return (Vec::new(), Vec::new());
    }
    let prefix = common_prefix_len(before, after);
    let suffix = common_suffix_len(&before[prefix..], &after[prefix..]);
    let before_end = before.len() - suffix;
    let after_end = after.len() - suffix;
    (
        vec![SourceEdit::new(
            prefix..before_end,
            &after[prefix..after_end],
            revision,
        )],
        vec![SourceEdit::new(
            prefix..after_end,
            &before[prefix..before_end],
            revision,
        )],
    )
}

fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum()
}

fn common_suffix_len(left: &str, right: &str) -> usize {
    left.chars()
        .rev()
        .zip(right.chars().rev())
        .take_while(|(left, right)| left == right)
        .map(|(character, _)| character.len_utf8())
        .sum()
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
