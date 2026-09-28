//! 工具卡片的行级 diff:把「改动前 / 改动后完整文本」算成可渲染的行。
//!
//! 为什么自己算而不复用工作区的 diff:那个模块解析的是**已经生成好的 unified
//! patch**(Git 给什么渲染什么);这里的输入是 ACP 协议直接给的完整文本,两侧
//! 内容都在手里,缺的只是行对齐。为此引一个 diff 依赖不值得——下面的实现是
//! 教科书式的最长公共子序列,先裁掉公共前后缀,再对小规模中段做 DP。
//!
//! 输出是**渲染行**(已带上行号与增删标记),不是 patch 文本:卡片刻意不再二次
//! 解析字符串,免得为了显示又写一遍 parser。

use serde::{Deserialize, Serialize};

/// 内联 diff 最多展示的行数。超出的行不进 `rows`,只记进 `hidden`。
///
/// 「一次大编辑只占一个卡片高度」是这一版的设计前提:视口高度必须与改动规模
/// 无关,否则一次全文件格式化就会把整条时间线顶走。
pub const TOOL_DIFF_MAX_ROWS: usize = 12;

/// 改动行上下各保留的上下文行数。
const CONTEXT_LINES: usize = 3;

/// 两段中段相乘超过这么多格就放弃 DP,退化成整块替换。
///
/// 纯手写的 O(n·m) 表在这种输入上(整文件重写、格式全变)既慢又没意义:
/// 逐行对齐的结果本来就没有阅读价值,不如老老实实报「删了这些、加了这些」。
const LCS_MAX_CELLS: usize = 250_000;

/// 单行 diff 的类型。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffRowKind {
    /// 未改动(上下文)。
    Context,
    /// 新增。
    Added,
    /// 删除。
    Removed,
}

/// 一行内联 diff。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffRow {
    /// 旧文件行号;新增行没有。
    #[serde(default)]
    pub old_no: Option<u32>,
    /// 新文件行号;删除行没有。
    #[serde(default)]
    pub new_no: Option<u32>,
    pub kind: DiffRowKind,
    pub text: String,
}

/// 一个文件的改动摘要:卡片渲染需要的全部信息。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChangeSummary {
    /// 文件路径(展示用;是否绝对路径取决于协议给的原文)。
    pub path: String,
    /// 新增行数(整个文件,不只是展示出来的那些)。
    #[serde(default)]
    pub added: u32,
    /// 删除行数(整个文件)。
    #[serde(default)]
    pub removed: u32,
    /// 是否为新建文件。
    #[serde(default)]
    pub created: bool,
    /// 展示用的行;最多 [`TOOL_DIFF_MAX_ROWS`] 行。
    #[serde(default)]
    pub rows: Vec<DiffRow>,
    /// 因行数上限未展示的行数。
    #[serde(default)]
    pub hidden: u32,
}

impl FileChangeSummary {
    /// 由改动前后的完整文本构造摘要。
    ///
    /// `old` 为 `None` 表示新建文件。`path` 为空时仍会产出摘要(展示层自行
    /// 决定要不要显示路径)。
    pub fn from_texts(path: impl Into<String>, old: Option<&str>, new: &str) -> Self {
        let created = old.is_none();
        let (ops, added, removed) = diff_ops(old.unwrap_or(""), new);
        let windows = display_windows(&ops);
        let total: usize = windows.iter().map(|(start, end)| end - start).sum();
        let mut rows = Vec::with_capacity(total.min(TOOL_DIFF_MAX_ROWS));
        for (start, end) in windows {
            for op in ops.iter().take(end).skip(start) {
                if rows.len() == TOOL_DIFF_MAX_ROWS {
                    break;
                }
                rows.push(DiffRow {
                    old_no: op.old_no,
                    new_no: op.new_no,
                    kind: op.kind,
                    text: op.text.to_string(),
                });
            }
            if rows.len() == TOOL_DIFF_MAX_ROWS {
                break;
            }
        }
        Self {
            path: path.into(),
            added: added as u32,
            removed: removed as u32,
            created,
            hidden: total.saturating_sub(rows.len()) as u32,
            rows,
        }
    }

