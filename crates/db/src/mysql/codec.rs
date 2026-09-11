//! MySQL wire-value codec: convert one owned `mysql_async::Value` into a typed
//! [`db_value::CellState`] plus the legacy string display used by `rows`.
//!
//! This keeps every MySQL-specific display rule (bounded hex binary preview,
//! BIT bit-strings, JSON, charset-aware GBK text, binary/ambiguous detection)
//! in one place while the connection layer assembles the typed
//! [`db_value::ResultBatch`].

use mysql_async::{
    Value,
    consts::{ColumnFlags, ColumnType},
};
use mysql_common::collations::{Collation, CollationId};

use crate::query_result_normalization::{MySqlTextDecoder, mysql_text_decoder_for_charset};

use db_value::{CellState, DbValue, FloatWidth};

/// MySQL's column packet calls this field `character_set`, but it carries the
/// collation ID. Collation 63 is also used when a server sends text through
/// `character_set_results=binary`; `BINARY_FLAG` distinguishes semantic byte
/// columns from that result-encoding choice. It must never be inferred to mean
/// "text".
pub(crate) const MYSQL_BINARY_COLLATION_ID: u16 = 63;

/// Resolved result result-encoding for a MySQL result column.
pub(crate) struct MysqlResultEncoding {
    pub charset: Option<String>,
    pub collation: Option<String>,
    pub collation_id: u16,
}

/// Resolve a MySQL result-column collation ID into charset/collation names.
///
/// The raw `collation_id` is retained even when the local collation table has
/// no entry (detected as `UNKNOWN_COLLATION_ID`), so diagnostics and forward
/// compatibility never lose the wire value.
pub(crate) fn result_encoding(collation_id: u16) -> MysqlResultEncoding {
    let id = CollationId::from(collation_id);
    if id == CollationId::UNKNOWN_COLLATION_ID {
        return MysqlResultEncoding {
            charset: None,
            collation: None,
            collation_id,
        };
    }

    let collation = Collation::from(id);
    MysqlResultEncoding {
        charset: Some(collation.charset().to_string()),
        collation: Some(collation.collation().to_string()),
        collation_id,
    }
}

/// Decode one owned wire value into its typed [`CellState`] and its legacy
/// string display. When a binary sidecar is warranted, the typed value is
/// [`DbValue::Binary`] (projected into `binary_cells` by the result builder)
/// and the display is a bounded hex preview.
pub(crate) fn decode_cell(
    value: Value,
    column: Option<&mysql_async::Column>,
) -> (CellState, Option<String>) {
    let Value::Bytes(bytes) = value else {
        return decode_scalar(value);
    };

    let Some(column) = column else {
        return decode_untyped_bytes(bytes);
    };

    if column.column_type() == ColumnType::MYSQL_TYPE_BIT {
        let display = format_bit_bytes(&bytes, column.column_length());
        return (
            CellState::Decoded(DbValue::Text(display.clone())),
            Some(display),
        );
    }

    if is_binary_wire_value(column.column_type(), column.flags(), column.character_set()) {
        let display = format_as_hex(&bytes);
        return (CellState::Decoded(DbValue::Binary(bytes)), Some(display));
    }

    if column.column_type() == ColumnType::MYSQL_TYPE_JSON {
        return decode_json(bytes);
    }

    if is_character_wire_type(column.column_type()) {
        return decode_character(bytes, column);
    }

    decode_untyped_bytes(bytes)
}

fn decode_scalar(value: Value) -> (CellState, Option<String>) {
    match value {
        Value::NULL => (CellState::Decoded(DbValue::Null), None),
        Value::Bytes(bytes) => decode_untyped_bytes(bytes),
        Value::Int(i) => {
            let display = i.to_string();
            (
                CellState::Decoded(DbValue::Integer(display.clone())),
                Some(display),
            )
        }
        Value::UInt(u) => {
            let display = u.to_string();
            (
                CellState::Decoded(DbValue::Unsigned(display.clone())),
                Some(display),
            )
        }
        Value::Float(f) => {
            let display = f.to_string();
            (
                CellState::Decoded(DbValue::Float {
                    value: display.clone(),
                    width: FloatWidth::F32,
                }),
                Some(display),
            )
        }
        Value::Double(d) => {
            let display = d.to_string();
            (
                CellState::Decoded(DbValue::Float {
                    value: display.clone(),
                    width: FloatWidth::F64,
                }),
                Some(display),
            )
        }
        Value::Date(year, month, day, hour, min, sec, micro) => {
            let display = format_datetime(year, month, day, hour, min, sec, micro);
            let state = if hour == 0 && min == 0 && sec == 0 && micro == 0 {
                DbValue::Date(display.clone())
            } else {
                DbValue::DateTime(display.clone())
            };
            (CellState::Decoded(state), Some(display))
        }
        Value::Time(is_neg, days, hours, minutes, seconds, micros) => {
            let display = format_time(is_neg, days, hours, minutes, seconds, micros);
            (
                CellState::Decoded(DbValue::Time(display.clone())),
                Some(display),
            )
        }
    }
}

