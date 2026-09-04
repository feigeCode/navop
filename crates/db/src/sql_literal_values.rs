use one_core::storage::DatabaseType;

use crate::binary_value::parse_binary_input;
use crate::sql_literal::{parse_boolean, quote_string, strict_numeric_literal};
use crate::sql_literal_types::{
    is_clickhouse_numeric_type, is_duckdb_numeric_type, is_mssql_numeric_type,
    is_mysql_numeric_type, is_oracle_numeric_type, is_postgres_numeric_type,
    is_sqlite_numeric_type, normalize_base_type, normalize_mysql_base_type, unwrap_clickhouse_type,
};

pub(crate) fn format_special_table_value(
    database_type: &DatabaseType,
    value: &str,
    data_type: &str,
) -> Option<String> {
    match database_type {
        DatabaseType::MySQL => format_mysql_value(value, data_type),
        // TDengine 与 MySQL 同臂处理(数值/布尔/二进制字面量规则一致)。
        DatabaseType::TDengine => format_mysql_value(value, data_type),
        DatabaseType::PostgreSQL => format_postgres_value(value, data_type),
        DatabaseType::SQLite => format_sqlite_value(value, data_type),
        DatabaseType::DuckDB => format_duckdb_value(value, data_type),
        DatabaseType::MSSQL => format_mssql_value(value, data_type),
        DatabaseType::Oracle => format_oracle_value(value, data_type),
        DatabaseType::ClickHouse => format_clickhouse_value(value, data_type),
        DatabaseType::External { .. } => None,
    }
}

fn format_mysql_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_mysql_base_type(data_type);
    if data_type == "BIT" {
        return Some(format_mysql_bit_literal(value).unwrap_or_else(|| quote_string(value)));
    }
    if is_one_of(&data_type, &["BOOLEAN", "BOOL"]) {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("1", "0"),
        ));
    }
    if is_mysql_numeric_type(&data_type) {
        return Some(format_numeric_or_quoted(value));
    }
    if is_one_of(
        &data_type,
        &[
            "BINARY",
            "VARBINARY",
            "TINYBLOB",
            "BLOB",
            "MEDIUMBLOB",
            "LONGBLOB",
        ],
    ) {
        return Some(format_binary_or_quoted(DatabaseType::MySQL, value));
    }
    None
}

pub(crate) fn format_mysql_bit_literal(value: &str) -> Option<String> {
    let value = value.trim();
    if let Some(boolean) = parse_word_boolean(value) {
        return Some(if boolean { "1" } else { "0" }.to_string());
    }
    if !value.is_empty() && value.chars().all(|character| character.is_ascii_digit()) {
        if value.len() > 1
            && value
                .chars()
                .all(|character| matches!(character, '0' | '1'))
        {
            return Some(format!("b'{value}'"));
        }
        if value.parse::<u64>().is_ok() {
            return Some(value.to_string());
        }
    }
    if let Some(bits) = mysql_quoted_bits(value) {
        return Some(format!("b'{bits}'"));
    }
    let hex = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))?;
    (!hex.is_empty() && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then(|| format!("0x{hex}"))
}

fn mysql_quoted_bits(value: &str) -> Option<&str> {
    let prefix = value.get(..2)?;
    if !prefix.eq_ignore_ascii_case("b'") || !value.ends_with('\'') {
        return None;
    }
    let bits = &value[2..value.len() - 1];
    (!bits.is_empty() && bits.chars().all(|ch| matches!(ch, '0' | '1'))).then_some(bits)
}

fn format_postgres_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_base_type(data_type);
    if is_one_of(&data_type, &["BOOLEAN", "BOOL"]) {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("TRUE", "FALSE"),
        ));
    }
    if is_one_of(&data_type, &["BIT", "BIT VARYING", "VARBIT"]) {
        return Some(format_bit_or_quoted(value));
    }
    if data_type == "BYTEA" {
        return Some(format_binary_or_quoted(DatabaseType::PostgreSQL, value));
    }
    is_postgres_numeric_type(&data_type).then(|| format_numeric_or_quoted(value))
}

