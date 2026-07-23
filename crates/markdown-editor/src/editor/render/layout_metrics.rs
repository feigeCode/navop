use super::active_block::active_block_height;
use gpui::{Pixels, Size, px};
use markdown_source::{SourceBlock, SourceBlockKind};

pub(super) const DOCUMENT_MAX_WIDTH: f32 = 780.;
pub(super) const DOCUMENT_SIDE_PADDING: f32 = 44.;
pub(super) const DOCUMENT_TOP_PADDING: f32 = 38.;
pub(super) const DOCUMENT_BOTTOM_PADDING: f32 = 80.;
const APPROXIMATE_TEXT_COLUMNS: usize = 72;
const VIRTUALIZATION_HEIGHT_THRESHOLD: f32 = 12_000.;
const VIRTUALIZATION_BLOCK_THRESHOLD: usize = 80;

pub(crate) fn should_virtualize(blocks: &[SourceBlock]) -> bool {
    blocks.len() >= VIRTUALIZATION_BLOCK_THRESHOLD
        || blocks
            .iter()
            .map(|block| block_size(block).height.as_f32())
            .sum::<f32>()
            >= VIRTUALIZATION_HEIGHT_THRESHOLD
}

pub(super) fn virtual_item_sizes(
    blocks: &[SourceBlock],
    measured: &std::collections::HashMap<markdown_source::SourceNodeId, Pixels>,
) -> Vec<Size<Pixels>> {
    blocks
        .iter()
        .enumerate()
        .map(|(index, block)| {
            let mut size = measured
                .get(&block.id)
                .copied()
                .map(|height| gpui::size(px(0.), height))
                .unwrap_or_else(|| block_size(block));
            if index == 0 {
                size.height += px(DOCUMENT_TOP_PADDING);
            }
            if index + 1 == blocks.len() {
                size.height += px(DOCUMENT_BOTTOM_PADDING);
            }
            size
        })
        .collect()
}

pub(super) fn block_size(block: &SourceBlock) -> Size<Pixels> {
    let lines = estimated_visual_lines(&block.original_source) as f32;
    let preview_height = preview_height(block, lines);
    let height = preview_height.max(estimated_active_height(block, lines));
    gpui::size(px(0.), px(height))
}

fn preview_height(block: &SourceBlock, lines: f32) -> f32 {
    match &block.kind {
        SourceBlockKind::Heading { level, .. } => match level {
            1 => 58.,
            2 => 48.,
            3 => 40.,
            _ => 34.,
        },
        SourceBlockKind::Table(table) => estimated_table_height(table),
        SourceBlockKind::MathBlock { .. } => 230.,
        SourceBlockKind::CodeFence { .. } if is_mermaid(block) => 260.,
        SourceBlockKind::CodeFence { .. }
        | SourceBlockKind::FrontMatter
        | SourceBlockKind::Html
        | SourceBlockKind::RawMarkdown => lines.mul_add(23., 18.),
        SourceBlockKind::OrderedList { .. }
        | SourceBlockKind::UnorderedList
        | SourceBlockKind::BlockQuote => lines.mul_add(25., 6.),
        _ => lines.mul_add(24., 6.),
    }
}

fn estimated_visual_lines(source: &str) -> usize {
    source
        .lines()
        .map(|line| {
            let columns = line.chars().map(approximate_char_columns).sum::<usize>();
            columns.max(1).div_ceil(APPROXIMATE_TEXT_COLUMNS)
        })
        .sum::<usize>()
        .max(1)
}

fn estimated_table_height(table: &markdown_source::SourceTableMap) -> f32 {
    let columns = table
        .rows
        .iter()
        .map(|row| row.cells.len())
        .max()
        .unwrap_or(1)
        .max(1);
    let cell_columns = (APPROXIMATE_TEXT_COLUMNS / columns).max(8);
    table
        .rows
        .iter()
        .enumerate()
        .filter(|(index, _)| *index != 1)
        .map(|(_, row)| {
            let lines = row
                .cells
                .iter()
                .map(|cell| estimated_lines_for_width(cell.original_source.trim(), cell_columns))
                .max()
                .unwrap_or(1);
            lines as f32 * 24. + 16.
        })
        .sum::<f32>()
        + 52.
}

fn estimated_lines_for_width(source: &str, columns: usize) -> usize {
    source
        .lines()
        .map(|line| {
            line.chars()
                .map(approximate_char_columns)
                .sum::<usize>()
                .max(1)
                .div_ceil(columns)
        })
        .sum::<usize>()
        .max(1)
}

fn approximate_char_columns(ch: char) -> usize {
    if ch.is_ascii() { 1 } else { 2 }
}

fn estimated_active_height(block: &SourceBlock, rows: f32) -> f32 {
    if matches!(block.kind, SourceBlockKind::Table(_)) {
        return 0.;
    }
    let heading = match block.kind {
        SourceBlockKind::Heading { level, .. } => Some(level),
        _ => None,
    };
    let source_code = matches!(
        block.kind,
        SourceBlockKind::CodeFence { .. } | SourceBlockKind::MathBlock { .. }
    );
    active_block_height(rows, heading, source_code) * 16.
}

fn is_mermaid(block: &SourceBlock) -> bool {
    code_language(block).is_some_and(|language| language.eq_ignore_ascii_case("mermaid"))
}

fn code_language(block: &SourceBlock) -> Option<&str> {
    let SourceBlockKind::CodeFence {
        language_range: Some(range),
        ..
    } = &block.kind
    else {
        return None;
    };
    let start = range.start.checked_sub(block.source_range.start)?;
    let end = range.end.checked_sub(block.source_range.start)?;
    block.original_source.get(start..end)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visual_line_estimate_accounts_for_wrapping_and_wide_text() {
        assert_eq!(1, estimated_visual_lines("short text"));
        assert_eq!(2, estimated_visual_lines(&"a".repeat(73)));
        assert_eq!(2, estimated_visual_lines(&"中".repeat(37)));
        assert_eq!(3, estimated_visual_lines("one\ntwo\nthree"));
    }

    #[test]
    fn virtual_items_include_reading_column_outer_spacing() {
        let document = markdown_source::SourceMarkdownDocument::parse("one\n\ntwo").unwrap();
        let raw = document.blocks.iter().map(block_size).collect::<Vec<_>>();
        let virtualized = virtual_item_sizes(&document.blocks, &Default::default());
        assert_eq!(
            raw[0].height + px(DOCUMENT_TOP_PADDING),
            virtualized[0].height
        );
        assert_eq!(
            raw[1].height + px(DOCUMENT_BOTTOM_PADDING),
            virtualized[1].height
        );
    }

    #[test]
    fn a_few_very_large_blocks_still_use_virtual_layout() {
        let source = (0..12)
            .map(|_| "line\n".repeat(50))
            .collect::<Vec<_>>()
            .join("\n");
        let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
        assert!(should_virtualize(&document.blocks));
    }

    #[test]
    fn table_height_accounts_for_wrapped_cell_content() {
        let source = "| 接口 | 当前用途 | 处理结论 |\n| --- | --- | --- |\n| POST /ai-manager/dashboard/publish | 从服务器本地绝对路径发布看板 | 不允许普通浏览器直接调用，并且这里有很长的中文说明需要自动换行 |";
        let document = markdown_source::SourceMarkdownDocument::parse(source).unwrap();
        let height = block_size(&document.blocks[0]).height;
        assert!(height > px(100.));
    }
}