fn decode_untyped_bytes(bytes: Vec<u8>) -> (CellState, Option<String>) {
    if is_valid_utf8_text(&bytes) {
        let display = String::from_utf8_lossy(&bytes).into_owned();
        return (
            CellState::Decoded(DbValue::Text(display.clone())),
            Some(display),
        );
    }
    let display = format_as_hex(&bytes);
    let display_str = display.clone();
    (
        CellState::Decoded(DbValue::LegacyText(display_str)),
        Some(display),
    )
}

/// Decode a JSON wire value. UTF-8 text is kept as text (parsed when possible
/// for the typed model); bytes that are not valid UTF-8 stay lossless as a
/// binary sidecar.
fn decode_json(bytes: Vec<u8>) -> (CellState, Option<String>) {
    match String::from_utf8(bytes) {
        Ok(text) => {
            let state = serde_json::from_str::<serde_json::Value>(&text)
                .ok()
                .map(DbValue::Json)
                .unwrap_or_else(|| DbValue::Text(text.clone()));
            (CellState::Decoded(state), Some(text))
        }
        Err(error) => {
            let bytes = error.into_bytes();
            let display = format_as_hex(&bytes);
            (CellState::Decoded(DbValue::Binary(bytes)), Some(display))
        }
    }
}

fn decode_character(bytes: Vec<u8>, column: &mysql_async::Column) -> (CellState, Option<String>) {
    let decoder = column_text_decoder(column);
    if let Some(decoder) = decoder.filter(|decoder| decoder.is_valid(&bytes)) {
        let display = decoder.decode_validated(bytes);
        return (
            CellState::Decoded(DbValue::Text(display.clone())),
            Some(display),
        );
    }
    let display = format_as_hex(&bytes);
    (CellState::Decoded(DbValue::Binary(bytes)), Some(display))
}

fn column_text_decoder(column: &mysql_async::Column) -> Option<MySqlTextDecoder> {
    let encoding = result_encoding(column.character_set());
    encoding
        .charset
        .as_deref()
        .and_then(mysql_text_decoder_for_charset)
}

fn is_character_wire_type(column_type: ColumnType) -> bool {
    matches!(
        column_type,
        ColumnType::MYSQL_TYPE_STRING
            | ColumnType::MYSQL_TYPE_VAR_STRING
            | ColumnType::MYSQL_TYPE_VARCHAR
            | ColumnType::MYSQL_TYPE_TINY_BLOB
            | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
            | ColumnType::MYSQL_TYPE_LONG_BLOB
            | ColumnType::MYSQL_TYPE_BLOB
            | ColumnType::MYSQL_TYPE_ENUM
            | ColumnType::MYSQL_TYPE_SET
    )
}

fn is_valid_utf8_text(bytes: &[u8]) -> bool {
    match std::str::from_utf8(bytes) {
        Ok(s) => s
            .chars()
            .all(|c| !c.is_control() || c == '\n' || c == '\r' || c == '\t'),
        Err(_) => false,
    }
}

/// A binary value is only treated as byte data when the column keeps the
/// binary field flag together with MySQL's binary pseudo-collation and uses one
/// of the binary-capable string families. A `_bin` character collation or a
/// bare `BINARY_FLAG` alone never implies raw bytes.
pub(crate) fn is_binary_wire_value(
    column_type: ColumnType,
    flags: ColumnFlags,
    collation_id: u16,
) -> bool {
    flags.contains(ColumnFlags::BINARY_FLAG)
        && collation_id == MYSQL_BINARY_COLLATION_ID
        && matches!(
            column_type,
            ColumnType::MYSQL_TYPE_STRING
                | ColumnType::MYSQL_TYPE_VAR_STRING
                | ColumnType::MYSQL_TYPE_VARCHAR
                | ColumnType::MYSQL_TYPE_TINY_BLOB
                | ColumnType::MYSQL_TYPE_MEDIUM_BLOB
                | ColumnType::MYSQL_TYPE_LONG_BLOB
                | ColumnType::MYSQL_TYPE_BLOB
                | ColumnType::MYSQL_TYPE_GEOMETRY
                | ColumnType::MYSQL_TYPE_VECTOR
        )
}

