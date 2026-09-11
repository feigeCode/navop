//! ClickHouse `JSONCompact` value → `db-value` typed cell codec。
//!
//! 目标:把 `fetch_json_compact` 取回的 `serde_json::Value` 依据列声明类型映射成
//! [`db_value::CellState`]/[`db_value::DbValue`],再在 `connection.rs` 组装成
//! [`db_value::ResultBatch`],经 [`crate::executor::QueryResult::from_typed_batch`]
//! 携带 `typed_batch`。legacy 的 `rows`/`binary_cells` 仍是其兼容投影。
//!
//! ## 无损限制(重要)
//!
//! ClickHouse 的 `String`/`FixedString` 是任意字节容器,但 `JSONCompact` 只能把它们
//! 序列化成 JSON 字符串。`serde_json` 解析时,非 UTF-8 / 任意字节已经被替换或损坏,
//! **原始字节无法从 JSON 值恢复**。因此这里对 `String`/`FixedString` 一律映射为
//! [`DbValue::LegacyText`]——标记“这是来自 legacy 字符串链路、未经证实的文本”,
//! **不默认按 UTF-8 强转成权威的 [`DbValue::Text`]**,也不伪造 `Binary` 字节(那会是
//! 损坏数据的伪装)。需要无损字节语义时必须改用 `RowBinaryWithNamesAndTypes` 等二进制
//! 读取路径,本模块不承诺 JSONCompact 路径的无损性。
//!
//! `FixedString` 去尾 NUL 行为按现状保留(见 [`trim_fixed_string_padding`]),它本身是有损
//! 的,仅因任务约定不被改动。
//!
//! 未知/复杂类型([`Array`]、[`Tuple`]、[`Map`]、`AggregateFunction` 等)不猜测解码,统一
//! 落到 [`DbValue::Unsupported`],既不伪装成 `Text`/NULL,也不会让整条查询因无法投影
//! Undecoded 单元而失败。

use db_value::{CellState, DbValue, FloatWidth};
use serde_json::Value;

/// 解析列声明类型,逐层剥离 `Nullable(...)` / `LowCardinality(...)` 包装,返回内层类型。
fn strip_wrappers(data_type: &str) -> &str {
    let mut current = data_type.trim();
    loop {
        let lower = current.to_ascii_lowercase();
        let inner = if lower.starts_with("nullable(") {
            &current["nullable(".len()..]
        } else if lower.starts_with("lowcardinality(") {
            &current["lowcardinality(".len()..]
        } else {
            return current;
        };
        let Some(inner) = inner.strip_suffix(')') else {
            return current;
        };
        current = inner.trim();
    }
}

/// 取类型的基本名(去掉参数部分),例如 `"DateTime64(3, 'UTC')"` → `"DATETIME64"`。
fn base_type(data_type: &str) -> String {
    let stripped = strip_wrappers(data_type);
    let end = stripped.find('(').unwrap_or(stripped.len());
    stripped[..end].trim().to_ascii_uppercase()
}

/// 判断列类型是否被 `Nullable(...)` 包裹(含 `LowCardinality(Nullable(...))`)。
pub(crate) fn is_nullable(data_type: &str) -> bool {
    let mut current = data_type.trim();
    loop {
        let lower = current.to_ascii_lowercase();
        if lower.starts_with("nullable(") {
            return true;
        }
        let Some(inner) = lower
            .starts_with("lowcardinality(")
            .then(|| &current["lowcardinality(".len()..])
        else {
            return false;
        };
        let Some(inner) = inner.strip_suffix(')') else {
            return false;
        };
        current = inner.trim();
    }
}

/// `FixedString` 现有显示行为:去掉尾部 NUL 填充。
fn trim_fixed_string_padding(value: &str) -> &str {
    value.trim_end_matches('\0')
}

/// 把 JSON 值规范化为字符串承载的原始文本(不补引号、不 JSON 序列化)。
fn json_value_text(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        _ => None,
    }
}

/// 把 JSON 值规范化为整数值的精确十进制字符串(不经过浮点)。
fn json_integer_text(value: &Value) -> Option<String> {
    match value {
        Value::Number(number) => Some(number.to_string()),
        Value::String(text) => Some(text.clone()),
        _ => None,
    }
}

