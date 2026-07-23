use markdown_source::{SourceBlock, SourceBlockKind};
use std::ops::Range;

pub(super) fn block_hidden_ranges(source: &str, block: &SourceBlock) -> Vec<Range<usize>> {
    match &block.kind {
        SourceBlockKind::Heading { marker_range, .. } => {
            let end = block
                .content_range
                .as_ref()
                .map_or(marker_range.end, |range| range.start);
            vec![marker_range.start..end]
        }
        SourceBlockKind::BlockQuote => line_marker_ranges(source, block, quote_marker_len),
        SourceBlockKind::OrderedList { .. } | SourceBlockKind::UnorderedList => {
            list_marker_ranges(source, block)
        }
        SourceBlockKind::CodeFence { .. } => block
            .content_range
            .as_ref()
            .map(|content| {
                vec![
                    block.source_range.start..content.start,
                    content.end..block.source_range.end,
                ]
            })
            .unwrap_or_default(),
        SourceBlockKind::MathBlock {
            opening_marker,
            closing_marker,
        } => vec![
            opening_marker.start
                ..block
                    .content_range
                    .as_ref()
                    .map_or(opening_marker.end, |content| content.start),
            block
                .content_range
                .as_ref()
                .map_or(closing_marker.start, |content| content.end)
                ..closing_marker.end,
        ],
        _ => Vec::new(),
    }
}

fn list_marker_ranges(source: &str, block: &SourceBlock) -> Vec<Range<usize>> {
    let mut line_start = block.source_range.start;
    source[block.source_range.clone()]
        .split_inclusive('\n')
        .filter_map(|line| {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let indent = content.len().saturating_sub(content.trim_start().len());
            let length = list_marker_len(content);
            let range = (length > indent).then_some(line_start + indent..line_start + length);
            line_start += line.len();
            range
        })
        .collect()
}

fn line_marker_ranges(
    source: &str,
    block: &SourceBlock,
    marker_len: fn(&str) -> usize,
) -> Vec<Range<usize>> {
    let mut line_start = block.source_range.start;
    source[block.source_range.clone()]
        .split_inclusive('\n')
        .filter_map(|line| {
            let content = line.strip_suffix('\n').unwrap_or(line);
            let length = marker_len(content);
            let range = (length > 0).then_some(line_start..line_start + length);
            line_start += line.len();
            range
        })
        .collect()
}

fn quote_marker_len(line: &str) -> usize {
    let indent = line.len() - line.trim_start().len();
    let mut rest = &line[indent..];
    let mut length = indent;
    while let Some(quoted) = rest.strip_prefix('>') {
        length += 1;
        rest = quoted;
        if let Some(spaced) = rest.strip_prefix(' ') {
            length += 1;
            rest = spaced;
        }
    }
    (length > indent).then_some(length).unwrap_or_default()
}

fn list_marker_len(line: &str) -> usize {
    let indent = line.len() - line.trim_start().len();
    let trimmed = &line[indent..];
    let marker_len = unordered_marker_len(trimmed).or_else(|| ordered_marker_len(trimmed));
    marker_len.map_or(0, |length| indent + length)
}

fn unordered_marker_len(line: &str) -> Option<usize> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))?;
    let base = line.len() - rest.len();
    let task = rest
        .strip_prefix("[ ] ")
        .or_else(|| rest.strip_prefix("[x] "))
        .or_else(|| rest.strip_prefix("[X] "));
    Some(task.map_or(base, |content| line.len() - content.len()))
}

fn ordered_marker_len(line: &str) -> Option<usize> {
    let whitespace = line.find(char::is_whitespace)?;
    let marker = &line[..whitespace];
    let delimiter = marker.chars().last()?;
    let number = &marker[..marker.len() - delimiter.len_utf8()];
    if !matches!(delimiter, '.' | ')') || number.parse::<u64>().is_err() {
        return None;
    }
    let spacing = line[whitespace..].len() - line[whitespace..].trim_start().len();
    Some(whitespace + spacing)
}
