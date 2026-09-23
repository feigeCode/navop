//! 表格内查找（Cmd/Ctrl+F）
//!
//! 表格只能看到当前滚动窗口内的行，所以查找必须落在 delegate 的
//! 「显示行」坐标系里：匹配结果与可见性判断都由 delegate 提供，
//! 这里只负责查询状态、跨行计数、命中解析和纯函数工具。

mod element;
mod panel;

pub use element::{FindHighlightElement, HighlightBand, HighlightSegment, highlight_bands};
pub use panel::{SearchPanel, SearchPanelEvent};

use std::ops::Range;

use gpui::{Hsla, Pixels, hsla, px};

/// 单元格文本之间的逻辑分隔符，用于把整行文本拼成一个可比较的字符串。
pub const CELL_SEPARATOR: &str = "  ";

/// 分隔符的字符数（不依赖字节数，方便按字符坐标推导单元格偏移）。
pub const CELL_SEPARATOR_CHARS: usize = 2;

/// NULL 单元格在查找里表现为 `NULL`，与用户肉眼看到的一致。
pub const NULL_TEXT: &str = "NULL";

/// 一次查询在 delegate 行坐标系中的完整结果集。
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FindOutcome {
    /// 全表命中总数
    pub total: usize,
    /// 有命中的行（升序、去重）
    pub rows: Vec<usize>,
}

impl FindOutcome {
    /// 是否没有命中
    pub fn is_empty(&self) -> bool {
        self.total == 0
    }
}

/// 在 delegate 行坐标系上执行一次查询。
///
/// - `row_count`：当前可显示的行数
/// - `query`：已规范化的查询词
/// - `matcher`：对单行返回命中次数
///
/// 命中「第几个」由调用方按每行命中数累加得出，这里只负责
/// 「哪些行有命中、总共几个命中」。
pub fn resolve_find(
    row_count: usize,
    query: &str,
    mut matcher: impl FnMut(usize) -> usize,
) -> FindOutcome {
    if query.is_empty() || row_count == 0 {
        return FindOutcome::default();
    }

    let mut total = 0usize;
    let mut rows = Vec::new();
    for row in 0..row_count {
        let hits = matcher(row);
        if hits > 0 {
            total += hits;
            rows.push(row);
        }
    }

    FindOutcome { total, rows }
}

/// 单元格文本中的命中区间、在整行文本中的位置，以及是否当前命中。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FindMatch {
    /// 在单元格文本中的字符下标区间
    pub char_range: Range<usize>,
    /// 该单元格在整行文本中的字符起点
    pub row_offset: usize,
    /// 是否为当前导航到的命中
    pub is_current: bool,
}

/// 把一行按单元格切分后计算所有命中区间。
///
/// `query` 必须已被 [`normalize_find_query`] 规范化（小写）。
pub fn row_matches(cells: &[Option<String>], query: &str) -> Vec<FindMatch> {
    if query.is_empty() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut offset = 0usize;
    for cell in cells {
        let text = cell_text(cell);
        let lowered: Vec<char> = text.to_lowercase().chars().collect();
        matches.extend(collect_cell_matches(&lowered, query, offset));
        offset += text.chars().count() + CELL_SEPARATOR_CHARS;
    }
    matches
}

/// 行的整行逻辑文本（单元格之间用两个空格分隔）。
///
/// 先小写化再拼接，保证与 [`row_matches`] 的匹配口径一致。
pub fn row_text(cells: &[Option<String>]) -> String {
    cells
        .iter()
        .map(|cell| cell_text(cell).to_lowercase())
        .collect::<Vec<_>>()
        .join(CELL_SEPARATOR)
}

fn cell_text(cell: &Option<String>) -> &str {
    cell.as_deref().unwrap_or(NULL_TEXT)
}

fn collect_cell_matches(lowered: &[char], query: &str, offset: usize) -> Vec<FindMatch> {
    let needle: Vec<char> = query.chars().collect();
    if needle.is_empty() || needle.len() > lowered.len() {
        return Vec::new();
    }

    let mut matches = Vec::new();
    let mut index = 0usize;
    while index + needle.len() <= lowered.len() {
        if lowered[index..index + needle.len()] == needle[..] {
            matches.push(FindMatch {
                char_range: index..index + needle.len(),
                row_offset: offset,
                is_current: false,
            });
            // 命中区间不重叠，避免 "aa" 在 "aaa" 里产生交叠结果。
            index += needle.len();
        } else {
            index += 1;
        }
    }
    matches
}

/// 查询词按小写 + trim 规范化，与 [`row_text`] 的匹配口径保持一致。
pub fn normalize_find_query(query: &str) -> String {
    query.trim().to_lowercase()
}

