use super::SourceOperationError;
use crate::{
    SourceBlockKind, SourceEditOrigin, SourceMarkdownDocument, SourceNodeId, SourceTransaction,
};
use std::ops::Range;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineFormat {
    Bold,
    Italic,
    Underline,
    Strike,
    Code,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ListFormat {
    Bullet,
    Ordered,
    Task,
}

impl SourceMarkdownDocument {
    pub fn toggle_inline_format(
        &self,
        range: Range<usize>,
        format: InlineFormat,
    ) -> Result<SourceTransaction, SourceOperationError> {
        if range.is_empty() || range.end > self.source.len() {
            return Err(SourceOperationError::CannotFormatSelection);
        }
        let (opening, closing) = inline_markers(format);
        let selected = &self.source[range.clone()];
        let replacement = selected
            .strip_prefix(opening)
            .and_then(|value| value.strip_suffix(closing))
            .map_or_else(|| format!("{opening}{selected}{closing}"), str::to_owned);
        Ok(self.single_edit(range, replacement, SourceEditOrigin::Formatting))
    }

    pub fn set_block_heading(
        &self,
        block_id: SourceNodeId,
        level: Option<u8>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = self
            .block_by_id(block_id)
            .ok_or(SourceOperationError::NodeNotFound)?;
        let content = block
            .content_range
            .as_ref()
            .map(|range| self.source[range.clone()].to_owned())
            .unwrap_or_else(|| block.original_source.clone());
        let replacement = level.map_or(content.clone(), |level| {
            format!("{} {content}", "#".repeat(level.clamp(1, 6) as usize))
        });
        Ok(self.single_edit(
            block.source_range.clone(),
            replacement,
            SourceEditOrigin::Formatting,
        ))
    }

    pub fn toggle_list_format(
        &self,
        block_id: SourceNodeId,
        format: ListFormat,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = self
            .block_by_id(block_id)
            .ok_or(SourceOperationError::NodeNotFound)?;
        let already_active = matches!(
            (&block.kind, format),
            (
                SourceBlockKind::UnorderedList,
                ListFormat::Bullet | ListFormat::Task
            ) | (SourceBlockKind::OrderedList { .. }, ListFormat::Ordered)
        );
        let replacement = block
            .original_source
            .lines()
            .map(|line| list_line(line, format, already_active))
            .collect::<Vec<_>>()
            .join("\n");
        Ok(self.single_edit(
            block.source_range.clone(),
            replacement,
            SourceEditOrigin::Formatting,
        ))
    }

    pub fn toggle_task_checked(
        &self,
        state_range: Range<usize>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let checked = task_marker_checked(&self.source, &state_range)
            .ok_or(SourceOperationError::NotTaskMarker)?;
        let block = self
            .block_at(state_range.start)
            .filter(|block| matches!(block.kind, SourceBlockKind::UnorderedList))
            .filter(|block| state_range.end <= block.source_range.end)
            .ok_or(SourceOperationError::NotTaskMarker)?;
        let expected_range =
            task_state_range_on_line(&self.source, block.source_range.start, state_range.start)
                .ok_or(SourceOperationError::NotTaskMarker)?;
        if expected_range != state_range {
            return Err(SourceOperationError::NotTaskMarker);
        }
        Ok(self.single_edit(
            state_range,
            if checked { " " } else { "x" },
            SourceEditOrigin::Formatting,
        ))
    }

    pub fn duplicate_block(
        &self,
        block_id: SourceNodeId,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = self
            .block_by_id(block_id)
            .ok_or(SourceOperationError::NodeNotFound)?;
        Ok(self.single_edit(
            block.source_range.end..block.source_range.end,
            format!("\n\n{}", block.original_source),
            SourceEditOrigin::InsertBlock,
        ))
    }
}

fn inline_markers(format: InlineFormat) -> (&'static str, &'static str) {
    match format {
        InlineFormat::Bold => ("**", "**"),
        InlineFormat::Italic => ("_", "_"),
        InlineFormat::Underline => ("<u>", "</u>"),
        InlineFormat::Strike => ("~~", "~~"),
        InlineFormat::Code => ("`", "`"),
    }
}

fn list_line(line: &str, format: ListFormat, remove: bool) -> String {
    let content = strip_list_marker(line);
    if remove {
        return content.to_owned();
    }
    let marker = match format {
        ListFormat::Bullet => "- ",
        ListFormat::Ordered => "1. ",
        ListFormat::Task => "- [ ] ",
    };
    format!("{marker}{content}")
}

fn strip_list_marker(line: &str) -> &str {
    let trimmed = line.trim_start();
    let indent = &line[..line.len() - trimmed.len()];
    let content = trimmed
        .strip_prefix("- [ ] ")
        .or_else(|| trimmed.strip_prefix("- [x] "))
        .or_else(|| trimmed.strip_prefix("- [X] "))
        .or_else(|| trimmed.strip_prefix("- "))
        .or_else(|| trimmed.strip_prefix("* "))
        .or_else(|| trimmed.strip_prefix("+ "))
        .or_else(|| strip_ordered_marker(trimmed))
        .unwrap_or(trimmed);
    content.strip_prefix(indent).unwrap_or(content)
}

fn strip_ordered_marker(line: &str) -> Option<&str> {
    let marker_end = line.find(char::is_whitespace)?;
    let marker = &line[..marker_end];
    let delimiter = marker.chars().last()?;
    matches!(delimiter, '.' | ')')
        .then(|| marker[..marker.len() - 1].parse::<u64>().ok())
        .flatten()?;
    Some(line[marker_end..].trim_start())
}

fn task_marker_checked(source: &str, state_range: &Range<usize>) -> Option<bool> {
    if state_range.end != state_range.start.checked_add(1)?
        || state_range.end > source.len()
        || !source.is_char_boundary(state_range.start)
        || !source.is_char_boundary(state_range.end)
    {
        return None;
    }
    match source.as_bytes()[state_range.start] {
        b' ' => Some(false),
        b'x' | b'X' => Some(true),
        _ => None,
    }
}

fn task_state_range_on_line(
    source: &str,
    block_start: usize,
    state_offset: usize,
) -> Option<Range<usize>> {
    let line_start = source[block_start..state_offset]
        .rfind('\n')
        .map_or(block_start, |offset| block_start + offset + 1);
    let before_state = &source[line_start..state_offset];
    let indent = before_state
        .len()
        .saturating_sub(before_state.trim_start().len());
    let marker = &before_state[indent..];
    let marker = marker
        .strip_prefix("- ")
        .or_else(|| marker.strip_prefix("* "))
        .or_else(|| marker.strip_prefix("+ "))?;
    let suffix = source.as_bytes().get(state_offset + 2).copied();
    (marker == "["
        && source.as_bytes().get(state_offset + 1) == Some(&b']')
        && suffix.is_none_or(|byte| matches!(byte, b' ' | b'\t' | b'\r' | b'\n')))
    .then_some(state_offset..state_offset + 1)
}
