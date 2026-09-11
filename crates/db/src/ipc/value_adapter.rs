//! `extension_protocol::row::CellValue` → `db-value` typed model adapter。
//!
//! 目标:IPC 在结果获取阶段先进入带类型的 [`db_value::DbValue`]/[`db_value::CellState`],
//! 不再像旧的 `cell_to_display_value` 那样把所有值在一次转换里塌缩成显示字符串。
//! 同时保留旧 `QueryResult.rows` + `binary_cells` 兼容投影,供尚未迁移的消费者使用。
//!
//! wire 形状(`CellValue`/`ColumnSpec`)保持不变;这里只做宿主侧方向转换。

use crate::connection::DbError;
#[cfg(test)]
use crate::executor::BinaryCell;
use base64::Engine;
use db_value::{
    CellState, ColumnDescriptor, DbValue, FloatWidth, Nullability, ResultBatch, ResultRow,
};
use extension_protocol::row::{CellValue, ColumnSpec};
use serde_json::Value;

/// 单个 wire cell → 带类型的 [`CellState`]。
///
/// - `Bytes` 的 base64 非法时直接失败(不降级为 NULL 或空值)。
/// - `Datetime`/`Decimal` 保留 wire 原字符串,不做浮点或时区归一。
/// - 模型尚未表达 `Array`/`Map`/`Geo`/`Custom`,显式落到
///   [`DbValue::Unsupported`] 而不是伪装成 `Text`。
pub(super) fn cell_to_cell_state(cell: CellValue) -> Result<CellState, DbError> {
    let value = match cell {
        CellValue::Null => DbValue::Null,
        CellValue::Bool { value } => DbValue::Bool(value),
        CellValue::I64 { value } => DbValue::Integer(value.to_string()),
        CellValue::U64 { value } => DbValue::Unsigned(value.to_string()),
        CellValue::F64 { value } => DbValue::Float {
            value: value.to_string(),
            width: FloatWidth::F64,
        },
        CellValue::Decimal { value } => DbValue::Decimal(value),
        CellValue::Text { value } => DbValue::Text(value),
        CellValue::Bytes { value } => DbValue::Binary(decode_base64_bytes(&value)?),
        CellValue::Json { value } => DbValue::Json(value),
        CellValue::Uuid { value } => DbValue::Uuid(value),
        CellValue::Date { value } => DbValue::Date(value),
        CellValue::Time { value } => DbValue::Time(value),
        CellValue::Datetime { value } => DbValue::DateTime(value),
        CellValue::Duration { value } => DbValue::Duration(value),
        CellValue::Array {
            element_type,
            value,
        } => DbValue::Unsupported {
            native_type: format!("ARRAY<{element_type:?}>"),
            display: serde_json::to_string(&value).unwrap_or_else(|_| "ARRAY".to_string()),
        },
        CellValue::Map { value } => DbValue::Unsupported {
            native_type: "MAP".to_string(),
            display: Value::Object(value).to_string(),
        },
        CellValue::Geo { subtype, value } => DbValue::Unsupported {
            native_type: format!("GEO:{subtype}"),
            display: value,
        },
        CellValue::Custom { subtype, raw } => DbValue::Unsupported {
            native_type: subtype.clone(),
            display: format!("custom:{subtype}({raw})"),
        },
    };
    Ok(CellState::Decoded(value))
}

fn decode_base64_bytes(encoded: &str) -> Result<Vec<u8>, DbError> {
    base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|error| DbError::query_with_source("invalid base64 in CellValue::Bytes", error))
}

/// 用 query/start 的列描述 + 已带类型的行构造受校验的 [`ResultBatch`]。
///
/// 该 batch 是 IPC 结果在宿主的唯一类型权威;`QueryResult.rows`/`binary_cells`
/// 仅作为它的兼容投影存在。
pub(super) fn result_batch_from_columns(
    generation: u64,
    specs: &[ColumnSpec],
    rows: Vec<ResultRow>,
) -> Result<ResultBatch, DbError> {
    let columns = specs
        .iter()
        .enumerate()
        .map(|(index, spec)| ColumnDescriptor {
            id: format!("column:{index}"),
            label: spec.name.clone(),
            native_type: spec.type_str.clone(),
            logical_type: format!("{:?}", spec.type_kind),
            nullable: match spec.nullable {
                Some(true) => Nullability::Yes,
                Some(false) => Nullability::No,
                None => Nullability::Unknown,
            },
            charset: extra_str(spec, "result_charset"),
            collation: extra_str(spec, "result_collation"),
            precision: spec.precision,
            scale: spec.scale,
        })
        .collect::<Vec<_>>();

    ResultBatch::try_new(generation, columns, rows, true)
        .map_err(|error| DbError::query_with_source("invalid typed IPC result batch", error))
}

