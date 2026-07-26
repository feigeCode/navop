use crate::{
    SourceBlockKind, SourceEdit, SourceEditOrigin, SourceImageMap, SourceInlineKind,
    SourceMarkdownDocument, SourceNodeId, SourceTransaction,
};
use std::ops::Range;

mod block_operations;
mod format_operations;
mod table_operations;
pub use block_operations::BlockMoveDirection;
pub use format_operations::{InlineFormat, ListFormat};
pub use table_operations::{TableAlignment, TableInsertPosition};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableCellAddress {
    pub block_id: SourceNodeId,
    pub row: usize,
    pub column: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SourceOperationError {
    #[error("Markdown source node was not found")]
    NodeNotFound,
    #[error("Markdown source node is not a table")]
    NotTable,
    #[error("Markdown table cell was not found")]
    CellNotFound,
    #[error("Markdown table operation would remove the last column")]
    CannotDeleteLastTableColumn,
    #[error("Markdown table size must contain at least one row and one column")]
    InvalidTableSize,
    #[error("Markdown source node is not an image")]
    NotImage,
    #[error("Markdown block cannot move in that direction")]
    CannotMoveBlock,
    #[error("Markdown block cannot be split at that position")]
    CannotSplitBlock,
    #[error("Markdown selection cannot be formatted")]
    CannotFormatSelection,
    #[error("Markdown source range is not a task-list checkbox")]
    NotTaskMarker,
}

impl SourceMarkdownDocument {
    pub fn table_cell(
        &self,
        address: TableCellAddress,
    ) -> Result<&crate::SourceTableCell, SourceOperationError> {
        let block = self
            .blocks
            .iter()
            .find(|block| block.id == address.block_id)
            .ok_or(SourceOperationError::NodeNotFound)?;
        let SourceBlockKind::Table(table) = &block.kind else {
            return Err(SourceOperationError::NotTable);
        };
        table
            .rows
            .get(address.row)
            .and_then(|row| row.cells.get(address.column))
            .ok_or(SourceOperationError::CellNotFound)
    }

    pub fn edit_table_cell(
        &self,
        address: TableCellAddress,
        replacement: impl Into<String>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let cell = self.table_cell(address)?;
        Ok(self.single_edit(
            cell.content_range.clone(),
            replacement,
            SourceEditOrigin::TableCellEdit,
        ))
    }

    pub fn clear_table_cell(
        &self,
        address: TableCellAddress,
    ) -> Result<SourceTransaction, SourceOperationError> {
        self.edit_table_cell(address, "")
    }

    pub fn edit_image_alt(
        &self,
        node_id: SourceNodeId,
        replacement: impl Into<String>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let image = self.image_map(node_id)?;
        Ok(self.single_edit(
            image.alt_range.clone(),
            replacement,
            SourceEditOrigin::ImageEdit,
        ))
    }

    pub fn edit_image(
        &self,
        node_id: SourceNodeId,
        alt: impl Into<String>,
        destination: impl Into<String>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let image = self.image_map(node_id)?;
        let alt_range = image.alt_range.clone();
        let destination_range = image.destination_range.clone();
        Ok(SourceTransaction {
            edits: vec![
                SourceEdit::new(alt_range.clone(), alt, self.revision),
                SourceEdit::new(destination_range.clone(), destination, self.revision),
            ],
            allowed_ranges: vec![alt_range, destination_range],
            origin: SourceEditOrigin::ImageEdit,
            selection_before: crate::SourceSelection::default(),
            selection_after: crate::SourceSelection::default(),
        })
    }

    pub fn edit_image_destination(
        &self,
        node_id: SourceNodeId,
        replacement: impl Into<String>,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let image = self.image_map(node_id)?;
        Ok(self.single_edit(
            image.destination_range.clone(),
            replacement,
            SourceEditOrigin::ImageEdit,
        ))
    }

    pub fn delete_image(
        &self,
        node_id: SourceNodeId,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let image = self.image_map(node_id)?;
        Ok(self.single_edit(image.full_range.clone(), "", SourceEditOrigin::ImageEdit))
    }

    pub fn image_map(
        &self,
        node_id: SourceNodeId,
    ) -> Result<&SourceImageMap, SourceOperationError> {
        let node = self
            .blocks
            .iter()
            .flat_map(|block| block.inline_nodes.iter())
            .chain(self.table_inline_nodes())
            .find(|node| node.id == node_id)
            .ok_or(SourceOperationError::NodeNotFound)?;
        match &node.kind {
            SourceInlineKind::Image(image) => Ok(image),
            _ => Err(SourceOperationError::NotImage),
        }
    }

    fn table_inline_nodes(&self) -> impl Iterator<Item = &crate::SourceInlineNode> {
        self.blocks.iter().flat_map(|block| match &block.kind {
            SourceBlockKind::Table(table) => table
                .rows
                .iter()
                .flat_map(|row| row.cells.iter())
                .flat_map(|cell| cell.inline_nodes.iter())
                .collect::<Vec<_>>(),
            _ => Vec::new(),
        })
    }

    pub(super) fn single_edit(
        &self,
        range: Range<usize>,
        replacement: impl Into<String>,
        origin: SourceEditOrigin,
    ) -> SourceTransaction {
        SourceTransaction {
            edits: vec![SourceEdit::new(range.clone(), replacement, self.revision)],
            allowed_ranges: vec![range],
            origin,
            selection_before: crate::SourceSelection::default(),
            selection_after: crate::SourceSelection::default(),
        }
    }
}