    /// 是否有可展示的差异行。
    pub fn has_rows(&self) -> bool {
        !self.rows.is_empty()
    }
}

/// diff 中间表示:一行文本 + 它在两侧的行号(没有的那侧为 `None`)。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Op<'a> {
    old_no: Option<u32>,
    new_no: Option<u32>,
    kind: DiffRowKind,
    text: &'a str,
}

/// 按行切分,并且**不把结尾换行当成一行**。
///
/// `"a\n"` 是「一行 a」,不是「一行 a 加一行空」。这一点必须统一,否则
/// 「末尾补一个换行」会被渲染成凭空多出一行。
fn lines(text: &str) -> Vec<&str> {
    if text.is_empty() {
        return Vec::new();
    }
    let mut lines: Vec<&str> = text.split('\n').collect();
    if lines.last() == Some(&"") {
        lines.pop();
    }
    lines
}

/// 生成整份文件的逐行操作序列,同时给出增删计数。
fn diff_ops<'a>(old: &'a str, new: &'a str) -> (Vec<Op<'a>>, usize, usize) {
    let old_lines = lines(old);
    let new_lines = lines(new);

    // 公共前后缀先裁掉。真实的编辑几乎总是改动一小段,这一步能把 DP 的规模
    // 从「文件行数」压到「改动块大小」。
    let mut prefix = 0;
    while prefix < old_lines.len()
        && prefix < new_lines.len()
        && old_lines[prefix] == new_lines[prefix]
    {
        prefix += 1;
    }
    let mut suffix = 0;
    while suffix < old_lines.len() - prefix
        && suffix < new_lines.len() - prefix
        && old_lines[old_lines.len() - 1 - suffix] == new_lines[new_lines.len() - 1 - suffix]
    {
        suffix += 1;
    }

    let mid_old = &old_lines[prefix..old_lines.len() - suffix];
    let mid_new = &new_lines[prefix..new_lines.len() - suffix];

    let mut ops: Vec<Op<'a>> = Vec::with_capacity(
        old_lines.len().max(new_lines.len()) + mid_old.len() + mid_new.len(),
    );
    let mut old_no = 1u32;
    let mut new_no = 1u32;

    for line in &old_lines[..prefix] {
        ops.push(Op {
            old_no: Some(old_no),
            new_no: Some(new_no),
            kind: DiffRowKind::Context,
            text: line,
        });
        old_no += 1;
        new_no += 1;
    }

    let mut added = 0usize;
    let mut removed = 0usize;
    for (kind, text) in align_mid(mid_old, mid_new) {
        let op = match kind {
            DiffRowKind::Context => {
                let op = Op {
                    old_no: Some(old_no),
                    new_no: Some(new_no),
                    kind,
                    text,
                };
                old_no += 1;
                new_no += 1;
                op
            }
            DiffRowKind::Removed => {
                let op = Op {
                    old_no: Some(old_no),
                    new_no: None,
                    kind,
                    text,
                };
                old_no += 1;
                removed += 1;
                op
            }
            DiffRowKind::Added => {
                let op = Op {
                    old_no: None,
                    new_no: Some(new_no),
                    kind,
                    text,
                };
                new_no += 1;
                added += 1;
                op
            }
        };
        ops.push(op);
    }

    for line in &old_lines[old_lines.len() - suffix..] {
        ops.push(Op {
            old_no: Some(old_no),
            new_no: Some(new_no),
            kind: DiffRowKind::Context,
            text: line,
        });
        old_no += 1;
        new_no += 1;
    }

    (ops, added, removed)
}

