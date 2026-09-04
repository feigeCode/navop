use one_core::storage::DatabaseType;

use crate::DatabasePlugin;
use crate::executor::QueryColumnMeta;
use crate::types::{ColumnInfo, TableCellValue};

pub(crate) fn quote_string(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub(crate) fn parse_boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "t" | "1" => Some(true),
        "false" | "f" | "0" => Some(false),
        _ => None,
    }
}

pub(crate) fn strict_numeric_literal(value: &str) -> Option<&str> {
    let value = value.trim();
    if value.is_empty() {
        return None;
    }

    let bytes = value.as_bytes();
    let mut index = usize::from(matches!(bytes.first(), Some(b'+') | Some(b'-')));
    if index == bytes.len() {
        return None;
    }

    let integer_start = index;
    while index < bytes.len() && bytes[index].is_ascii_digit() {
        index += 1;
    }
    let integer_digits = index - integer_start;

    let mut fractional_digits = 0;
    if bytes.get(index) == Some(&b'.') {
        index += 1;
        let fractional_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        fractional_digits = index - fractional_start;
    }

    if integer_digits == 0 && fractional_digits == 0 {
        return None;
    }

    if matches!(bytes.get(index), Some(b'e') | Some(b'E')) {
        index += 1;
        if matches!(bytes.get(index), Some(b'+') | Some(b'-')) {
            index += 1;
        }
        let exponent_start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        if index == exponent_start {
            return None;
        }
    }

    (index == bytes.len()).then_some(value)
}

pub(crate) fn format_binary_literal_for_database(
    database_type: &DatabaseType,
    bytes: &[u8],
) -> String {
    let hex = hex::encode(bytes);
    match database_type {
        DatabaseType::PostgreSQL => format!("decode('{hex}', 'hex')"),
        DatabaseType::MSSQL => format!("0x{hex}"),
        DatabaseType::Oracle => format!("HEXTORAW('{hex}')"),
        DatabaseType::DuckDB => format!("from_hex('{hex}')"),
        DatabaseType::ClickHouse => format!("unhex('{hex}')"),
        // TDengine 与 MySQL 同臂处理(X'..' 十六进制字面量)。
        DatabaseType::MySQL
        | DatabaseType::SQLite
        | DatabaseType::TDengine
        | DatabaseType::External { .. } => {
            format!("X'{hex}'")
        }
    }
}

pub(crate) fn format_table_value_for_database(
    database_type: &DatabaseType,
    value: &TableCellValue,
    column: Option<&ColumnInfo>,
) -> String {
    match value {
        TableCellValue::Null => "NULL".to_string(),
        TableCellValue::Binary(bytes) => format_binary_literal_for_database(database_type, bytes),
        TableCellValue::Text(value) => {
            format_special_table_value_for_database(database_type, value, column)
                .unwrap_or_else(|| quote_string(value))
        }
    }
}

pub(crate) fn format_query_text_value<P>(
    plugin: &P,
    value: Option<&str>,
    meta: Option<&QueryColumnMeta>,
) -> String
where
    P: DatabasePlugin + ?Sized,
{
    let Some(value) = value else {
        return "NULL".to_string();
    };
    let column = meta.map(column_info_from_query_meta);
    plugin.format_table_change_value(&TableCellValue::Text(value.to_string()), column.as_ref())
}

fn column_info_from_query_meta(meta: &QueryColumnMeta) -> ColumnInfo {
    ColumnInfo {
        name: meta.name.clone(),
        data_type: meta.db_type.clone(),
        is_nullable: meta.nullable,
        is_primary_key: false,
        default_value: None,
        comment: None,
        charset: None,
        collation: None,
    }
}

pub(crate) fn format_special_table_value_for_database(
    database_type: &DatabaseType,
    value: &str,
    column: Option<&ColumnInfo>,
) -> Option<String> {
    let column = column?;
    crate::sql_literal_values::format_special_table_value(database_type, value, &column.data_type)
}

#[cfg(test)]
#[path = "sql_literal_tests.rs"]
mod tests;