/// 解码单个单元格为带类型的 [`CellState`]。
///
/// `data_type` 是 JSONCompact 元数据里声明的完整列类型(如 `"Nullable(String)"`)。
/// JSON 的 `null` 一律映射为 SQL NULL;其余按列类型精确映射,未知/复杂类型落
/// [`DbValue::Unsupported`]。
pub(crate) fn decode_cell(data_type: &str, value: &Value) -> CellState {
    // SQL NULL / `Nullable(T)` 的空值。
    if value.is_null() {
        return CellState::Decoded(DbValue::Null);
    }

    let base = base_type(data_type);
    let decoded = match base.as_str() {
        // 布尔。
        "BOOL" => match value {
            Value::Bool(flag) => DbValue::Bool(*flag),
            Value::Number(number) => match number.as_u64() {
                Some(0) => DbValue::Bool(false),
                Some(1) => DbValue::Bool(true),
                _ => unsupported(data_type, value),
            },
            Value::String(text) => match text.as_str() {
                "true" | "1" => DbValue::Bool(true),
                "false" | "0" => DbValue::Bool(false),
                _ => unsupported(data_type, value),
            },
            _ => unsupported(data_type, value),
        },
        // 整数。
        "INT8" | "INT16" | "INT32" | "INT64" | "INT128" | "INT256" => {
            match json_integer_text(value) {
                Some(text) => DbValue::Integer(text),
                None => unsupported(data_type, value),
            }
        }
        "UINT8" | "UINT16" | "UINT32" | "UINT64" | "UINT128" | "UINT256" => {
            match json_integer_text(value) {
                Some(text) => DbValue::Unsigned(text),
                None => unsupported(data_type, value),
            }
        }
        // 浮点。
        "FLOAT32" => match json_value_text(value) {
            Some(text) => DbValue::Float {
                value: text,
                width: FloatWidth::F32,
            },
            None => unsupported(data_type, value),
        },
        "FLOAT64" => match json_value_text(value) {
            Some(text) => DbValue::Float {
                value: text,
                width: FloatWidth::F64,
            },
            None => unsupported(data_type, value),
        },
        // 精确十进制,不经浮点。
        "DECIMAL" | "DECIMAL32" | "DECIMAL64" | "DECIMAL128" | "DECIMAL256" => {
            match json_value_text(value) {
                Some(text) => DbValue::Decimal(text),
                None => unsupported(data_type, value),
            }
        }
        // 时间。
        "DATE" | "DATE32" => match json_value_text(value) {
            Some(text) => DbValue::Date(text),
            None => unsupported(data_type, value),
        },
        "DATETIME" | "DATETIME64" => match json_value_text(value) {
            Some(text) => DbValue::DateTime(text),
            None => unsupported(data_type, value),
        },
        "TIME" | "TIME64" => match json_value_text(value) {
            Some(text) => DbValue::Time(text),
            None => unsupported(data_type, value),
        },
        // UUID / 定义明确的文本类型。
        "UUID" => match value.as_str() {
            Some(text) => DbValue::Uuid(text.to_string()),
            None => unsupported(data_type, value),
        },
        // 枚举值是已定义的字符串字面量,可安全视为 Text。
        "ENUM8" | "ENUM16" => match value.as_str() {
            Some(text) => DbValue::Text(text.to_string()),
            None => unsupported(data_type, value),
        },
        "IPV4" | "IPV6" => match value.as_str() {
            Some(text) => DbValue::Text(text.to_string()),
            None => unsupported(data_type, value),
        },
        // 任意字节容器:JSONCompact 已损坏非 UTF-8 字节,无法无损恢复,保守用
        // LegacyText 标记来源,不伪装成权威 Text。
        "STRING" => legacy_string(data_type, value),
        "FIXEDSTRING" => match value.as_str() {
            Some(text) => DbValue::LegacyText(trim_fixed_string_padding(text).to_string()),
            None => unsupported(data_type, value),
        },
        // JSON 类型保留结构化值。
        "JSON" => json_value(value),
        // 未知/复杂类型:不伪造值。
        _ => unsupported(data_type, value),
    };
    CellState::Decoded(decoded)
}

/// `String` 的映射:JSONCompact 无法确认原始字节是否为合法 UTF-8,保守标记为
/// [`db_value::DbValue::LegacyText`](见模块级无损限制说明)。
fn legacy_string(data_type: &str, value: &Value) -> DbValue {
    match value.as_str() {
        Some(text) => DbValue::LegacyText(text.to_string()),
        None => unsupported(data_type, value),
    }
}

fn unsupported(data_type: &str, value: &Value) -> DbValue {
    DbValue::Unsupported {
        native_type: data_type.to_string(),
        display: legacy_json_display(value),
    }
}

/// `Json` 值:值为对象/数组时保留结构化 JSON,否则保守降级。
fn json_value(value: &Value) -> DbValue {
    match value {
        Value::Object(_) | Value::Array(_) => DbValue::Json(value.clone()),
        _ => DbValue::Unsupported {
            native_type: "JSON".to_string(),
            display: legacy_json_display(value),
        },
    }
}

