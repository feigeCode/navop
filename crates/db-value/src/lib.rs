//! Database values independent of database SDKs, UI runtimes, and transports.

use serde::{Deserialize, Serialize};
use std::fmt;

const DEFAULT_BINARY_PREVIEW_BYTES: usize = 1024;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum DbValue {
    Null,
    Bool(bool),
    Integer(String),
    Unsigned(String),
    Float {
        value: String,
        width: FloatWidth,
    },
    Decimal(String),
    Text(String),
    /// Text inherited from the legacy string-only result contract.
    LegacyText(String),
    Binary(Vec<u8>),
    Date(String),
    Time(String),
    DateTime(String),
    Json(serde_json::Value),
    Uuid(String),
    Duration(String),
    Unsupported {
        native_type: String,
        display: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FloatWidth {
    F32,
    F64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RawRepresentation {
    DatabaseValueBytes,
    IpcPayload,
    ExternalResource { id: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RawPayload {
    pub bytes: Vec<u8>,
    pub representation: RawRepresentation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum CellState {
    Decoded(DbValue),
    Undecoded {
        native_type: String,
        raw: Option<RawPayload>,
        reason: String,
    },
    DecodeError {
        native_type: String,
        raw: Option<RawPayload>,
        diagnostic: String,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Nullability {
    Unknown,
    Yes,
    No,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnDescriptor {
    pub id: String,
    pub label: String,
    pub native_type: String,
    pub logical_type: String,
    pub nullable: Nullability,
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub precision: Option<u32>,
    pub scale: Option<i32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultRow {
    pub id: u64,
    pub cells: Vec<CellState>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ResultBatch {
    pub generation: u64,
    pub columns: Vec<ColumnDescriptor>,
    pub rows: Vec<ResultRow>,
    pub complete: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ValueModelError {
    #[error("result row {row_id} has {actual} cells, expected {expected}")]
    RowWidth {
        row_id: u64,
        actual: usize,
        expected: usize,
    },
    #[error("result row id {0} is duplicated")]
    DuplicateRowId(u64),
    #[error("binary preview is not the original value")]
    PreviewIsNotValue,
}

impl ResultBatch {
    pub fn try_new(
        generation: u64,
        columns: Vec<ColumnDescriptor>,
        rows: Vec<ResultRow>,
        complete: bool,
    ) -> Result<Self, ValueModelError> {
        let mut row_ids = std::collections::HashSet::with_capacity(rows.len());
        for row in &rows {
            if row.cells.len() != columns.len() {
                return Err(ValueModelError::RowWidth {
                    row_id: row.id,
                    actual: row.cells.len(),
                    expected: columns.len(),
                });
            }
            if !row_ids.insert(row.id) {
                return Err(ValueModelError::DuplicateRowId(row.id));
            }
        }
        Ok(Self {
            generation,
            columns,
            rows,
            complete,
        })
    }

    pub fn display_value(value: &DbValue) -> String {
        match value {
            DbValue::Null => "NULL".to_string(),
            DbValue::Bool(value) => value.to_string(),
            DbValue::Integer(value)
            | DbValue::Unsigned(value)
            | DbValue::Decimal(value)
            | DbValue::Text(value)
            | DbValue::LegacyText(value)
            | DbValue::Date(value)
            | DbValue::Time(value)
            | DbValue::DateTime(value)
            | DbValue::Uuid(value)
            | DbValue::Duration(value) => value.clone(),
            DbValue::Float { value, .. } => value.clone(),
            DbValue::Binary(bytes) => format_binary_preview(bytes),
            DbValue::Json(value) => value.to_string(),
            DbValue::Unsupported { display, .. } => display.clone(),
        }
    }
}

pub fn format_binary_preview(bytes: &[u8]) -> String {
    let preview = &bytes[..bytes.len().min(DEFAULT_BINARY_PREVIEW_BYTES)];
    let hex = preview
        .iter()
        .map(|byte| format!("{byte:02X}"))
        .collect::<String>();
    if bytes.len() > preview.len() {
        format!("0x{hex}... ({} bytes)", bytes.len())
    } else {
        format!("0x{hex}")
    }
}

impl fmt::Display for DbValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&ResultBatch::display_value(self))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(id: &str) -> ColumnDescriptor {
        ColumnDescriptor {
            id: id.to_string(),
            label: id.to_string(),
            native_type: "TEXT".to_string(),
            logical_type: "text".to_string(),
            nullable: Nullability::Unknown,
            charset: None,
            collation: None,
            precision: None,
            scale: None,
        }
    }

    #[test]
    fn binary_and_text_with_same_bytes_are_distinct() {
        assert_ne!(
            DbValue::Text("hello".into()),
            DbValue::Binary(b"hello".to_vec())
        );
    }

    #[test]
    fn binary_preview_is_bounded_but_value_is_not() {
        let bytes = vec![0xAB; 2048];
        assert!(format_binary_preview(&bytes).contains("2048 bytes"));
        assert_eq!(DbValue::Binary(bytes.clone()), DbValue::Binary(bytes));
    }

    #[test]
    fn result_batch_rejects_wrong_row_width_and_duplicate_ids() {
        let error = ResultBatch::try_new(
            1,
            vec![column("id")],
            vec![ResultRow {
                id: 1,
                cells: vec![],
            }],
            true,
        )
        .unwrap_err();
        assert!(matches!(error, ValueModelError::RowWidth { .. }));

        let error = ResultBatch::try_new(
            1,
            vec![column("id")],
            vec![
                ResultRow {
                    id: 1,
                    cells: vec![CellState::Decoded(DbValue::Integer("1".into()))],
                },
                ResultRow {
                    id: 1,
                    cells: vec![CellState::Decoded(DbValue::Integer("2".into()))],
                },
            ],
            true,
        )
        .unwrap_err();
        assert_eq!(error, ValueModelError::DuplicateRowId(1));
    }

    #[test]
    fn null_empty_text_and_empty_binary_are_distinct() {
        assert_ne!(DbValue::Null, DbValue::Text(String::new()));
        assert_ne!(DbValue::Text(String::new()), DbValue::Binary(Vec::new()));
    }
}
