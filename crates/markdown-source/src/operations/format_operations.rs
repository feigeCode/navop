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