fn extra_str(spec: &ColumnSpec, key: &str) -> Option<String> {
    spec.extra
        .get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// 把带类型的 batch 投影成旧 `QueryResult` 的 `rows` + `binary_cells`(测试助手)。
///
/// 投影实现统一收敛在 [`crate::executor::project_batch_to_legacy`],避免 IPC 与
/// executor 两处漂移。`Undecoded`/`DecodeError` 无法无损投影时显式失败。
#[cfg(test)]
pub(super) fn project_legacy(
    batch: &ResultBatch,
) -> Result<(Vec<Vec<Option<String>>>, Vec<BinaryCell>), DbError> {
    crate::executor::project_batch_to_legacy(batch).map_err(|error| match error {
        crate::executor::QueryResultValueError::UnsupportedCell {
            row_index,
            column_index,
        } => DbError::query(format!(
            "cannot project undecoded IPC cell at row {row_index}, column {column_index} \
             to the legacy result"
        )),
        other => DbError::query(format!(
            "cannot project typed IPC result to the legacy result: {other}"
        )),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::row::ColumnTypeKind;

    fn spec() -> ColumnSpec {
        ColumnSpec::new("value", "TEXT", ColumnTypeKind::Text)
    }

    fn typed(cell: CellValue) -> CellState {
        cell_to_cell_state(cell).expect("cell should convert")
    }

    fn batch_for(cell: CellValue) -> ResultBatch {
        result_batch_from_columns(
            7,
            &[spec()],
            vec![ResultRow {
                id: 0,
                cells: vec![typed(cell)],
            }],
        )
        .expect("batch should build")
    }

    #[test]
    fn text_keeps_typed_text_and_projects() {
        let state = typed(CellValue::Text {
            value: "héllo".into(),
        });
        assert_eq!(state, CellState::Decoded(DbValue::Text("héllo".into())));

        let (rows, binary_cells) = project_legacy(&batch_for(CellValue::Text {
            value: "héllo".into(),
        }))
        .unwrap();
        assert_eq!(rows, vec![vec![Some("héllo".to_string())]]);
        assert!(binary_cells.is_empty());
    }

    #[test]
    fn bytes_decode_to_binary_and_keep_exact_sidecar() {
        // base64 of [1, 2, 3] = "AQID"
        let state = typed(CellValue::Bytes {
            value: "AQID".into(),
        });
        assert_eq!(state, CellState::Decoded(DbValue::Binary(vec![1, 2, 3])));

        let (rows, binary_cells) = project_legacy(&batch_for(CellValue::Bytes {
            value: "AQID".into(),
        }))
        .unwrap();
        assert_eq!(rows, vec![vec![Some("0x010203".to_string())]]);
        assert_eq!(
            binary_cells,
            vec![BinaryCell {
                row_index: 0,
                column_index: 0,
                bytes: vec![1, 2, 3],
            }]
        );
    }

    #[test]
    fn invalid_base64_bytes_fail_instead_of_becoming_null() {
        let error = cell_to_cell_state(CellValue::Bytes {
            value: "not_base64!".into(),
        })
        .unwrap_err();
        assert!(matches!(error, DbError::Query { .. }));
        assert!(
            error
                .to_string()
                .contains("invalid base64 in CellValue::Bytes")
        );
    }

    #[test]
    fn decimal_stays_exact_decimal_not_float() {
        let literal = "1234567890.123456789012345678";
        let state = typed(CellValue::Decimal {
            value: literal.into(),
        });
        assert_eq!(state, CellState::Decoded(DbValue::Decimal(literal.into())));

        let (rows, _) = project_legacy(&batch_for(CellValue::Decimal {
            value: literal.into(),
        }))
        .unwrap();
        assert_eq!(rows, vec![vec![Some(literal.to_string())]]);
    }

    #[test]
    fn datetime_keeps_raw_authoritative_value_and_normalizes_display() {
        let raw = "2026-04-27T15:08:53.085Z";
        let state = typed(CellValue::Datetime { value: raw.into() });
        assert_eq!(state, CellState::Decoded(DbValue::DateTime(raw.into())));

        let (rows, _) =
            project_legacy(&batch_for(CellValue::Datetime { value: raw.into() })).unwrap();
        assert_eq!(
            rows,
            vec![vec![Some("2026-04-27 15:08:53.085".to_string())]]
        );
    }

    #[test]
    fn null_projects_to_none_and_stays_distinct_from_empty_text() {
        assert_eq!(typed(CellValue::Null), CellState::Decoded(DbValue::Null));

        let (rows, _) = project_legacy(&batch_for(CellValue::Null)).unwrap();
        assert_eq!(rows, vec![vec![None]]);

        let (empty_rows, _) = project_legacy(&batch_for(CellValue::Text {
            value: String::new(),
        }))
        .unwrap();
        assert_eq!(empty_rows, vec![vec![Some(String::new())]]);
    }
}
