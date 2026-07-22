use super::SourceOperationError;
use crate::{
    SourceBlock, SourceBlockKind, SourceEditOrigin, SourceMarkdownDocument, SourceNodeId,
    SourceTransaction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockMoveDirection {
    Up,
    Down,
}

impl SourceMarkdownDocument {
    pub fn delete_block(
        &self,
        block_id: SourceNodeId,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let index = block_index(self, block_id)?;
        let range = if let Some(next) = self.blocks.get(index + 1) {
            self.blocks[index].source_range.start..next.source_range.start
        } else if let Some(previous) = index.checked_sub(1).and_then(|i| self.blocks.get(i)) {
            previous.source_range.end..self.blocks[index].source_range.end
        } else {
            self.blocks[index].source_range.clone()
        };
        Ok(self.single_edit(range, "", SourceEditOrigin::DeleteBlock))
    }

    pub fn move_block(
        &self,
        block_id: SourceNodeId,
        direction: BlockMoveDirection,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let index = block_index(self, block_id)?;
        let other = match direction {
            BlockMoveDirection::Up => index.checked_sub(1),
            BlockMoveDirection::Down => (index + 1 < self.blocks.len()).then_some(index + 1),
        }
        .ok_or(SourceOperationError::CannotMoveBlock)?;
        let (first, second) = if index < other {
            (index, other)
        } else {
            (other, index)
        };
        let left = &self.blocks[first];
        let right = &self.blocks[second];
        let gap = &self.source[left.source_range.end..right.source_range.start];
        let replacement = format!("{}{}{}", right.original_source, gap, left.original_source);
        Ok(self.single_edit(
            left.source_range.start..right.source_range.end,
            replacement,
            SourceEditOrigin::MoveBlock,
        ))
    }

    pub fn split_block(
        &self,
        block_id: SourceNodeId,
        byte_offset: usize,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = find_block(self, block_id)?;
        if byte_offset < block.source_range.start || byte_offset > block.source_range.end {
            return Err(SourceOperationError::CannotSplitBlock);
        }
        let separator = split_separator(&self.source, block, byte_offset)?;
        Ok(self.single_edit(
            byte_offset..byte_offset,
            separator,
            SourceEditOrigin::InsertBlock,
        ))
    }

    pub fn toggle_blockquote(
        &self,
        block_id: SourceNodeId,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = find_block(self, block_id)?;
        let replacement = if matches!(block.kind, SourceBlockKind::BlockQuote) {
            block
                .original_source
                .lines()
                .map(strip_quote_marker)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            block
                .original_source
                .lines()
                .map(|line| format!("> {line}"))
                .collect::<Vec<_>>()
                .join("\n")
        };
        Ok(self.single_edit(
            block.source_range.clone(),
            replacement,
            SourceEditOrigin::Formatting,
        ))
    }

    pub fn toggle_code_fence(
        &self,
        block_id: SourceNodeId,
        language: Option<&str>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let block = find_block(self, block_id)?;
        let replacement = if matches!(block.kind, SourceBlockKind::CodeFence { .. }) {
            block
                .content_range
                .as_ref()
                .map(|range| self.source[range.clone()].to_owned())
                .unwrap_or_default()
        } else {
            format!(
                "```{}\n{}\n```",
                language.unwrap_or_default(),
                block.original_source
            )
        };
        Ok(self.single_edit(
            block.source_range.clone(),
            replacement,
            SourceEditOrigin::Formatting,
        ))
    }
}

fn block_index(
    document: &SourceMarkdownDocument,
    block_id: SourceNodeId,
) -> Result<usize, SourceOperationError> {
    document
        .blocks
        .iter()
        .position(|block| block.id == block_id)
        .ok_or(SourceOperationError::NodeNotFound)
}

fn find_block(
    document: &SourceMarkdownDocument,
    block_id: SourceNodeId,
) -> Result<&SourceBlock, SourceOperationError> {
    document
        .blocks
        .iter()
        .find(|block| block.id == block_id)
        .ok_or(SourceOperationError::NodeNotFound)
}

fn split_separator(
    source: &str,
    block: &SourceBlock,
    byte_offset: usize,
) -> Result<String, SourceOperationError> {
    match block.kind {
        SourceBlockKind::OrderedList { .. } | SourceBlockKind::UnorderedList => {
            list_item_separator(source, block, byte_offset)
        }
        SourceBlockKind::BlockQuote => Ok("\n> ".to_owned()),
        SourceBlockKind::CodeFence { .. } => Ok("\n".to_owned()),
        SourceBlockKind::Paragraph | SourceBlockKind::Heading { .. } => Ok("\n\n".to_owned()),
        _ => Err(SourceOperationError::CannotSplitBlock),
    }
}

fn list_item_separator(
    source: &str,
    block: &SourceBlock,
    byte_offset: usize,
) -> Result<String, SourceOperationError> {
    let line_start = source[block.source_range.start..byte_offset]
        .rfind('\n')
        .map_or(block.source_range.start, |offset| {
            block.source_range.start + offset + 1
        });
    let line = &source[line_start..byte_offset];
    let indent_len = line.len() - line.trim_start().len();
    let trimmed = &line[indent_len..];
    let marker_end = trimmed
        .find(char::is_whitespace)
        .ok_or(SourceOperationError::CannotSplitBlock)?;
    let marker = &trimmed[..marker_end];
    let next_marker = ordered_next_marker(marker).unwrap_or_else(|| marker.to_owned());
    Ok(format!("\n{}{} ", &line[..indent_len], next_marker))
}

fn ordered_next_marker(marker: &str) -> Option<String> {
    let delimiter = marker.chars().last()?;
    if !matches!(delimiter, '.' | ')') {
        return None;
    }
    let value = marker[..marker.len() - delimiter.len_utf8()]
        .parse::<u64>()
        .ok()?;
    Some(format!("{}{delimiter}", value.saturating_add(1)))
}

fn strip_quote_marker(line: &str) -> &str {
    line.strip_prefix("> ")
        .or_else(|| line.strip_prefix('>'))
        .unwrap_or(line)
}