/// 把整行文本的字符下标换算成 gpui 布局查询需要的 UTF-8 字节下标。
///
/// gpui 的 `LineLayout::x_for_index`（以及 `TextRun::len`）都按**字节**下标
/// 定位——`LineLayout::len` 的文档写的是 "the length of the line in utf-8
/// bytes"，`x_for_index` 比对的也是字形的起始字节。而查找的匹配与计数按
/// **字符**下标做（CJK 一个字符占 3 字节）。两者混用会让高亮矩形落到别的
/// 字符上；当命中区间首尾落在同一个字形的字节范围内时，两端取到同一个 x，
/// 矩形宽度变成 0 被丢弃，表现就是「明明有命中，却完全看不到高亮」。
///
/// 下标越界（例如 `to_lowercase` 改变了字符数）时退回文本末尾，不 panic。
pub fn char_index_to_byte_index(text: &str, char_index: usize) -> usize {
    text.char_indices()
        .nth(char_index)
        .map_or(text.len(), |(byte_index, _)| byte_index)
}

/// 把整行文本的字符区间换算成字节区间（坐标系同 [`char_index_to_byte_index`]）。
pub fn char_range_to_byte_range(text: &str, range: Range<usize>) -> Range<usize> {
    char_index_to_byte_index(text, range.start)..char_index_to_byte_index(text, range.end)
}

/// 把单元格内的命中区间平移到整行文本的字符坐标。
fn match_row_range(cell_match: &FindMatch) -> Range<usize> {
    let start = cell_match.row_offset + cell_match.char_range.start;
    start..cell_match.row_offset + cell_match.char_range.end
}

/// 把行内的第 `current_in_row` 个命中标记为当前命中。
///
/// `None` 表示当前命中不在这一行，此时不做任何标记。
pub fn mark_current_match_at(matches: &mut [FindMatch], current_in_row: Option<usize>) {
    let Some(target) = current_in_row else {
        for cell_match in matches.iter_mut() {
            cell_match.is_current = false;
        }
        return;
    };
    for (index, cell_match) in matches.iter_mut().enumerate() {
        cell_match.is_current = index == target;
    }
}

/// 首尾相接的命中区间在整行文本中的合并段。
///
/// 例如 `aaa` 查 `a` 会得到 3 个相邻命中，合并成一段矩形可以
/// 避免逐字符绘制时圆角与半透明边界互相叠加、看起来颜色变深。
pub fn row_highlight_ranges(matches: &[FindMatch], char_count: usize) -> Vec<(Range<usize>, bool)> {
    let mut candidates: Vec<(Range<usize>, bool)> = matches
        .iter()
        .map(|cell_match| (match_row_range(cell_match), cell_match.is_current))
        .collect();
    candidates.sort_by_key(|(range, _)| range.start);

    let mut ranges: Vec<(Range<usize>, bool)> = Vec::new();
    for (range, is_current) in candidates {
        let start = range.start.min(char_count);
        let end = range.end.min(char_count);
        if start >= end {
            continue;
        }
        match ranges.last_mut() {
            // 首尾相接的命中（同一单元格内连续命中）合成一段。
            Some((previous, previous_current)) if previous.end == start => {
                previous.end = end;
                *previous_current |= is_current;
            }
            _ => ranges.push((start..end, is_current)),
        }
    }
    ranges
}

/// 当前命中在可见文本之后时，需要向上滚动多少行。
///
/// 命中不在可见区时才移动视口，并且只把目标行停靠到视口顶/底，
/// 避免每次输入都让表格跳动。
pub fn scroll_target_for_match(
    match_row: usize,
    visible: &Range<usize>,
    current_top: usize,
    rows_count: usize,
) -> Option<usize> {
    if rows_count == 0 || visible.is_empty() {
        return None;
    }
    if visible.contains(&match_row) {
        return None;
    }
    if match_row < visible.start {
        return Some(match_row);
    }
    let page = visible.len();
    Some(match_row.min(rows_count.saturating_sub(page)))
        .filter(|target| *target != current_top)
}

/// 命中高亮的背景色。
///
/// 当前命中用醒目的琥珀色，其余命中沿用主题的选区色但提高不透明度：
/// 表格背景会随斑马纹和悬浮变化，高亮太透会让深色主题下几乎看不见。
pub fn find_highlight_color(selection: Hsla, is_current: bool) -> Hsla {
    if is_current {
        return hsla(
            CURRENT_MATCH_HUE,
            CURRENT_MATCH_SATURATION,
            CURRENT_MATCH_LIGHTNESS,
            CURRENT_MATCH_ALPHA,
        );
    }
    let mut color = selection;
    color.a = MATCH_ALPHA;
    color
}

/// 当前命中的色相（琥珀色），与终端搜索高亮保持一致的观感。
const CURRENT_MATCH_HUE: f32 = 0.11;
const CURRENT_MATCH_SATURATION: f32 = 0.95;
const CURRENT_MATCH_LIGHTNESS: f32 = 0.55;
const CURRENT_MATCH_ALPHA: f32 = 0.9;
const MATCH_ALPHA: f32 = 0.45;

/// 命中矩形的圆角，让连续命中看起来是一个整体。
pub const MATCH_RADIUS: Pixels = px(2.);

#[cfg(test)]
mod find_tests;

#[cfg(test)]
mod highlight_geometry_tests;

#[cfg(test)]
mod find_keyboard_tests;
