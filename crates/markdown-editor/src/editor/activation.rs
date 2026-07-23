use super::MarkdownEditor;
use gpui::{Context, Window};
use markdown_source::{SourceBlock, SourceBlockKind, SourceNodeId};

impl MarkdownEditor {
    pub(super) fn activate_block_line(
        &mut self,
        block_id: SourceNodeId,
        line_index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(block) = self
            .history
            .document()
            .blocks
            .iter()
            .find(|block| block.id == block_id)
        else {
            return false;
        };
        let cursor = line_content_offset(block, line_index);
        self.active_block = Some(block_id);
        self.active_table_cell = None;
        self.sync_projection(cursor, window, cx);
        self.input.update(cx, |input, cx| input.focus(window, cx));
        true
    }
}

fn line_content_offset(block: &SourceBlock, requested_index: usize) -> usize {
    if let Some(content) = block.content_range.as_ref() {
        return content_line_offset(block, content, requested_index);
    }
    let lines = block
        .original_source
        .split_inclusive('\n')
        .collect::<Vec<_>>();
    let index = requested_index.min(lines.len().saturating_sub(1));
    let line_start = lines
        .iter()
        .take(index)
        .map(|line| line.len())
        .sum::<usize>();
    let line = lines.get(index).copied().unwrap_or_default();
    block.source_range.start + line_start + structural_marker_len(line, &block.kind)
}

fn content_line_offset(
    block: &SourceBlock,
    content: &std::ops::Range<usize>,
    requested_index: usize,
) -> usize {
    let start = content
        .start
        .saturating_sub(block.source_range.start)
        .min(block.original_source.len());
    let end = content
        .end
        .saturating_sub(block.source_range.start)
        .min(block.original_source.len());
    let source = block.original_source.get(start..end).unwrap_or_default();
    let lines = source.split_inclusive('\n').collect::<Vec<_>>();
    let index = requested_index.min(lines.len().saturating_sub(1));
    content.start
        + lines
            .iter()
            .take(index)
            .map(|line| line.len())
            .sum::<usize>()
}

fn structural_marker_len(line: &str, kind: &SourceBlockKind) -> usize {
    let indent = line.len().saturating_sub(line.trim_start().len());
    let trimmed = &line[indent..];
    match kind {
        SourceBlockKind::BlockQuote => indent + quote_marker_len(trimmed),
        SourceBlockKind::UnorderedList => {
            unordered_marker_len(trimmed).map_or(indent, |len| indent + len)
        }
        SourceBlockKind::OrderedList { .. } => {
            ordered_marker_len(trimmed).map_or(indent, |len| indent + len)
        }
        _ => indent,
    }
}

fn quote_marker_len(line: &str) -> usize {
    line.bytes()
        .take_while(|byte| matches!(byte, b'>' | b' '))
        .count()
}

fn unordered_marker_len(line: &str) -> Option<usize> {
    let rest = line
        .strip_prefix("- ")
        .or_else(|| line.strip_prefix("* "))
        .or_else(|| line.strip_prefix("+ "))?;
    let content = rest
        .strip_prefix("[ ] ")
        .or_else(|| rest.strip_prefix("[x] "))
        .or_else(|| rest.strip_prefix("[X] "));
    Some(content.map_or(line.len() - rest.len(), |value| line.len() - value.len()))
}

fn ordered_marker_len(line: &str) -> Option<usize> {
    let whitespace = line.find(char::is_whitespace)?;
    let marker = &line[..whitespace];
    let delimiter = marker.chars().last()?;
    let number = &marker[..marker.len().saturating_sub(delimiter.len_utf8())];
    if !matches!(delimiter, '.' | ')') || number.parse::<u64>().is_err() {
        return None;
    }
    Some(whitespace + line[whitespace..].len() - line[whitespace..].trim_start().len())
}