fn format_bit_bytes(bytes: &[u8], bit_length: u32) -> String {
    let bits_per_byte = u8::BITS as usize;
    let available_bits = bytes.len().saturating_mul(bits_per_byte);
    if available_bits == 0 {
        return String::new();
    }

    let requested_bits = usize::try_from(bit_length)
        .ok()
        .filter(|bits| *bits > 0)
        .unwrap_or(available_bits);
    let display_bits = requested_bits.min(available_bits);
    let first_bit = available_bits - display_bits;

    (first_bit..available_bits)
        .map(|bit_index| {
            let byte = bytes[bit_index / bits_per_byte];
            let shift = bits_per_byte - 1 - (bit_index % bits_per_byte);
            if byte & (1u8 << shift) == 0 { '0' } else { '1' }
        })
        .collect()
}

fn format_as_hex(bytes: &[u8]) -> String {
    const MAX_HEX_DISPLAY: usize = 1024;
    let display_bytes = if bytes.len() > MAX_HEX_DISPLAY {
        &bytes[..MAX_HEX_DISPLAY]
    } else {
        bytes
    };
    let hex: String = display_bytes.iter().map(|b| format!("{:02X}", b)).collect();
    if bytes.len() > MAX_HEX_DISPLAY {
        format!("0x{}... ({} bytes)", hex, bytes.len())
    } else {
        format!("0x{}", hex)
    }
}

fn format_datetime(
    year: u16,
    month: u8,
    day: u8,
    hour: u8,
    min: u8,
    sec: u8,
    micro: u32,
) -> String {
    if year == 0 && month == 0 && day == 0 {
        return String::from("0000-00-00");
    }
    if hour == 0 && min == 0 && sec == 0 && micro == 0 {
        format!("{:04}-{:02}-{:02}", year, month, day)
    } else if micro == 0 {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
            year, month, day, hour, min, sec
        )
    } else {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:06}",
            year, month, day, hour, min, sec, micro
        )
    }
}

