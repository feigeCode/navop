//! PostgreSQL → `db-value` typed cell codec。
//!
//! 目标:把 PostgreSQL 驱动的 `extract_value`/`build_query_result` 从“一次塌缩成
//! 显示字符串”改为先进入带类型的 [`db_value::CellState`]/[`db_value::DbValue`],
//! 再由 [`crate::executor::QueryResult::from_typed_batch`] 投影 legacy。类型权威
//! 保留在 `typed_batch`,`QueryResult.rows`/`binary_cells` 只是它的兼容投影。
//!
//! 未知/数组类型不猜测解码:统一落到 [`DbValue::Unsupported`],既不伪装成 `Text`/NULL,
//! 也不会让整条查询因无法投影 Undecoded 单元而失败(与 `ipc::value_adapter` 一致)。

use std::error::Error;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use db_value::{CellState, DbValue, FloatWidth};
use tokio_postgres::Row;
use tokio_postgres::types::{FromSql, Type};

use super::connection::decode_numeric;

/// PostgreSQL wire 原始字节,不做任何解码地按列复制出来。
///
/// `FromSql::accepts` 恒为 true,因此 `row.try_get::<_, RawCell>` 对任意类型都能
/// 拿到逐字节的原始值,再由 [`decode_raw`] 依据列类型定向解码。
#[derive(Debug)]
pub(super) struct RawCell(pub(super) Option<Vec<u8>>);

impl<'a> FromSql<'a> for RawCell {
    fn from_sql(
        _ty: &Type,
        raw: &'a [u8],
    ) -> std::result::Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(Self(Some(raw.to_vec())))
    }

    fn from_sql_null(_ty: &Type) -> std::result::Result<Self, Box<dyn Error + Sync + Send>> {
        Ok(Self(None))
    }

    fn accepts(_ty: &Type) -> bool {
        true
    }
}

/// 解码实时查询行中的单个单元为带类型的 [`CellState`]。
///
/// SQL NULL 映射为 `Decoded(DbValue::Null)`;取原始字节失败才进入 `DecodeError`。
pub(super) fn decode_cell(row: &Row, index: usize) -> CellState {
    let column = &row.columns()[index];
    let native_type = column.type_().name().to_string();
    match row.try_get::<_, RawCell>(index) {
        Ok(RawCell(Some(raw))) => decode_raw(column.type_(), &raw),
        Ok(RawCell(None)) => CellState::Decoded(DbValue::Null),
        Err(error) => CellState::DecodeError {
            native_type,
            raw: None,
            diagnostic: error.to_string(),
        },
    }
}

/// 依据列类型解码 PostgreSQL wire 字节为带类型的 [`CellState`]。
///
/// 这是纯函数,便于在不连接数据库时用手工构造的 wire 字节做单元测试。未覆盖或
/// 解码失败的类型落到 `Decoded(DbValue::Unsupported)`,保证下游
/// [`crate::executor::project_batch_to_legacy`] 始终能投影。
pub(super) fn decode_raw(ty: &Type, raw: &[u8]) -> CellState {
    let value = match decode(ty, raw) {
        Ok(value) => value,
        Err(_) => DbValue::Unsupported {
            native_type: ty.name().to_string(),
            display: format!("<{}>", ty.name()),
        },
    };
    CellState::Decoded(value)
}

