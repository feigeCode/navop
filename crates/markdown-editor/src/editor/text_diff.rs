pub(super) fn common_prefix(left: &str, right: &str) -> usize {
    left.char_indices()
        .zip(right.chars())
        .take_while(|((_, left), right)| left == right)
        .last()
        .map_or(0, |((offset, ch), _)| offset + ch.len_utf8())
}

pub(super) fn common_suffix(left: &str, right: &str) -> usize {
    left.char_indices()
        .rev()
        .zip(right.chars().rev())
        .take_while(|((_, left), right)| left == right)
        .map(|((_, ch), _)| ch.len_utf8())
        .sum()
}

pub(super) fn minimal_text_patch(current: &str, target: &str) -> Option<(Range<usize>, String)> {
    if current == target {
        return None;
    }
    let prefix = common_prefix(current, target);
    let suffix = common_suffix(&current[prefix..], &target[prefix..]);
    let current_end = current.len().saturating_sub(suffix);
    let target_end = target.len().saturating_sub(suffix);
    Some((prefix..current_end, target[prefix..target_end].to_owned()))
}
use std::ops::Range;