fn format_time(
    is_neg: bool,
    days: u32,
    hours: u8,
    minutes: u8,
    seconds: u8,
    micros: u32,
) -> String {
    let sign = if is_neg { "-" } else { "" };
    let total_hours = (days * 24) + hours as u32;
    if micros == 0 {
        format!("{}{}:{:02}:{:02}", sign, total_hours, minutes, seconds)
    } else {
        format!(
            "{}{}:{:02}:{:02}.{:06}",
            sign, total_hours, minutes, seconds, micros
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column_with_charset(column_type: ColumnType, collation_id: u16) -> mysql_async::Column {
        mysql_async::Column::new(column_type).with_character_set(collation_id)
    }

    #[test]
    fn binary_wire_value_detection_accepts_only_binary_string_families() {
        const UTF8MB4_BIN_COLLATION_ID: u16 = 46;
        let accepted_types = [
            ColumnType::MYSQL_TYPE_STRING,
            ColumnType::MYSQL_TYPE_VAR_STRING,
            ColumnType::MYSQL_TYPE_VARCHAR,
            ColumnType::MYSQL_TYPE_TINY_BLOB,
            ColumnType::MYSQL_TYPE_MEDIUM_BLOB,
            ColumnType::MYSQL_TYPE_LONG_BLOB,
            ColumnType::MYSQL_TYPE_BLOB,
            ColumnType::MYSQL_TYPE_GEOMETRY,
            ColumnType::MYSQL_TYPE_VECTOR,
        ];

        for column_type in accepted_types {
            assert!(
                is_binary_wire_value(
                    column_type,
                    ColumnFlags::BINARY_FLAG,
                    MYSQL_BINARY_COLLATION_ID
                ),
                "{column_type:?} should be binary for MySQL's binary pseudo-collation"
            );
            assert!(
                !is_binary_wire_value(
                    column_type,
                    ColumnFlags::BINARY_FLAG,
                    UTF8MB4_BIN_COLLATION_ID
                ),
                "{column_type:?} must not treat a character _bin collation as binary bytes"
            );
            assert!(
                !is_binary_wire_value(column_type, ColumnFlags::BINARY_FLAG, 0,),
                "{column_type:?} must require the binary collation id"
            );
            assert!(
                !is_binary_wire_value(column_type, ColumnFlags::empty(), MYSQL_BINARY_COLLATION_ID),
                "{column_type:?} must require the binary field flag"
            );
        }
    }

    #[test]
    fn binary_wire_value_detection_rejects_non_binary_value_types() {
        for column_type in [
            ColumnType::MYSQL_TYPE_BIT,
            ColumnType::MYSQL_TYPE_TINY,
            ColumnType::MYSQL_TYPE_LONG,
            ColumnType::MYSQL_TYPE_LONGLONG,
            ColumnType::MYSQL_TYPE_DOUBLE,
            ColumnType::MYSQL_TYPE_NEWDECIMAL,
            ColumnType::MYSQL_TYPE_DATE,
            ColumnType::MYSQL_TYPE_DATETIME,
            ColumnType::MYSQL_TYPE_TIMESTAMP,
            ColumnType::MYSQL_TYPE_JSON,
        ] {
            assert!(
                !is_binary_wire_value(
                    column_type,
                    ColumnFlags::BINARY_FLAG,
                    MYSQL_BINARY_COLLATION_ID
                ),
                "{column_type:?} must remain a typed non-binary value"
            );
        }
    }

    #[test]
    fn binary_query_cell_moves_exact_bytes_into_a_bounded_preview_sidecar() {
        let column = mysql_async::Column::new(ColumnType::MYSQL_TYPE_LONG_BLOB)
            .with_flags(ColumnFlags::BINARY_FLAG | ColumnFlags::BLOB_FLAG)
            .with_character_set(MYSQL_BINARY_COLLATION_ID);
        let bytes = "文".repeat(800).into_bytes();

        let (state, display) = decode_cell(Value::Bytes(bytes.clone()), Some(&column));

        let display = display.expect("binary cell should keep a display preview");
        assert!(display.ends_with(&format!("... ({} bytes)", bytes.len())));
        assert!(display.len() < bytes.len());
        assert_eq!(state, CellState::Decoded(DbValue::Binary(bytes)));
    }

    #[test]
    fn ambiguous_binary_result_encoding_remains_lossless() {
        let column =
            column_with_charset(ColumnType::MYSQL_TYPE_VAR_STRING, MYSQL_BINARY_COLLATION_ID);
        let bytes = "utf8mb4_0900_ai_ci 中文".as_bytes().to_vec();

        let (state, display) = decode_cell(Value::Bytes(bytes.clone()), Some(&column));

        assert_eq!(
            display.as_deref(),
            Some("0x757466386D62345F303930305F61695F636920E4B8ADE69687")
        );
        assert_eq!(state, CellState::Decoded(DbValue::Binary(bytes)));
    }

    #[test]
    fn ordinary_text_query_cell_is_not_promoted_to_binary() {
        let column = column_with_charset(ColumnType::MYSQL_TYPE_LONG_BLOB, 45);
        let bytes = "文".repeat(800).into_bytes();

        let (state, display) = decode_cell(Value::Bytes(bytes.clone()), Some(&column));

        assert_eq!(display.as_deref(), std::str::from_utf8(&bytes).ok());
        assert_eq!(state, CellState::Decoded(DbValue::Text("文".repeat(800))));
    }

    #[test]
    fn binary_flag_does_not_override_character_collation() {
        let column = mysql_async::Column::new(ColumnType::MYSQL_TYPE_VAR_STRING)
            .with_flags(ColumnFlags::BINARY_FLAG)
            .with_character_set(45);
        let bytes = "带 BINARY 属性的文本".repeat(80).into_bytes();

        let (state, display) = decode_cell(Value::Bytes(bytes.clone()), Some(&column));

        assert_eq!(display.as_deref(), std::str::from_utf8(&bytes).ok());
        assert_eq!(
            state,
            CellState::Decoded(DbValue::Text("带 BINARY 属性的文本".repeat(80))),
            "BINARY_FLAG only describes comparison/padding semantics and must not override a character collation"
        );
    }

    #[test]
    fn gbk_text_query_cell_uses_result_column_charset() {
        use encoding_rs::GBK;

        let column = column_with_charset(ColumnType::MYSQL_TYPE_LONG_BLOB, 28);
        let (bytes, _, had_errors) = GBK.encode("中文");
        assert!(!had_errors);

        let (state, display) = decode_cell(Value::Bytes(bytes.into_owned()), Some(&column));

        assert_eq!(display.as_deref(), Some("中文"));
        assert_eq!(state, CellState::Decoded(DbValue::Text("中文".to_string())));
    }

    #[test]
    fn invalid_text_bytes_remain_lossless_in_a_binary_sidecar() {
        let column = column_with_charset(ColumnType::MYSQL_TYPE_LONG_BLOB, 45);
        let bytes = vec![0xff, 0xfe];

        let (state, display) = decode_cell(Value::Bytes(bytes.clone()), Some(&column));

        assert_eq!(display.as_deref(), Some("0xFFFE"));
        assert_eq!(state, CellState::Decoded(DbValue::Binary(bytes)));
    }

    #[test]
    fn mysql_result_encoding_resolves_known_and_unknown_collations() {
        let utf8mb4 = result_encoding(45);
        assert_eq!(utf8mb4.charset.as_deref(), Some("utf8mb4"));
        assert_eq!(utf8mb4.collation.as_deref(), Some("utf8mb4_general_ci"));
        assert_eq!(utf8mb4.collation_id, 45);

        let binary = result_encoding(MYSQL_BINARY_COLLATION_ID);
        assert_eq!(binary.charset.as_deref(), Some("binary"));
        assert_eq!(binary.collation.as_deref(), Some("binary"));

        let unknown = result_encoding(u16::MAX);
        assert!(unknown.charset.is_none());
        assert!(unknown.collation.is_none());
        assert_eq!(unknown.collation_id, u16::MAX);
    }

    #[test]
    fn bit_values_are_formatted_as_editable_fixed_width_bit_strings() {
        assert_eq!("0", format_bit_bytes(&[0], 1));
        assert_eq!("1", format_bit_bytes(&[1], 1));
        assert_eq!("0010", format_bit_bytes(&[0b0010], 4));
        assert_eq!("1010", format_bit_bytes(&[0b1010], 4));
        assert_eq!(
            "100000010",
            format_bit_bytes(&[0b0000_0001, 0b0000_0010], 9)
        );
        assert_eq!("00000010", format_bit_bytes(&[0b0000_0010], 0));
        assert_eq!("", format_bit_bytes(&[], 1));
    }

    #[test]
    fn bit_column_value_decodes_as_fixed_width_bit_text() {
        let column = mysql_async::Column::new(ColumnType::MYSQL_TYPE_BIT).with_column_length(4);

        let (state, display) = decode_cell(Value::Bytes(vec![0b0010]), Some(&column));

        assert_eq!(display.as_deref(), Some("0010"));
        assert_eq!(state, CellState::Decoded(DbValue::Text("0010".to_string())));
        assert!(!is_binary_wire_value(
            column.column_type(),
            column.flags(),
            column.character_set(),
        ));
    }

    #[test]
    fn non_byte_scalar_values_map_to_typed_variants() {
        assert_eq!(
            decode_cell(Value::Int(-5), None),
            (
                CellState::Decoded(DbValue::Integer("-5".to_string())),
                Some("-5".to_string())
            )
        );
        assert_eq!(
            decode_cell(Value::UInt(7), None),
            (
                CellState::Decoded(DbValue::Unsigned("7".to_string())),
                Some("7".to_string())
            )
        );
        assert_eq!(
            decode_cell(Value::NULL, None),
            (CellState::Decoded(DbValue::Null), None)
        );
    }

    #[test]
    fn date_and_datetime_map_to_distinct_variants() {
        let (state, display) = decode_cell(Value::Date(2024, 1, 2, 0, 0, 0, 0), None);
        assert_eq!(
            state,
            CellState::Decoded(DbValue::Date("2024-01-02".to_string()))
        );
        assert_eq!(display.as_deref(), Some("2024-01-02"));

        let (state, display) = decode_cell(Value::Date(2024, 1, 2, 12, 30, 45, 6), None);
        assert_eq!(
            state,
            CellState::Decoded(DbValue::DateTime("2024-01-02 12:30:45.000006".to_string()))
        );
        assert_eq!(display.as_deref(), Some("2024-01-02 12:30:45.000006"));
    }

    #[test]
    fn untyped_invalid_utf8_stays_hex_without_a_sidecar() {
        let (state, display) = decode_cell(Value::Bytes(vec![0xff, 0xfe]), None);
        assert_eq!(display.as_deref(), Some("0xFFFE"));
        assert_eq!(
            state,
            CellState::Decoded(DbValue::LegacyText("0xFFFE".to_string()))
        );
    }
}
