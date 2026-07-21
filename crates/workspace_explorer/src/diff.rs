//! Parses unified Git diffs into a side-by-side row model so the editor can
//! render the old and new file contents next to each other.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DiffLine {
    pub(crate) number: usize,
    pub(crate) text: String,
    pub(crate) kind: DiffLineKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct DiffRow {
    pub(crate) left: Option<DiffLine>,
    pub(crate) right: Option<DiffLine>,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct SideBySideDiff {
    pub(crate) rows: Vec<DiffRow>,
    pub(crate) old_line_count: usize,
    pub(crate) new_line_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct AlignedDiffSide {
    pub(crate) text: String,
    pub(crate) line_numbers: Vec<Option<usize>>,
    pub(crate) changed: Vec<bool>,
    pub(crate) placeholders: Vec<bool>,
}

pub(crate) fn parse_side_by_side(diff: &str) -> SideBySideDiff {
    let mut result = SideBySideDiff::default();
    let mut old_line = 0usize;
    let mut new_line = 0usize;
    let mut in_hunk = false;
    let mut pending_removed: Vec<DiffLine> = Vec::new();
    let mut pending_added: Vec<DiffLine> = Vec::new();

    for line in first_file_section(diff).lines() {
        if let Some((old_start, new_start)) = line.strip_prefix("@@ ").and_then(parse_hunk_header) {
            flush_pending(&mut result, &mut pending_removed, &mut pending_added);
            old_line = old_start;
            new_line = new_start;
            in_hunk = true;
            continue;
        }
        if !in_hunk || line.starts_with('\\') {
            continue;
        }
        let (tag, text) = line.split_at(line.len().min(1));
        match tag {
            " " => {
                flush_pending(&mut result, &mut pending_removed, &mut pending_added);
                result.rows.push(DiffRow {
                    left: Some(DiffLine {
                        number: old_line,
                        text: text.to_string(),
                        kind: DiffLineKind::Context,
                    }),
                    right: Some(DiffLine {
                        number: new_line,
                        text: text.to_string(),
                        kind: DiffLineKind::Context,
                    }),
                });
                old_line += 1;
                new_line += 1;
            }
            "-" => {
                pending_removed.push(DiffLine {
                    number: old_line,
                    text: text.to_string(),
                    kind: DiffLineKind::Removed,
                });
                old_line += 1;
            }
            "+" => {
                pending_added.push(DiffLine {
                    number: new_line,
                    text: text.to_string(),
                    kind: DiffLineKind::Added,
                });
                new_line += 1;
            }
            _ => {}
        }
    }
    flush_pending(&mut result, &mut pending_removed, &mut pending_added);
    result.old_line_count = old_line.saturating_sub(1);
    result.new_line_count = new_line.saturating_sub(1);
    result
}

pub(crate) fn aligned_side_by_side(diff: &SideBySideDiff) -> (AlignedDiffSide, AlignedDiffSide) {
    let mut left = AlignedDiffSide::default();
    let mut right = AlignedDiffSide::default();

    for (index, row) in diff.rows.iter().enumerate() {
        if index > 0 {
            left.text.push('\n');
            right.text.push('\n');
        }

        append_side_line(&mut left, row.left.as_ref());
        append_side_line(&mut right, row.right.as_ref());
    }

    (left, right)
}

/// Returns the aligned row index where each contiguous change block starts.
pub(crate) fn change_starts(diff: &SideBySideDiff) -> Vec<usize> {
    let mut previous_changed = false;
    let mut starts = Vec::new();

    for (index, row) in diff.rows.iter().enumerate() {
        let changed = row
            .left
            .iter()
            .chain(row.right.iter())
            .any(|line| !matches!(line.kind, DiffLineKind::Context));
        if changed && !previous_changed {
            starts.push(index);
        }
        previous_changed = changed;
    }

    starts
}

fn append_side_line(side: &mut AlignedDiffSide, line: Option<&DiffLine>) {
    match line {
        Some(line) => {
            side.text.push_str(&line.text);
            side.line_numbers.push(Some(line.number));
            side.changed
                .push(!matches!(line.kind, DiffLineKind::Context));
            side.placeholders.push(false);
        }
        None => {
            side.line_numbers.push(None);
            side.changed.push(false);
            side.placeholders.push(true);
        }
    }
}

/// Restricts parsing to the first `diff --git` section. Combined diffs (for
/// example staged plus worktree output concatenated together) would otherwise
/// render the same file twice.
fn first_file_section(diff: &str) -> &str {
    let Some(start) = diff.find("diff --git ") else {
        return diff;
    };
    let section = &diff[start..];
    match section["diff --git ".len()..].find("\ndiff --git ") {
        Some(offset) => &section[..("diff --git ".len() + offset + 1)],
        None => section,
    }
}

fn parse_hunk_header(header: &str) -> Option<(usize, usize)> {
    // Header shape: "-old_start,old_count +new_start,new_count @@ optional context"
    let mut parts = header.split_whitespace();
    let old = parse_range_start(parts.next()?, b'-')?;
    let new = parse_range_start(parts.next()?, b'+')?;
    Some((old, new))
}

fn parse_range_start(range: &str, tag: u8) -> Option<usize> {
    let range = range.strip_prefix(char::from(tag))?;
    let start = range.split(',').next()?;
    start.parse().ok()
}

fn flush_pending(
    result: &mut SideBySideDiff,
    removed: &mut Vec<DiffLine>,
    added: &mut Vec<DiffLine>,
) {
    let pairs = removed.len().max(added.len());
    for index in 0..pairs {
        result.rows.push(DiffRow {
            left: removed.get(index).cloned(),
            right: added.get(index).cloned(),
        });
    }
    removed.clear();
    added.clear();
}

#[cfg(test)]
mod tests;