/// legacy `rows` 使用的 JSON 展示文本,保持与旧 `json_value_to_string` 一致:
/// 字符串不带引号,对象/数组用紧凑 JSON。SQL NULL 在 `decode_cell` 中已单独处理,
/// 不会走到这里。
fn legacy_json_display(value: &Value) -> String {
    match value {
        Value::Null => "NULL".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        Value::Array(_) | Value::Object(_) => value.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db_value::DbValue;

    fn decode(data_type: &str, value: Value) -> CellState {
        decode_cell(data_type, &value)
    }

    #[test]
    fn null_is_sql_null() {
        assert_eq!(
            decode("Nullable(String)", Value::Null),
            CellState::Decoded(DbValue::Null)
        );
        assert_eq!(
            decode("Nullable(Int64)", Value::Null),
            CellState::Decoded(DbValue::Null)
        );
    }

    #[test]
    fn bool_maps_from_json_boolean_and_numeric() {
        assert_eq!(
            decode("Bool", Value::Bool(true)),
            CellState::Decoded(DbValue::Bool(true))
        );
        assert_eq!(
            decode("Bool", Value::from(1)),
            CellState::Decoded(DbValue::Bool(true))
        );
        assert_eq!(
            decode("Bool", Value::from(0)),
            CellState::Decoded(DbValue::Bool(false))
        );
    }

    #[test]
    fn signed_and_unsigned_integers_keep_exact_digits() {
        assert_eq!(
            decode("Int32", Value::from(-300)),
            CellState::Decoded(DbValue::Integer("-300".into()))
        );
        // ClickHouse 对 64 位及以上整数按字符串输出,保留精确位数。
        assert_eq!(
            decode("Int64", Value::String("9000000000000000000".into())),
            CellState::Decoded(DbValue::Integer("9000000000000000000".into()))
        );
        assert_eq!(
            decode("UInt32", Value::from(42u32)),
            CellState::Decoded(DbValue::Unsigned("42".into()))
        );
        assert_eq!(
            decode("UInt64", Value::String("18446744073709551615".into())),
            CellState::Decoded(DbValue::Unsigned("18446744073709551615".into()))
        );
    }

    #[test]
    fn floats_keep_width_by_type() {
        assert_eq!(
            decode("Float32", Value::from(1.5)),
            CellState::Decoded(DbValue::Float {
                value: "1.5".into(),
                width: FloatWidth::F32,
            })
        );
        assert_eq!(
            decode("Float64", Value::String("nan".into())),
            CellState::Decoded(DbValue::Float {
                value: "nan".into(),
                width: FloatWidth::F64,
            })
        );
    }

    #[test]
    fn decimal_stays_exact_string() {
        assert_eq!(
            decode("Decimal(18, 4)", Value::String("12345.6789".into())),
            CellState::Decoded(DbValue::Decimal("12345.6789".into()))
        );
    }

    #[test]
    fn temporal_types_map_by_kind() {
        assert_eq!(
            decode("Date", Value::String("2026-09-11".into())),
            CellState::Decoded(DbValue::Date("2026-09-11".into()))
        );
        assert_eq!(
            decode(
                "DateTime64(3, 'UTC')",
                Value::String("2026-09-11 12:33:00.123".into())
            ),
            CellState::Decoded(DbValue::DateTime("2026-09-11 12:33:00.123".into()))
        );
        assert_eq!(
            decode("Time", Value::String("12:33:00".into())),
            CellState::Decoded(DbValue::Time("12:33:00".into()))
        );
    }

    #[test]
    fn uuid_enum_and_ip_are_text() {
        assert_eq!(
            decode(
                "UUID",
                Value::String("67e55044-10b1-426f-9247-bb680e5fe0c8".into())
            ),
            CellState::Decoded(DbValue::Uuid("67e55044-10b1-426f-9247-bb680e5fe0c8".into()))
        );
        assert_eq!(
            decode("Enum8('a' = 1)", Value::String("a".into())),
            CellState::Decoded(DbValue::Text("a".into()))
        );
        assert_eq!(
            decode("IPv4", Value::String("127.0.0.1".into())),
            CellState::Decoded(DbValue::Text("127.0.0.1".into()))
        );
    }

    #[test]
    fn string_is_legacy_text_not_authoritative_text() {
        // JSONCompact 已损坏可能的非 UTF-8 字节,不能默认强转成 Text。
        assert_eq!(
            decode("String", Value::String("héllo".into())),
            CellState::Decoded(DbValue::LegacyText("héllo".into()))
        );
        assert_eq!(
            decode("LowCardinality(String)", Value::String("x".into())),
            CellState::Decoded(DbValue::LegacyText("x".into()))
        );
    }

    #[test]
    fn fixed_string_keeps_existing_trailing_nul_trimming() {
        assert_eq!(
            decode(
                "FixedString(8)",
                Value::String("abc\u{0}\u{0}\u{0}\u{0}\u{0}".into())
            ),
            CellState::Decoded(DbValue::LegacyText("abc".into()))
        );
    }

    #[test]
    fn json_keeps_structured_value_and_is_never_fabricated() {
        assert_eq!(
            decode("JSON", serde_json::json!({"a": 1})),
            CellState::Decoded(DbValue::Json(serde_json::json!({"a": 1})))
        );
    }

    #[test]
    fn complex_and_unknown_types_fall_to_unsupported() {
        let array = CellState::Decoded(DbValue::Unsupported {
            native_type: "Array(UInt8)".into(),
            display: "[1,2,3]".into(),
        });
        assert_eq!(decode("Array(UInt8)", serde_json::json!([1, 2, 3])), array);

        let tuple = CellState::Decoded(DbValue::Unsupported {
            native_type: "Tuple(String, UInt8)".into(),
            display: "[\"x\",1]".into(),
        });
        assert_eq!(
            decode("Tuple(String, UInt8)", serde_json::json!(["x", 1])),
            tuple
        );
    }
}
