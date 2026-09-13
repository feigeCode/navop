use std::collections::HashMap;

use anyhow::{Result, anyhow};

use crate::executor::{QueryCellRef, QueryResult, QueryResultView, project_batch_to_legacy};

/// A render-ready cell projected from the typed batch when one is present.
pub(super) enum RenderCell<'a> {
    Null,
    Text(&'a str),
    Binary(&'a [u8]),
}

enum CellSource<'a> {
    /// Projection of the in-process typed batch. Reusing `project_batch_to_legacy`
    /// keeps datetime normalization and binary tri-state semantics identical to
    /// the legacy `rows`/`binary_cells` projection.
    Typed {
        rows: Vec<Vec<Option<String>>>,
        binary: HashMap<(usize, usize), Vec<u8>>,
    },
    Legacy(QueryResultView<'a>),
}

/// Read-only access to a query result's cells that prefers the typed batch.
///
/// When [`QueryResult::typed_batch`] is present it is the authority; otherwise
/// the validated legacy `rows`/`binary_cells` view is used unchanged.
pub(super) struct ResultCells<'a> {
    column_count: usize,
    row_count: usize,
    source: CellSource<'a>,
}

impl<'a> ResultCells<'a> {
    pub(super) fn new(result: &'a QueryResult, format_name: &str) -> Result<Self> {
        if let Some(batch) = result.typed_batch() {
            if batch.columns.len() != result.columns.len() {
                return Err(anyhow!(
                    "Invalid typed result for {format_name} export: typed batch has {} columns \
                     but the result declares {}",
                    batch.columns.len(),
                    result.columns.len()
                ));
            }
            let (rows, binary_cells) = project_batch_to_legacy(batch).map_err(|error| {
                anyhow!("Invalid typed result for {format_name} export: {error}")
            })?;
            let mut binary = HashMap::with_capacity(binary_cells.len());
            for cell in binary_cells {
                binary.insert((cell.row_index, cell.column_index), cell.bytes);
            }
            return Ok(Self {
                column_count: result.columns.len(),
                row_count: rows.len(),
                source: CellSource::Typed { rows, binary },
            });
        }

        let view = result
            .typed_view()
            .map_err(|error| anyhow!("Invalid query result for {format_name} export: {error}"))?;
        Ok(Self {
            column_count: result.columns.len(),
            row_count: result.rows.len(),
            source: CellSource::Legacy(view),
        })
    }

    pub(super) fn row_count(&self) -> usize {
        self.row_count
    }

    pub(super) fn column_count(&self) -> usize {
        self.column_count
    }

    pub(super) fn cell(&self, row_index: usize, column_index: usize) -> Option<RenderCell<'_>> {
        match &self.source {
            CellSource::Typed { rows, binary } => {
                if let Some(bytes) = binary.get(&(row_index, column_index)) {
                    return Some(RenderCell::Binary(bytes.as_slice()));
                }
                let value = rows.get(row_index)?.get(column_index)?;
                Some(match value {
                    None => RenderCell::Null,
                    Some(text) => RenderCell::Text(text.as_str()),
                })
            }
            CellSource::Legacy(view) => match view.cell(row_index, column_index)? {
                QueryCellRef::Null => Some(RenderCell::Null),
                QueryCellRef::Text(text) => Some(RenderCell::Text(text)),
                QueryCellRef::Binary(bytes) => Some(RenderCell::Binary(bytes)),
            },
        }
    }
}
