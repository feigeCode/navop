use crate::SourceInlineNode;
use crate::inline::InlineMapper;
use std::ops::Range;

const TABLE_DELIMITER_ROW_INDEX: usize = 1;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTableMap {
    pub table_range: Range<usize>,
    pub rows: Vec<SourceTableRow>,
    pub delimiter_row: Range<usize>,
    pub leading_pipe: bool,
    pub trailing_pipe: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTableRow {
    pub full_range: Range<usize>,
    pub cells: Vec<SourceTableCell>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceTableCell {
    pub full_range: Range<usize>,
    pub content_range: Range<usize>,
    pub original_source: String,
    pub inline_nodes: Vec<SourceInlineNode>,
}

pub(crate) fn build_table_map(
    source: &str,
    table_range: Range<usize>,
    next_id: &mut u64,
) -> SourceTableMap {
    let line_ranges = line_ranges(source, table_range.clone());
    let rows = line_ranges
        .iter()
        .map(|range| build_row(source, range.clone(), next_id))
        .collect();
    let first = line_ranges.first().cloned().unwrap_or(table_range.clone());
    let first_source = &source[first];
    SourceTableMap {
        table_range,
        rows,
        delimiter_row: line_ranges
            .get(TABLE_DELIMITER_ROW_INDEX)
            .cloned()
            .unwrap_or(0..0),
        leading_pipe: first_source.trim_start().starts_with('|'),
        trailing_pipe: first_source.trim_end().ends_with('|'),
    }
}

fn line_ranges(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let mut result = Vec::new();
    let mut start = range.start;
    while start < range.end {
        let relative_end = source[start..range.end].find('\n');
        let raw_end = relative_end.map_or(range.end, |offset| start + offset);
        let end = raw_end
            .checked_sub(1)
            .filter(|index| source.as_bytes().get(*index) == Some(&b'\r'))
            .unwrap_or(raw_end);
        result.push(start..end);
        start = raw_end.saturating_add(1);
    }
    if result.is_empty() {
        result.push(range);
    }
    result
}

fn build_row(source: &str, range: Range<usize>, next_id: &mut u64) -> SourceTableRow {
    let cells = cell_ranges(source, range.clone())
        .into_iter()
        .map(|full_range| build_cell(source, full_range, next_id))
        .collect();
    SourceTableRow {
        full_range: range,
        cells,
    }
}

fn build_cell(source: &str, full_range: Range<usize>, next_id: &mut u64) -> SourceTableCell {
    let content_range = trim_range(source, full_range.clone());
    let inline_nodes = parse_cell_inlines(source, content_range.clone(), next_id);
    SourceTableCell {
        original_source: source[full_range.clone()].to_owned(),
        full_range,
        content_range,
        inline_nodes,
    }
}

fn parse_cell_inlines(
    source: &str,
    content_range: Range<usize>,
    next_id: &mut u64,
) -> Vec<SourceInlineNode> {
    if content_range.is_empty() {
        return Vec::new();
    }
    let Ok(tree) = markdown::to_mdast(
        &source[content_range.clone()],
        &crate::parser::parse_options(),
    ) else {
        return Vec::new();
    };
    let Some(root) = tree.children() else {
        return Vec::new();
    };
    let Some(children) = root.first().and_then(markdown::mdast::Node::children) else {
        return Vec::new();
    };
    InlineMapper::new(source, content_range.start, next_id).collect(children)
}

fn cell_ranges(source: &str, range: Range<usize>) -> Vec<Range<usize>> {
    let raw = &source[range.clone()];
    let separators = pipe_offsets(raw);
    let leading = raw.trim_start().starts_with('|');
    let trailing = raw.trim_end().ends_with('|');
    let mut cells = Vec::new();
    let mut start = leading
        .then(|| separators.first().copied().unwrap_or(0) + 1)
        .unwrap_or(0);
    let skip = usize::from(leading);
    for separator in separators.iter().copied().skip(skip) {
        cells.push(range.start + start..range.start + separator);
        start = separator + 1;
    }
    if !trailing {
        cells.push(range.start + start..range.end);
    }
    cells
}

fn pipe_offsets(value: &str) -> Vec<usize> {
    value
        .bytes()
        .enumerate()
        .filter_map(|(index, byte)| {
            (byte == b'|' && !is_escaped(value.as_bytes(), index)).then_some(index)
        })
        .collect()
}

fn is_escaped(bytes: &[u8], index: usize) -> bool {
    let slash_count = bytes[..index]
        .iter()
        .rev()
        .take_while(|byte| **byte == b'\\')
        .count();
    slash_count % 2 == 1
}

fn trim_range(source: &str, range: Range<usize>) -> Range<usize> {
    let raw = &source[range.clone()];
    let leading = raw.len() - raw.trim_start().len();
    let trailing = raw.len() - raw.trim_end().len();
    range.start + leading..range.end - trailing
}
