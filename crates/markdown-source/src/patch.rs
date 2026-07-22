use crate::SourceEdit;
use std::ops::Range;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PatchError {
    #[error("source revision does not match")]
    StaleRevision,
    #[error("source edits overlap")]
    OverlappingEdits,
    #[error("source edit range is invalid")]
    InvalidRange,
    #[error("source edit is outside the allowed ranges")]
    OutsideAllowedRanges,
    #[error("candidate changes bytes outside the allowed ranges")]
    UnexpectedChange,
    #[error("updated Markdown could not be parsed: {0}")]
    Parse(String),
}

pub fn apply_edits(source: &str, edits: &[SourceEdit]) -> Result<String, PatchError> {
    validate_edit_ranges(source, edits)?;
    let mut ordered = edits.iter().collect::<Vec<_>>();
    ordered.sort_by_key(|edit| edit.range.start);
    validate_non_overlapping(&ordered)?;
    let mut result = source.to_owned();
    for edit in ordered.into_iter().rev() {
        result.replace_range(edit.range.clone(), &edit.replacement);
    }
    Ok(result)
}

pub fn validate_expected_changes(
    original: &str,
    candidate: &str,
    allowed_ranges: &[Range<usize>],
) -> Result<(), PatchError> {
    if allowed_ranges.is_empty() {
        return (original == candidate)
            .then_some(())
            .ok_or(PatchError::UnexpectedChange);
    }
    let allowed = merged_ranges(original, allowed_ranges)?;
    let unchanged = unchanged_ranges(original.len(), &allowed);
    validate_unchanged_segments(original, candidate, &unchanged)
}

fn validate_edit_ranges(source: &str, edits: &[SourceEdit]) -> Result<(), PatchError> {
    let valid = edits.iter().all(|edit| {
        edit.range.start <= edit.range.end
            && edit.range.end <= source.len()
            && source.is_char_boundary(edit.range.start)
            && source.is_char_boundary(edit.range.end)
    });
    valid.then_some(()).ok_or(PatchError::InvalidRange)
}

fn validate_non_overlapping(edits: &[&SourceEdit]) -> Result<(), PatchError> {
    let overlaps = edits
        .windows(2)
        .any(|pair| pair[0].range.end > pair[1].range.start);
    (!overlaps)
        .then_some(())
        .ok_or(PatchError::OverlappingEdits)
}

fn merged_ranges(source: &str, ranges: &[Range<usize>]) -> Result<Vec<Range<usize>>, PatchError> {
    let mut ordered = ranges.to_vec();
    ordered.sort_by_key(|range| range.start);
    if ordered.iter().any(|range| {
        range.start > range.end
            || range.end > source.len()
            || !source.is_char_boundary(range.start)
            || !source.is_char_boundary(range.end)
    }) {
        return Err(PatchError::InvalidRange);
    }
    let mut merged: Vec<Range<usize>> = Vec::new();
    for range in ordered {
        match merged.last_mut() {
            Some(last) if range.start <= last.end => last.end = last.end.max(range.end),
            _ => merged.push(range),
        }
    }
    Ok(merged)
}

fn unchanged_ranges(length: usize, allowed: &[Range<usize>]) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut cursor = 0;
    for range in allowed {
        if cursor < range.start {
            result.push(cursor..range.start);
        }
        cursor = range.end;
    }
    if cursor < length {
        result.push(cursor..length);
    }
    result
}

fn validate_unchanged_segments(
    original: &str,
    candidate: &str,
    unchanged: &[Range<usize>],
) -> Result<(), PatchError> {
    let mut candidate_cursor = 0;
    for (index, range) in unchanged.iter().enumerate() {
        let segment = &original[range.clone()];
        let is_prefix = range.start == 0;
        let is_suffix = range.end == original.len();
        let found = if is_prefix {
            candidate.starts_with(segment).then_some(0)
        } else if is_suffix {
            candidate
                .ends_with(segment)
                .then(|| candidate.len() - segment.len())
        } else {
            candidate[candidate_cursor..]
                .find(segment)
                .map(|offset| candidate_cursor + offset)
        };
        let Some(found) = found.filter(|found| *found >= candidate_cursor) else {
            return Err(PatchError::UnexpectedChange);
        };
        candidate_cursor = found + segment.len();
        if index + 1 == unchanged.len() && is_suffix && candidate_cursor != candidate.len() {
            return Err(PatchError::UnexpectedChange);
        }
    }
    Ok(())
}