fn format_mssql_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_base_type(data_type);
    if data_type == "BIT" {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("1", "0"),
        ));
    }
    if is_one_of(&data_type, &["NCHAR", "NVARCHAR", "NTEXT"]) {
        return Some(format!("N{}", quote_string(value)));
    }
    if is_one_of(&data_type, &["BINARY", "VARBINARY", "IMAGE"]) {
        return Some(format_binary_or_quoted(DatabaseType::MSSQL, value));
    }
    is_mssql_numeric_type(&data_type).then(|| format_numeric_or_quoted(value))
}

fn format_sqlite_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_base_type(data_type);
    if is_one_of(&data_type, &["BOOLEAN", "BOOL"]) {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("1", "0"),
        ));
    }
    if is_one_of(&data_type, &["BLOB", "BINARY", "VARBINARY"]) {
        return Some(format_binary_or_quoted(DatabaseType::SQLite, value));
    }
    is_sqlite_numeric_type(&data_type).then(|| format_numeric_or_quoted(value))
}

fn format_duckdb_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_base_type(data_type);
    if is_one_of(&data_type, &["BOOLEAN", "BOOL", "LOGICAL"]) {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("TRUE", "FALSE"),
        ));
    }
    if is_one_of(&data_type, &["BLOB", "BINARY", "VARBINARY"]) {
        return Some(format_binary_or_quoted(DatabaseType::DuckDB, value));
    }
    is_duckdb_numeric_type(&data_type).then(|| format_numeric_or_quoted(value))
}

fn format_oracle_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = normalize_base_type(data_type);
    if data_type == "BOOLEAN" {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::words("TRUE", "FALSE"),
        ));
    }
    if is_one_of(&data_type, &["RAW", "LONG RAW", "BLOB"]) {
        return Some(format_binary_or_quoted(DatabaseType::Oracle, value));
    }
    is_oracle_numeric_type(&data_type).then(|| format_numeric_or_quoted(value))
}

fn format_clickhouse_value(value: &str, data_type: &str) -> Option<String> {
    let data_type = unwrap_clickhouse_type(data_type);
    let base_type = normalize_base_type(&data_type);
    if is_one_of(&base_type, &["BOOL", "BOOLEAN"]) {
        return Some(format_boolean_or_quoted(
            value,
            BooleanDialect::numeric("true", "false"),
        ));
    }
    is_clickhouse_numeric_type(&base_type).then(|| format_numeric_or_quoted(value))
}

struct BooleanDialect<'a> {
    true_literal: &'a str,
    false_literal: &'a str,
    allow_numeric: bool,
}

impl<'a> BooleanDialect<'a> {
    fn numeric(true_literal: &'a str, false_literal: &'a str) -> Self {
        Self {
            true_literal,
            false_literal,
            allow_numeric: true,
        }
    }

    fn words(true_literal: &'a str, false_literal: &'a str) -> Self {
        Self {
            true_literal,
            false_literal,
            allow_numeric: false,
        }
    }
}

fn format_boolean_or_quoted(value: &str, dialect: BooleanDialect<'_>) -> String {
    let boolean = dialect
        .allow_numeric
        .then(|| parse_boolean(value))
        .flatten()
        .or_else(|| parse_word_boolean(value));
    match boolean {
        Some(true) => dialect.true_literal.to_string(),
        Some(false) => dialect.false_literal.to_string(),
        None => quote_string(value),
    }
}

fn parse_word_boolean(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "t" => Some(true),
        "false" | "f" => Some(false),
        _ => None,
    }
}

fn format_bit_or_quoted(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|character| matches!(character, '0' | '1'))
    {
        format!("B'{value}'")
    } else {
        quote_string(value)
    }
}

fn format_binary_or_quoted(database_type: DatabaseType, value: &str) -> String {
    parse_binary_input(value)
        .ok()
        .map(|bytes| crate::sql_literal::format_binary_literal_for_database(&database_type, &bytes))
        .unwrap_or_else(|| quote_string(value))
}

fn format_numeric_or_quoted(value: &str) -> String {
    strict_numeric_literal(value)
        .map(str::to_string)
        .unwrap_or_else(|| quote_string(value))
}

fn is_one_of(data_type: &str, candidates: &[&str]) -> bool {
    candidates.contains(&data_type)
}
