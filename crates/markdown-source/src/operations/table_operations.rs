use super::{SourceOperationError, TableCellAddress};
use crate::{
    SourceBlock, SourceBlockKind, SourceEditOrigin, SourceMarkdownDocument, SourceNodeId,
    SourceTableMap, SourceTableRow, SourceTransaction,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TableInsertPosition {
    Before,
    After,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TableAlignment {
    #[default]
    None,
    Left,
    Center,
    Right,
}

impl SourceMarkdownDocument {
    pub fn insert_table_row(
        &self,
        address: TableCellAddress,
        position: TableInsertPosition,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let (block, table) = table_block(self, address.block_id)?;
        let mut model = TableModel::from_source(self, table);
        let row = visible_row(address.row)?;
        validate_cell(&model, row, address.column)?;
        let index = insertion_index(row, position);
        model
            .rows
            .insert(index, vec![String::new(); model.column_count()]);
        Ok(replace_table(self, block, model))
    }

    pub fn delete_table_row(
        &self,
        address: TableCellAddress,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let (block, table) = table_block(self, address.block_id)?;
        let mut model = TableModel::from_source(self, table);
        let row = visible_row(address.row)?;
        validate_cell(&model, row, address.column)?;
        model.rows.remove(row);
        if model.rows.is_empty() {
            model.rows.push(vec![String::new(); model.column_count()]);
        }
        Ok(replace_table(self, block, model))
    }

    pub fn insert_table_column(
        &self,
        address: TableCellAddress,
        position: TableInsertPosition,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let (block, table) = table_block(self, address.block_id)?;
        let mut model = TableModel::from_source(self, table);
        let row = visible_row(address.row)?;
        validate_cell(&model, row, address.column)?;
        let index = insertion_index(address.column, position);
        for row in &mut model.rows {
            row.insert(index, String::new());
        }
        model.alignments.insert(index, TableAlignment::None);
        Ok(replace_table(self, block, model))
    }

    pub fn delete_table_column(
        &self,
        address: TableCellAddress,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let (block, table) = table_block(self, address.block_id)?;
        let mut model = TableModel::from_source(self, table);
        let row = visible_row(address.row)?;
        validate_cell(&model, row, address.column)?;
        if model.column_count() == 1 {
            return Err(SourceOperationError::CannotDeleteLastTableColumn);
        }
        for row in &mut model.rows {
            row.remove(address.column);
        }
        model.alignments.remove(address.column);
        Ok(replace_table(self, block, model))
    }

    pub fn set_table_column_alignment(
        &self,
        address: TableCellAddress,
        alignment: TableAlignment,
    ) -> Result<SourceTransaction, SourceOperationError> {
        let (block, table) = table_block(self, address.block_id)?;
        let mut model = TableModel::from_source(self, table);
        let row = visible_row(address.row)?;
        validate_cell(&model, row, address.column)?;
        model.alignments[address.column] = alignment;
        Ok(replace_table(self, block, model))
    }

    pub fn resize_table(
        &self,
        block_id: SourceNodeId,
        visible_rows: usize,
        columns: usize,
    ) -> Result<SourceTransaction, SourceOperationError> {
        if visible_rows == 0 || columns == 0 {
            return Err(SourceOperationError::InvalidTableSize);
        }
        let (block, table) = table_block(self, block_id)?;
        let mut model = TableModel::from_source(self, table);
        model.rows.resize_with(visible_rows, Vec::new);
        for row in &mut model.rows {
            row.resize(columns, String::new());
        }
        model.alignments.resize(columns, TableAlignment::None);
        model.alignments.truncate(columns);
        Ok(replace_table(self, block, model))
    }
}

struct TableModel {
    rows: Vec<Vec<String>>,
    alignments: Vec<TableAlignment>,
    leading_pipe: bool,
    trailing_pipe: bool,
    trailing_newline: bool,
}

impl TableModel {
    fn from_source(document: &SourceMarkdownDocument, table: &SourceTableMap) -> Self {
        let columns = table
            .rows
            .iter()
            .map(|row| row.cells.len())
            .max()
            .unwrap_or(1)
            .max(1);
        let mut rows = table
            .rows
            .iter()
            .enumerate()
            .filter(|(index, _)| *index != 1)
            .map(|(_, row)| row_values(document, row, columns))
            .collect::<Vec<_>>();
        if rows.is_empty() {
            rows.push(vec![String::new(); columns]);
        }
        let delimiter = table.rows.get(1);
        let alignments = (0..columns)
            .map(|column| {
                delimiter
                    .and_then(|row| row.cells.get(column))
                    .map(|cell| alignment(&document.source[cell.content_range.clone()]))
                    .unwrap_or_default()
            })
            .collect();
        Self {
            rows,
            alignments,
            leading_pipe: table.leading_pipe,
            trailing_pipe: table.trailing_pipe,
            trailing_newline: document.source[table.table_range.clone()].ends_with('\n'),
        }
    }

    fn column_count(&self) -> usize {
        self.alignments.len().max(1)
    }

    fn render(&self) -> String {
        let mut lines = Vec::with_capacity(self.rows.len() + 1);
        lines.push(render_row(
            &self.rows[0],
            self.leading_pipe,
            self.trailing_pipe,
        ));
        let delimiters = self
            .alignments
            .iter()
            .map(|alignment| delimiter(*alignment).to_owned())
            .collect::<Vec<_>>();
        lines.push(render_row(
            &delimiters,
            self.leading_pipe,
            self.trailing_pipe,
        ));
        lines.extend(
            self.rows
                .iter()
                .skip(1)
                .map(|row| render_row(row, self.leading_pipe, self.trailing_pipe)),
        );
        let mut source = lines.join("\n");
        if self.trailing_newline {
            source.push('\n');
        }
        source
    }
}

fn table_block(
    document: &SourceMarkdownDocument,
    block_id: SourceNodeId,
) -> Result<(&SourceBlock, &SourceTableMap), SourceOperationError> {
    let block = document
        .block_by_id(block_id)
        .ok_or(SourceOperationError::NodeNotFound)?;
    let SourceBlockKind::Table(table) = &block.kind else {
        return Err(SourceOperationError::NotTable);
    };
    Ok((block, table))
}

fn replace_table(
    document: &SourceMarkdownDocument,
    block: &SourceBlock,
    model: TableModel,
) -> SourceTransaction {
    document.single_edit(
        block.source_range.clone(),
        model.render(),
        SourceEditOrigin::TableStructureEdit,
    )
}

fn row_values(
    document: &SourceMarkdownDocument,
    row: &SourceTableRow,
    columns: usize,
) -> Vec<String> {
    let mut values = row
        .cells
        .iter()
        .map(|cell| document.source[cell.content_range.clone()].to_owned())
        .collect::<Vec<_>>();
    values.resize(columns, String::new());
    values
}

fn visible_row(source_row: usize) -> Result<usize, SourceOperationError> {
    match source_row {
        0 => Ok(0),
        1 => Err(SourceOperationError::CellNotFound),
        row => Ok(row - 1),
    }
}

fn validate_cell(
    model: &TableModel,
    row: usize,
    column: usize,
) -> Result<(), SourceOperationError> {
    model
        .rows
        .get(row)
        .and_then(|row| row.get(column))
        .map(|_| ())
        .ok_or(SourceOperationError::CellNotFound)
}

fn insertion_index(index: usize, position: TableInsertPosition) -> usize {
    index + usize::from(position == TableInsertPosition::After)
}

fn alignment(value: &str) -> TableAlignment {
    let value = value.trim();
    match (value.starts_with(':'), value.ends_with(':')) {
        (true, true) => TableAlignment::Center,
        (true, false) => TableAlignment::Left,
        (false, true) => TableAlignment::Right,
        (false, false) => TableAlignment::None,
    }
}

fn delimiter(alignment: TableAlignment) -> &'static str {
    match alignment {
        TableAlignment::None => "---",
        TableAlignment::Left => ":---",
        TableAlignment::Center => ":---:",
        TableAlignment::Right => "---:",
    }
}

fn render_row(cells: &[String], leading_pipe: bool, trailing_pipe: bool) -> String {
    let body = cells
        .iter()
        .map(|cell| format!(" {} ", cell.trim()))
        .collect::<Vec<_>>()
        .join("|");
    format!(
        "{}{}{}",
        if leading_pipe { "|" } else { "" },
        body,
        if trailing_pipe { "|" } else { "" }
    )
}
