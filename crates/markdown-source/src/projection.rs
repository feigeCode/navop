use crate::{
    SourceBlock, SourceEdit, SourceEditOrigin, SourceMarkdownDocument, SourceNodeCompatibility,
    SourceTransaction,
};

mod inline;

const MAX_LCS_CELLS: usize = 250_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectionResult {
    pub document: SourceMarkdownDocument,
    pub transaction: Option<SourceTransaction>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectionError {
    #[error(transparent)]
    Parse(#[from] crate::SourceParseError),
    #[error(transparent)]
    Patch(#[from] crate::PatchError),
}

pub fn reconcile_projection(
    document: &SourceMarkdownDocument,
    candidate: &str,
) -> Result<ProjectionResult, ProjectionError> {
    let projected = SourceMarkdownDocument::parse(candidate)?;
    let matches = matching_blocks(&document.blocks, &projected.blocks);
    let edits = projection_edits(document, &projected, &matches);
    if edits.is_empty() {
        return Ok(ProjectionResult {
            document: document.clone(),
            transaction: None,
        });
    }
    let transaction = SourceTransaction {
        allowed_ranges: edits.iter().map(|edit| edit.range.clone()).collect(),
        edits,
        origin: SourceEditOrigin::RichTextTyping,
        selection_before: crate::SourceSelection::default(),
        selection_after: crate::SourceSelection::default(),
    };
    let applied = document.apply_transaction(&transaction)?;
    Ok(ProjectionResult {
        document: applied.document,
        transaction: Some(transaction),
    })
}

fn projection_edits(
    original: &SourceMarkdownDocument,
    candidate: &SourceMarkdownDocument,
    matches: &[(usize, usize)],
) -> Vec<SourceEdit> {
    let mut edits = Vec::new();
    let mut old_cursor = 0;
    let mut new_cursor = 0;
    for &(old_match, new_match) in matches.iter().chain(std::iter::once(&(
        original.blocks.len(),
        candidate.blocks.len(),
    ))) {
        append_gap_edits(
            original,
            candidate,
            old_cursor..old_match,
            new_cursor..new_match,
            &mut edits,
        );
        old_cursor = old_match.saturating_add(1);
        new_cursor = new_match.saturating_add(1);
    }
    edits
}

fn append_gap_edits(
    original: &SourceMarkdownDocument,
    candidate: &SourceMarkdownDocument,
    old_gap: std::ops::Range<usize>,
    new_gap: std::ops::Range<usize>,
    edits: &mut Vec<SourceEdit>,
) {
    if old_gap.len() == new_gap.len() {
        for (old_index, new_index) in old_gap.zip(new_gap) {
            append_block_edit(original, candidate, old_index, new_index, edits);
        }
        return;
    }
    let old_range = gap_source_range(original, old_gap);
    let new_range = gap_source_range(candidate, new_gap);
    let replacement = &candidate.source[new_range];
    if original.source[old_range.clone()] != *replacement {
        edits.push(SourceEdit::new(old_range, replacement, original.revision));
    }
}

fn append_block_edit(
    original: &SourceMarkdownDocument,
    candidate: &SourceMarkdownDocument,
    old_index: usize,
    new_index: usize,
    edits: &mut Vec<SourceEdit>,
) {
    let old = &original.blocks[old_index];
    let new = &candidate.blocks[new_index];
    if old.original_source == new.original_source {
        return;
    }
    if old.compatibility != SourceNodeCompatibility::Editable {
        return;
    }
    if let Some(inline_edits) = inline::reconcile_inline_edits(inline::InlineProjection {
        old_source: &original.source,
        new_source: &candidate.source,
        old_block: old,
        new_block: new,
        revision: original.revision,
    }) {
        edits.extend(inline_edits);
        return;
    }
    edits.push(SourceEdit::new(
        old.source_range.clone(),
        &new.original_source,
        original.revision,
    ));
}

fn gap_source_range(
    document: &SourceMarkdownDocument,
    gap: std::ops::Range<usize>,
) -> std::ops::Range<usize> {
    let start = gap
        .start
        .checked_sub(1)
        .and_then(|index| document.blocks.get(index))
        .map_or(0, |block| block.source_range.end);
    let end = document
        .blocks
        .get(gap.end)
        .map_or(document.source.len(), |block| block.source_range.start);
    start..end
}

fn matching_blocks(old: &[SourceBlock], new: &[SourceBlock]) -> Vec<(usize, usize)> {
    if old.len().saturating_mul(new.len()) > MAX_LCS_CELLS {
        return greedy_matches(old, new);
    }
    let width = new.len() + 1;
    let mut lengths = vec![0_usize; (old.len() + 1) * width];
    for old_index in (0..old.len()).rev() {
        for new_index in (0..new.len()).rev() {
            let index = old_index * width + new_index;
            lengths[index] = if old[old_index].fingerprint == new[new_index].fingerprint {
                lengths[(old_index + 1) * width + new_index + 1] + 1
            } else {
                lengths[(old_index + 1) * width + new_index]
                    .max(lengths[old_index * width + new_index + 1])
            };
        }
    }
    collect_matches(old, new, &lengths, width)
}

fn collect_matches(
    old: &[SourceBlock],
    new: &[SourceBlock],
    lengths: &[usize],
    width: usize,
) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let (mut old_index, mut new_index) = (0, 0);
    while old_index < old.len() && new_index < new.len() {
        if old[old_index].fingerprint == new[new_index].fingerprint {
            result.push((old_index, new_index));
            old_index += 1;
            new_index += 1;
        } else if lengths[(old_index + 1) * width + new_index]
            >= lengths[old_index * width + new_index + 1]
        {
            old_index += 1;
        } else {
            new_index += 1;
        }
    }
    result
}

fn greedy_matches(old: &[SourceBlock], new: &[SourceBlock]) -> Vec<(usize, usize)> {
    let mut result = Vec::new();
    let mut new_cursor = 0;
    for (old_index, block) in old.iter().enumerate() {
        let Some(relative) = new[new_cursor..]
            .iter()
            .position(|candidate| candidate.fingerprint == block.fingerprint)
        else {
            continue;
        };
        let new_index = new_cursor + relative;
        result.push((old_index, new_index));
        new_cursor = new_index + 1;
    }
    result
}