fn decode(ty: &Type, raw: &[u8]) -> Result<DbValue, String> {
    match ty {
        &Type::BOOL => bool::from_sql(&Type::BOOL, raw)
            .map(DbValue::Bool)
            .map_err(|error| error.to_string()),
        &Type::INT2 => i16::from_sql(&Type::INT2, raw)
            .map(|value| DbValue::Integer(value.to_string()))
            .map_err(|error| error.to_string()),
        &Type::INT4 => i32::from_sql(&Type::INT4, raw)
            .map(|value| DbValue::Integer(value.to_string()))
            .map_err(|error| error.to_string()),
        &Type::INT8 => i64::from_sql(&Type::INT8, raw)
            .map(|value| DbValue::Integer(value.to_string()))
            .map_err(|error| error.to_string()),
        &Type::CHAR => i8::from_sql(&Type::CHAR, raw)
            .map(|value| DbValue::Text(char::from(value as u8).to_string()))
            .map_err(|error| error.to_string()),
        &Type::FLOAT4 => f32::from_sql(&Type::FLOAT4, raw)
            .map(|value| DbValue::Float {
                value: value.to_string(),
                width: FloatWidth::F32,
            })
            .map_err(|error| error.to_string()),
        &Type::FLOAT8 => f64::from_sql(&Type::FLOAT8, raw)
            .map(|value| DbValue::Float {
                value: value.to_string(),
                width: FloatWidth::F64,
            })
            .map_err(|error| error.to_string()),
        &Type::NUMERIC => decode_numeric(raw).map(DbValue::Decimal),
        &Type::TEXT | &Type::VARCHAR | &Type::BPCHAR | &Type::NAME | &Type::UNKNOWN => {
            String::from_sql(&Type::TEXT, raw)
                .map(DbValue::Text)
                .map_err(|error| error.to_string())
        }
        &Type::TIMESTAMP => NaiveDateTime::from_sql(&Type::TIMESTAMP, raw)
            .map(|value| DbValue::DateTime(value.format("%Y-%m-%d %H:%M:%S%.f").to_string()))
            .map_err(|error| error.to_string()),
        &Type::TIMESTAMPTZ => DateTime::<Utc>::from_sql(&Type::TIMESTAMPTZ, raw)
            // Keep the offset in the stored string so the shared legacy projection
            // (which only normalizes RFC3339 `T`-separated values) passes it
            // through unchanged, matching the pre-refactor `%z` display.
            .map(|value| DbValue::DateTime(value.format("%Y-%m-%d %H:%M:%S%.f %z").to_string()))
            .map_err(|error| error.to_string()),
        &Type::DATE => NaiveDate::from_sql(&Type::DATE, raw)
            .map(|value| DbValue::Date(value.format("%Y-%m-%d").to_string()))
            .map_err(|error| error.to_string()),
        &Type::TIME => NaiveTime::from_sql(&Type::TIME, raw)
            .map(|value| DbValue::Time(value.format("%H:%M:%S%.f").to_string()))
            .map_err(|error| error.to_string()),
        &Type::BYTEA => Vec::<u8>::from_sql(&Type::BYTEA, raw)
            .map(DbValue::Binary)
            .map_err(|error| error.to_string()),
        &Type::JSON | &Type::JSONB => serde_json::Value::from_sql(ty, raw)
            .map(DbValue::Json)
            .map_err(|error| error.to_string()),
        &Type::UUID => uuid::Uuid::from_sql(&Type::UUID, raw)
            .map(|value| DbValue::Uuid(value.to_string()))
            .map_err(|error| error.to_string()),
        _ => Err(format!("unsupported PostgreSQL type: {}", ty.name())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn numeric_bytes(ndigits: i16, weight: i16, sign: u16, scale: u16, digits: &[u16]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&ndigits.to_be_bytes());
        bytes.extend_from_slice(&weight.to_be_bytes());
        bytes.extend_from_slice(&sign.to_be_bytes());
        bytes.extend_from_slice(&scale.to_be_bytes());
        for digit in digits {
            bytes.extend_from_slice(&digit.to_be_bytes());
        }
        bytes
    }

    #[test]
    fn empty_bytes_decode_to_empty_values_not_null() {
        // decode_raw 只接收非 NULL 字节:空文本/空二进制与 NULL 语义不同。
        assert_eq!(
            decode_raw(&Type::TEXT, &[]),
            CellState::Decoded(DbValue::Text(String::new()))
        );
        assert_eq!(
            decode_raw(&Type::BYTEA, &[]),
            CellState::Decoded(DbValue::Binary(Vec::new()))
        );
        assert_ne!(
            CellState::Decoded(DbValue::Binary(Vec::new())),
            CellState::Decoded(DbValue::Null)
        );
    }

    #[test]
    fn bool_decodes_to_boolean() {
        assert_eq!(
            decode_raw(&Type::BOOL, &[1]),
            CellState::Decoded(DbValue::Bool(true))
        );
        assert_eq!(
            decode_raw(&Type::BOOL, &[0]),
            CellState::Decoded(DbValue::Bool(false))
        );
    }

    #[test]
    fn integer_widths_keep_exact_digits() {
        assert_eq!(
            decode_raw(&Type::INT2, &(-300_i16).to_be_bytes()),
            CellState::Decoded(DbValue::Integer("-300".into()))
        );
        assert_eq!(
            decode_raw(&Type::INT4, &20_000_000_i32.to_be_bytes()),
            CellState::Decoded(DbValue::Integer("20000000".into()))
        );
        assert_eq!(
            decode_raw(&Type::INT8, &9_000_000_000_i64.to_be_bytes()),
            CellState::Decoded(DbValue::Integer("9000000000".into()))
        );
    }

    #[test]
    fn pgsql_internal_char_type_preserves_single_character_display() {
        assert_eq!(
            decode_raw(&Type::CHAR, &65_i8.to_be_bytes()),
            CellState::Decoded(DbValue::Text("A".into()))
        );
    }

    #[test]
    fn float_width_is_captured_per_type() {
        assert_eq!(
            decode_raw(&Type::FLOAT4, &1.5_f32.to_be_bytes()),
            CellState::Decoded(DbValue::Float {
                value: "1.5".into(),
                width: FloatWidth::F32,
            })
        );
        assert_eq!(
            decode_raw(&Type::FLOAT8, &2.25_f64.to_be_bytes()),
            CellState::Decoded(DbValue::Float {
                value: "2.25".into(),
                width: FloatWidth::F64,
            })
        );
    }

    #[test]
    fn numeric_stays_exact_decimal() {
        let raw = numeric_bytes(3, 1, 0x0000, 3, &[1, 2345, 6780]);
        assert_eq!(
            decode_raw(&Type::NUMERIC, &raw),
            CellState::Decoded(DbValue::Decimal("12345.678".into()))
        );
    }

    #[test]
    fn text_family_decodes_to_text() {
        for ty in [
            Type::TEXT,
            Type::VARCHAR,
            Type::BPCHAR,
            Type::NAME,
            Type::UNKNOWN,
        ] {
            assert_eq!(
                decode_raw(&ty, "héllo".as_bytes()),
                CellState::Decoded(DbValue::Text("héllo".into())),
                "type {}",
                ty.name()
            );
        }
    }

    #[test]
    fn date_time_timestamp_and_timestamptz_decode() {
        let epoch_date = NaiveDate::from_ymd_opt(2000, 1, 1).expect("epoch data is in range");

        let date = NaiveDate::from_ymd_opt(2024, 3, 5).expect("date is in range");
        let date_raw = ((date - epoch_date).num_days() as i32)
            .to_be_bytes()
            .to_vec();
        assert_eq!(
            decode_raw(&Type::DATE, &date_raw),
            CellState::Decoded(DbValue::Date("2024-03-05".into()))
        );

        let time = NaiveTime::from_hms_opt(13, 45, 59).expect("time is valid");
        let time_raw = time
            .signed_duration_since(NaiveTime::MIN)
            .num_microseconds()
            .expect("microseconds in range")
            .to_be_bytes()
            .to_vec();
        assert_eq!(
            decode_raw(&Type::TIME, &time_raw),
            CellState::Decoded(DbValue::Time("13:45:59".into()))
        );

        let naive = date.and_time(time);
        let epoch_naive = NaiveDateTime::new(epoch_date, NaiveTime::MIN);
        let ts_raw = (naive - epoch_naive)
            .num_microseconds()
            .expect("microseconds in range")
            .to_be_bytes()
            .to_vec();
        assert_eq!(
            decode_raw(&Type::TIMESTAMP, &ts_raw),
            CellState::Decoded(DbValue::DateTime("2024-03-05 13:45:59".into()))
        );

        let utc = DateTime::<Utc>::from_naive_utc_and_offset(naive, Utc);
        let tz_raw = (utc - DateTime::<Utc>::from_naive_utc_and_offset(epoch_naive, Utc))
            .num_microseconds()
            .expect("microseconds in range")
            .to_be_bytes()
            .to_vec();
        assert_eq!(
            decode_raw(&Type::TIMESTAMPTZ, &tz_raw),
            CellState::Decoded(DbValue::DateTime("2024-03-05 13:45:59 +0000".into()))
        );
    }

    #[test]
    fn bytea_keeps_exact_bytes_not_text() {
        let bytes = [0xDE, 0xAD, 0xBE, 0xEF];
        assert_eq!(
            decode_raw(&Type::BYTEA, &bytes),
            CellState::Decoded(DbValue::Binary(bytes.to_vec()))
        );
    }

    #[test]
    fn json_and_jsonb_decode_to_json_value() {
        let json = br#"{"a":1}"#;
        assert_eq!(
            decode_raw(&Type::JSON, json),
            CellState::Decoded(DbValue::Json(serde_json::json!({"a": 1})))
        );
        let mut jsonb = vec![1u8];
        jsonb.extend_from_slice(json);
        assert_eq!(
            decode_raw(&Type::JSONB, &jsonb),
            CellState::Decoded(DbValue::Json(serde_json::json!({"a": 1})))
        );
    }

    #[test]
    fn uuid_decodes_to_canonical_string() {
        let uuid =
            uuid::Uuid::parse_str("67e55044-10b1-426f-9247-bb680e5fe0c8").expect("uuid is valid");
        assert_eq!(
            decode_raw(&Type::UUID, &uuid.as_bytes().to_vec()),
            CellState::Decoded(DbValue::Uuid("67e55044-10b1-426f-9247-bb680e5fe0c8".into()))
        );
    }

    #[test]
    fn array_and_unknown_types_are_not_fabricated() {
        let array_ty = Type::INT4_ARRAY;
        let array_native = array_ty.name().to_string();
        let array = CellState::Decoded(DbValue::Unsupported {
            native_type: array_native.clone(),
            display: format!("<{array_native}>"),
        });
        let mut array_bytes = vec![2u8, 0, 0, 0, 1];
        array_bytes.extend_from_slice(&42_i32.to_be_bytes());
        assert_eq!(decode_raw(&array_ty, &array_bytes), array);

        let oid_native = Type::OID.name().to_string();
        assert_eq!(
            decode_raw(&Type::OID, &5222_u32.to_be_bytes()),
            CellState::Decoded(DbValue::Unsupported {
                native_type: oid_native.clone(),
                display: format!("<{oid_native}>"),
            })
        );
    }
}
