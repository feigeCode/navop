//! DuckDB `ValueRef` ↔ `extension-protocol::CellValue` 转换。
//!
//! DuckDB 的类型枚举非常细(20+ variant),宿主侧只需粗粒度 `ColumnTypeKind`
//! 即可。这里把窄类型(`TinyInt`/`SmallInt`/`Int`)归一化到 `i64`,把无符号
//! 类型归一化到 `u64`,decimal 用字符串保留精度,blob 按协议使用 base64。

use anyhow::{Result, anyhow};
use base64::Engine;
use duckdb::types::{Value, ValueRef};
use extension_protocol::row::{CellValue, ColumnTypeKind};

fn bytes_to_base64(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// 把 DuckDB 单个 `ValueRef` 转成 `CellValue`。
pub fn value_ref_to_cell(value: ValueRef<'_>) -> CellValue {
    match value {
        ValueRef::Null => CellValue::Null,
        ValueRef::Boolean(v) => CellValue::Bool { value: v },
        ValueRef::TinyInt(v) => CellValue::I64 { value: v as i64 },
        ValueRef::SmallInt(v) => CellValue::I64 { value: v as i64 },
        ValueRef::Int(v) => CellValue::I64 { value: v as i64 },
        ValueRef::BigInt(v) => CellValue::I64 { value: v },
        ValueRef::HugeInt(v) => CellValue::Decimal {
            value: v.to_string(),
        },
        ValueRef::UTinyInt(v) => CellValue::U64 { value: v as u64 },
        ValueRef::USmallInt(v) => CellValue::U64 { value: v as u64 },
        ValueRef::UInt(v) => CellValue::U64 { value: v as u64 },
        ValueRef::UBigInt(v) => CellValue::U64 { value: v },
        ValueRef::Float(v) => CellValue::F64 { value: v as f64 },
        ValueRef::Double(v) => CellValue::F64 { value: v },
        ValueRef::Decimal(v) => CellValue::Decimal {
            value: v.to_string(),
        },
        ValueRef::Text(bytes) => match String::from_utf8(bytes.to_vec()) {
            Ok(s) => CellValue::Text { value: s },
            Err(_) => CellValue::Bytes {
                value: bytes_to_base64(bytes),
            },
        },
        ValueRef::Blob(bytes) => CellValue::Bytes {
            value: bytes_to_base64(bytes),
        },
        other => CellValue::Text {
            value: format!("{other:?}"),
        },
    }
}

/// 把 wire 参数值转成 DuckDB owning value,供 `params_from_iter` 绑定。
pub fn cell_value_to_duckdb_value(cell: &CellValue) -> Result<Value> {
    Ok(match cell {
        CellValue::Null => Value::Null,
        CellValue::Bool { value } => Value::Boolean(*value),
        CellValue::I64 { value } => Value::BigInt(*value),
        CellValue::U64 { value } => Value::UBigInt(*value),
        CellValue::F64 { value } => Value::Double(*value),
        CellValue::Decimal { value }
        | CellValue::Text { value }
        | CellValue::Uuid { value }
        | CellValue::Date { value }
        | CellValue::Time { value }
        | CellValue::Datetime { value }
        | CellValue::Duration { value } => Value::Text(value.clone()),
        CellValue::Bytes { value } => Value::Blob(
            base64::engine::general_purpose::STANDARD
                .decode(value.as_bytes())
                .map_err(|error| anyhow!("invalid base64 bytes parameter: {error}"))?,
        ),
        CellValue::Json { value } => Value::Text(value.to_string()),
        CellValue::Array { value, .. } => Value::Text(serde_json::to_string(value)?),
        CellValue::Map { value } => {
            Value::Text(serde_json::Value::Object(value.clone()).to_string())
        }
        CellValue::Geo { value, .. } => Value::Text(value.clone()),
        CellValue::Custom { raw, .. } => Value::Text(raw.clone()),
    })
}

/// 把 DuckDB column type 字符串映射到 `ColumnTypeKind`。
///
/// DuckDB 的 type debug 输出形如 `Int`/`Varchar`/`Decimal(10, 2)`;
/// 把括号外的 base name 大写后 match。
pub fn map_column_type_kind(db_type: &str) -> ColumnTypeKind {
    let upper = db_type.to_ascii_uppercase();
    let base = upper.split('(').next().unwrap_or(&upper).trim();
    match base {
        "BOOL" | "BOOLEAN" | "BIT" => ColumnTypeKind::Bool,
        "TINYINT" | "SMALLINT" | "INT" | "INTEGER" | "BIGINT" => ColumnTypeKind::I64,
        "UTINYINT" | "USMALLINT" | "UINT" | "UINTEGER" | "UBIGINT" => ColumnTypeKind::U64,
        "FLOAT" | "DOUBLE" | "REAL" => ColumnTypeKind::F64,
        "DECIMAL" | "NUMERIC" | "MONEY" | "HUGEINT" => ColumnTypeKind::Decimal,
        "VARCHAR" | "CHAR" | "TEXT" | "NVARCHAR" | "CHARACTER" | "STRING" => ColumnTypeKind::Text,
        "BLOB" | "BYTEA" | "VARBINARY" | "BINARY" => ColumnTypeKind::Bytes,
        "DATE" => ColumnTypeKind::Date,
        "TIME" => ColumnTypeKind::Time,
        "TIMESTAMP" | "TIMESTAMPTZ" | "DATETIME" => ColumnTypeKind::Datetime,
        "INTERVAL" => ColumnTypeKind::Duration,
        "UUID" => ColumnTypeKind::Uuid,
        "JSON" | "JSONB" => ColumnTypeKind::Json,
        "LIST" | "ARRAY" => ColumnTypeKind::Array,
        "MAP" | "STRUCT" => ColumnTypeKind::Map,
        _ => ColumnTypeKind::Text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn null_maps_to_null_cell() {
        assert!(matches!(value_ref_to_cell(ValueRef::Null), CellValue::Null));
    }

    #[test]
    fn bool_maps_to_bool_cell() {
        assert_eq!(
            value_ref_to_cell(ValueRef::Boolean(true)),
            CellValue::Bool { value: true }
        );
    }

    #[test]
    fn signed_int_widths_normalize_to_i64() {
        assert_eq!(
            value_ref_to_cell(ValueRef::TinyInt(-5)),
            CellValue::I64 { value: -5 }
        );
        assert_eq!(
            value_ref_to_cell(ValueRef::SmallInt(-1000)),
            CellValue::I64 { value: -1000 }
        );
        assert_eq!(
            value_ref_to_cell(ValueRef::Int(123_456)),
            CellValue::I64 { value: 123_456 }
        );
        assert_eq!(
            value_ref_to_cell(ValueRef::BigInt(i64::MIN)),
            CellValue::I64 { value: i64::MIN }
        );
    }

    #[test]
    fn unsigned_int_widths_normalize_to_u64() {
        assert_eq!(
            value_ref_to_cell(ValueRef::UTinyInt(255)),
            CellValue::U64 { value: 255 }
        );
        assert_eq!(
            value_ref_to_cell(ValueRef::USmallInt(65_535)),
            CellValue::U64 { value: 65_535 }
        );
        assert_eq!(
            value_ref_to_cell(ValueRef::UBigInt(u64::MAX)),
            CellValue::U64 { value: u64::MAX }
        );
    }

    #[test]
    fn float_double_become_f64_cell() {
        let f = value_ref_to_cell(ValueRef::Float(1.5_f32));
        if let CellValue::F64 { value } = f {
            assert!((value - 1.5).abs() < 1e-6);
        } else {
            panic!("expected F64");
        }
        let d = value_ref_to_cell(ValueRef::Double(2.5));
        assert_eq!(d, CellValue::F64 { value: 2.5 });
    }

    #[test]
    fn huge_int_serializes_as_decimal_string() {
        let v = value_ref_to_cell(ValueRef::HugeInt(123_456_789));
        assert_eq!(
            v,
            CellValue::Decimal {
                value: "123456789".to_string()
            }
        );
    }

    #[test]
    fn text_value_becomes_text_cell() {
        let v = value_ref_to_cell(ValueRef::Text(b"hello"));
        assert_eq!(
            v,
            CellValue::Text {
                value: "hello".to_string()
            }
        );
    }

    #[test]
    fn non_utf8_text_falls_back_to_bytes_base64() {
        let bytes: &[u8] = &[0xff, 0xfe, 0xfd];
        let v = value_ref_to_cell(ValueRef::Text(bytes));
        match v {
            CellValue::Bytes { value } => assert_eq!(value, "//79"),
            other => panic!("expected Bytes, got {other:?}"),
        }
    }

    #[test]
    fn blob_uses_base64_encoding() {
        let bytes: &[u8] = &[0x01, 0x02, 0x03];
        let v = value_ref_to_cell(ValueRef::Blob(bytes));
        assert_eq!(
            v,
            CellValue::Bytes {
                value: "AQID".to_string()
            }
        );
    }

    #[test]
    fn map_column_type_kind_known_types() {
        assert_eq!(map_column_type_kind("Int"), ColumnTypeKind::I64);
        assert_eq!(map_column_type_kind("BIGINT"), ColumnTypeKind::I64);
        assert_eq!(map_column_type_kind("UBigInt"), ColumnTypeKind::U64);
        assert_eq!(map_column_type_kind("Varchar"), ColumnTypeKind::Text);
        assert_eq!(map_column_type_kind("Boolean"), ColumnTypeKind::Bool);
        assert_eq!(map_column_type_kind("Double"), ColumnTypeKind::F64);
        assert_eq!(
            map_column_type_kind("Decimal(10, 2)"),
            ColumnTypeKind::Decimal
        );
        assert_eq!(map_column_type_kind("Timestamp"), ColumnTypeKind::Datetime);
        assert_eq!(map_column_type_kind("Date"), ColumnTypeKind::Date);
        assert_eq!(map_column_type_kind("Time"), ColumnTypeKind::Time);
        assert_eq!(map_column_type_kind("Blob"), ColumnTypeKind::Bytes);
        assert_eq!(map_column_type_kind("Json"), ColumnTypeKind::Json);
        assert_eq!(map_column_type_kind("Uuid"), ColumnTypeKind::Uuid);
        assert_eq!(map_column_type_kind("List"), ColumnTypeKind::Array);
    }

    #[test]
    fn map_column_type_kind_unknown_falls_back_to_text() {
        assert_eq!(
            map_column_type_kind("WeirdCustomType"),
            ColumnTypeKind::Text
        );
        assert_eq!(map_column_type_kind(""), ColumnTypeKind::Text);
    }

    #[test]
    fn cell_value_to_duckdb_value_maps_basic_values() {
        assert_eq!(
            cell_value_to_duckdb_value(&CellValue::Null).unwrap(),
            duckdb::types::Value::Null
        );
        assert_eq!(
            cell_value_to_duckdb_value(&CellValue::Bool { value: true }).unwrap(),
            duckdb::types::Value::Boolean(true)
        );
        assert_eq!(
            cell_value_to_duckdb_value(&CellValue::I64 { value: 42 }).unwrap(),
            duckdb::types::Value::BigInt(42)
        );
        assert_eq!(
            cell_value_to_duckdb_value(&CellValue::Text {
                value: "alice".into()
            })
            .unwrap(),
            duckdb::types::Value::Text("alice".into())
        );
    }

    #[test]
    fn cell_value_to_duckdb_value_decodes_base64_bytes() {
        assert_eq!(
            cell_value_to_duckdb_value(&CellValue::Bytes {
                value: "AQID".into()
            })
            .unwrap(),
            duckdb::types::Value::Blob(vec![1, 2, 3])
        );
    }
}
