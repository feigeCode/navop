//! Parses unified Git diffs into a side-by-side row model so the editor can
//! render the old and new file contents next to each other.

use std::ops::Range;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffLineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiffLine {
    pub number: usize,
    pub text: String,
    pub kind: DiffLineKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DiffRow {
    pub left: Option<DiffLine>,
    pub right: Option<DiffLine>,
}

#[derive(Clone, Debug, Default)]
pub struct SideBySideDiff {
    pub rows: Vec<DiffRow>,
    pub old_line_count: usize,
    pub new_line_count: usize,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AlignedDiffSide {
    pub text: String,
    pub line_numbers: Vec<Option<usize>>,
    pub changed: Vec<bool>,
    pub placeholders: Vec<bool>,
}

pub fn parse_side_by_side(diff: &str) -> SideBySideDiff {
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

pub fn aligned_side_by_side(diff: &SideBySideDiff) -> (AlignedDiffSide, AlignedDiffSide) {
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

/// How one aligned line reads in the side-by-side view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AlignedSpanKind {
    /// Identical on both sides; nothing to point out.
    Context,
    /// This side changed here. Direction is the pane's business: the left pane
    /// renders it as a removal, the right pane as an addition.
    Changed,
    /// This side has no counterpart on this row; the blank exists only to keep
    /// the two panes line-for-line aligned.
    Placeholder,
}

/// Byte range and decoration meaning of every aligned line.
///
/// Byte ranges index into [`AlignedDiffSide::text`] and line up with its
/// `changed` / `placeholders` / `line_numbers` vectors.
///
/// Every range **includes the trailing newline** (the last line has none). That
/// is load-bearing, not incidental: a `RangeDecoration` fill is dropped twice
/// over when its range is empty — `normalize` refuses empty ranges outright, and
/// `layout_range_corners` in the editor element returns `None` for them. A range
/// that covers the newline instead lands in the "selected newline has a
/// one-space cell" branch, which paints the whole line plus one space, and gives
/// an empty line its cell too. Without the newline the blank and placeholder
/// rows — the ones that most need pointing at — would simply not be painted.
pub fn aligned_span_ranges(side: &AlignedDiffSide) -> Vec<(Range<usize>, AlignedSpanKind)> {
    let line_count = side.line_numbers.len();
    let mut spans = Vec::with_capacity(line_count);
    let mut start = 0usize;

    for (index, line) in side.text.split('\n').enumerate() {
        if index >= line_count {
            break;
        }
        let end = start + line.len() + usize::from(start + line.len() < side.text.len());
        let kind = if side.placeholders.get(index).copied().unwrap_or(false) {
            AlignedSpanKind::Placeholder
        } else if side.changed.get(index).copied().unwrap_or(false) {
            AlignedSpanKind::Changed
        } else {
            AlignedSpanKind::Context
        };
        spans.push((start..end, kind));
        start = end;
    }

    spans
}

/// 单栏 diff 原文里一行的语义（只区分要染成什么色）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiffTextSpanKind {
    /// 新增行（`+`）。
    Added,
    /// 删除行（`-`）。
    Removed,
    /// 段落标记：`diff --git` 与 `@@`。整轮多文件 diff 里唯一的结构线索。
    Marker,
}

/// 单栏 diff **原文**里需要着色的行的字节区间。
///
/// 并排两栏各自带行背景；退回单栏查看时（整轮快照 diff 恒为单栏，单文件 diff
/// 关掉「并排对比」后也是单栏）没有任何装饰，整屏就只剩黑白文本，看不出哪行是
/// 增、哪行是删。`aligned_span_ranges` 服务于对齐后的两栏，这里服务于原始
/// diff 文本，两者不能互替。
///
/// 与 [`aligned_span_ranges`] 同一约定：区间**包含行尾换行**（最后一行没有）。
/// 删掉换行的话空行的填充会被丢弃两次（`normalize` 拒绝空区间，
/// `layout_range_corners` 对空区间返回 `None`），于是最该被点出来的空行反而
/// 看不见——理由与那一段相同，不重复。
///
/// 增删只在 hunk **内**按首字符判断。若不管 hunk 直接看前缀，`+++ b/x` /
/// `--- a/x` 这两行文件头会被染成增删色，而真正以 `++` 开头的新增行会被误当成
/// 文件头——状态机只需要认 `@@` 这个起点就能同时避开两种错法。
pub fn diff_text_spans(diff: &str) -> Vec<(Range<usize>, DiffTextSpanKind)> {
    let mut spans = Vec::new();
    let mut start = 0usize;
    let mut in_hunk = false;

    for line in diff.split_inclusive('\n') {
        let end = start + line.len();
        let kind = if line.starts_with("diff --git ") {
            in_hunk = false;
            Some(DiffTextSpanKind::Marker)
        } else if line.starts_with("@@") {
            in_hunk = true;
            Some(DiffTextSpanKind::Marker)
        } else if !in_hunk || line.starts_with('\\') {
            // 文件头（`index` / `---` / `+++` / `new file mode` …）与
            // `\ No newline at end of file` 都不染。
            None
        } else {
            match line.as_bytes().first() {
                Some(b'+') => Some(DiffTextSpanKind::Added),
                Some(b'-') => Some(DiffTextSpanKind::Removed),
                _ => None,
            }
        };
        if let Some(kind) = kind {
            spans.push((start..end, kind));
        }
        start = end;
    }

    spans
}

/// Returns the aligned row index where each contiguous change block starts.
pub fn change_starts(diff: &SideBySideDiff) -> Vec<usize> {
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