/// 对齐两段中段,产出 `(类型, 行文本)`。
fn align_mid<'a>(mid_old: &[&'a str], mid_new: &[&'a str]) -> Vec<(DiffRowKind, &'a str)> {
    // 整块新增 / 删除:不必进 DP。
    if mid_old.is_empty() {
        return mid_new.iter().map(|l| (DiffRowKind::Added, *l)).collect();
    }
    if mid_new.is_empty() {
        return mid_old.iter().map(|l| (DiffRowKind::Removed, *l)).collect();
    }
    if mid_old.len().saturating_mul(mid_new.len()) > LCS_MAX_CELLS {
        // 规模过大:放弃逐行对齐,报「整块替换」。这是**有意的降级**——逐行
        // 对齐的结果在这种输入上已经不可读,不值得为它付出 O(n·m) 的内存。
        let mut out: Vec<(DiffRowKind, &'a str)> =
            mid_old.iter().map(|l| (DiffRowKind::Removed, *l)).collect();
        out.extend(mid_new.iter().map(|l| (DiffRowKind::Added, *l)));
        return out;
    }

    let rows = mid_old.len();
    let cols = mid_new.len();
    // lcs[i * (cols + 1) + j] = mid_old[i..] 与 mid_new[j..] 的最长公共子序列长度。
    let mut lcs = vec![0u32; (rows + 1) * (cols + 1)];
    for i in (0..rows).rev() {
        for j in (0..cols).rev() {
            lcs[i * (cols + 1) + j] = if mid_old[i] == mid_new[j] {
                lcs[(i + 1) * (cols + 1) + j + 1] + 1
            } else {
                lcs[(i + 1) * (cols + 1) + j].max(lcs[i * (cols + 1) + j + 1])
            };
        }
    }

    let mut out = Vec::with_capacity(rows.max(cols));
    let (mut i, mut j) = (0usize, 0usize);
    while i < rows && j < cols {
        if mid_old[i] == mid_new[j] {
            out.push((DiffRowKind::Context, mid_old[i]));
            i += 1;
            j += 1;
        } else if lcs[(i + 1) * (cols + 1) + j] >= lcs[i * (cols + 1) + j + 1] {
            out.push((DiffRowKind::Removed, mid_old[i]));
            i += 1;
        } else {
            out.push((DiffRowKind::Added, mid_new[j]));
            j += 1;
        }
    }
    out.extend(mid_old[i..].iter().map(|l| (DiffRowKind::Removed, *l)));
    out.extend(mid_new[j..].iter().map(|l| (DiffRowKind::Added, *l)));
    out
}

/// 只保留改动行及其上下 [`CONTEXT_LINES`] 行,得到要渲染的区间。
///
/// 相隔很近的改动块会合并成一个区间,避免同一个文件里出现两段只隔一行的
/// 「上下文窗口」互相重叠。
fn display_windows(ops: &[Op<'_>]) -> Vec<(usize, usize)> {
    let mut windows: Vec<(usize, usize)> = Vec::new();
    for (index, op) in ops.iter().enumerate() {
        if op.kind == DiffRowKind::Context {
            continue;
        }
        match windows.last_mut() {
            Some(last) if index <= last.1 + CONTEXT_LINES * 2 => last.1 = index + 1,
            _ => windows.push((index, index + 1)),
        }
    }
    windows
        .into_iter()
        .map(|(start, end)| {
            (
                start.saturating_sub(CONTEXT_LINES),
                (end + CONTEXT_LINES).min(ops.len()),
            )
        })
        .filter(|(start, end)| end > start)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kinds(rows: &[DiffRow]) -> Vec<DiffRowKind> {
        rows.iter().map(|row| row.kind).collect()
    }

    #[test]
    fn identical_text_has_no_rows() {
        let summary = FileChangeSummary::from_texts("a.rs", Some("x\ny\n"), "x\ny\n");
        assert!(!summary.has_rows());
        assert_eq!(0, summary.added);
        assert_eq!(0, summary.removed);
        assert!(!summary.created);
    }

    #[test]
    fn a_single_line_edit_keeps_its_neighbours_as_context() {
        let old = "1\n2\n3\n4\n5\n6\n7\n";
        let new = "1\n2\n3\nX\n5\n6\n7\n";
        let summary = FileChangeSummary::from_texts("a.rs", Some(old), new);

        assert_eq!(1, summary.added);
        assert_eq!(1, summary.removed);
        assert_eq!(
            vec![
                // 改动前一行上下文
                DiffRowKind::Context,
                DiffRowKind::Context,
                DiffRowKind::Context,
                DiffRowKind::Removed,
                DiffRowKind::Added,
                // 改动后三行上下文
                DiffRowKind::Context,
                DiffRowKind::Context,
                DiffRowKind::Context,
            ],
            kinds(&summary.rows)
        );
        // 行号:删掉的是旧文件第 4 行,新增的是新文件第 4 行,上下文两侧对齐。
        assert_eq!(Some(4), summary.rows[3].old_no);
        assert_eq!(None, summary.rows[3].new_no);
        assert_eq!(None, summary.rows[4].old_no);
        assert_eq!(Some(4), summary.rows[4].new_no);
        assert_eq!(Some(5), summary.rows[5].old_no);
        assert_eq!(Some(5), summary.rows[5].new_no);
    }

    #[test]
    fn created_file_is_all_additions_and_marks_created() {
        let summary = FileChangeSummary::from_texts("new.rs", None, "a\nb\n");
        assert!(summary.created);
        assert_eq!(2, summary.added);
        assert_eq!(0, summary.removed);
        assert_eq!(
            vec![DiffRowKind::Added, DiffRowKind::Added],
            kinds(&summary.rows)
        );
    }

    #[test]
    fn trailing_newline_does_not_invent_a_line() {
        let summary = FileChangeSummary::from_texts("a.rs", Some("a\n"), "a\nb\n");
        assert_eq!(1, summary.added);
        assert_eq!(0, summary.removed);
        // 只多出一行:另一行是上下文,不是新增。
        assert_eq!(1, summary.rows.iter().filter(|r| r.kind == DiffRowKind::Added).count());
        assert_eq!(2, summary.rows.len());

        // 只是补了末尾换行:没有内容变化,就不该有差异行。
        let same = FileChangeSummary::from_texts("a.rs", Some("a"), "a\n");
        assert!(!same.has_rows());
    }

    #[test]
    fn rows_are_capped_and_the_remainder_is_counted_as_hidden() {
        let old = (0..40)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let new = (0..40)
            .map(|i| format!("changed{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let summary = FileChangeSummary::from_texts("a.rs", Some(&old), &new);

        assert_eq!(40, summary.added);
        assert_eq!(40, summary.removed);
        assert_eq!(TOOL_DIFF_MAX_ROWS, summary.rows.len());
        assert_eq!((80 - TOOL_DIFF_MAX_ROWS) as u32, summary.hidden);
    }

    #[test]
    fn huge_rewrites_degrade_to_a_whole_block_replacement() {
        // 超过 DP 上限后不应 panic,也不应产生交错的对齐结果。
        let old: String = (0..600).map(|i| format!("old{i}\n")).collect();
        let new: String = (0..600).map(|i| format!("new{i}\n")).collect();
        let summary = FileChangeSummary::from_texts("a.rs", Some(&old), &new);

        assert_eq!(600, summary.added);
        assert_eq!(600, summary.removed);
        assert_eq!(TOOL_DIFF_MAX_ROWS, summary.rows.len());
    }

    #[test]
    fn distant_changes_become_separate_windows() {
        let old: String = (0..60).map(|i| format!("line{i}\n")).collect();
        let mut new_lines: Vec<String> = (0..60).map(|i| format!("line{i}")).collect();
        new_lines[5] = "changed5".to_string();
        new_lines[50] = "changed50".to_string();
        let new = format!("{}\n", new_lines.join("\n"));

        let summary = FileChangeSummary::from_texts("a.rs", Some(&old), &new);
        // 两处改动各自带上下文,但总量仍受上限约束。
        assert_eq!(2, summary.added);
        assert_eq!(2, summary.removed);
        assert!(summary.rows.len() <= TOOL_DIFF_MAX_ROWS);
        assert!(summary.hidden > 0);
    }

    #[test]
    fn empty_path_is_still_renderable() {
        let summary = FileChangeSummary::from_texts("", None, "a\n");
        assert_eq!("", summary.path);
        assert!(summary.created);
        assert!(summary.has_rows());
    }
}
