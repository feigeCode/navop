use super::*;

fn column(data_type: &str) -> ColumnInfo {
    ColumnInfo {
        name: "value".to_string(),
        data_type: data_type.to_string(),
        is_nullable: true,
        is_primary_key: false,
        default_value: None,
        comment: None,
        charset: None,
        collation: None,
    }
}

fn format(database_type: DatabaseType, data_type: &str, value: &str) -> String {
    format_table_value_for_database(
        &database_type,
        &TableCellValue::Text(value.to_string()),
        Some(&column(data_type)),
    )
}

#[test]
fn validates_numeric_literals_without_allowing_sql_fragments() {
    for valid in ["0", "-12", "+12.50", ".5", "1.", "6.02e23", "-1E-3"] {
        assert_eq!(Some(valid), strict_numeric_literal(valid));
    }
    for invalid in ["", "+", ".", "1e", "NaN", "Infinity", "1; DROP TABLE x"] {
        assert_eq!(None, strict_numeric_literal(invalid));
    }
}

#[test]
fn formats_mysql_typed_values() {
    assert_eq!("42", format(DatabaseType::MySQL, "int unsigned", "42"));
    assert_eq!("1", format(DatabaseType::MySQL, "boolean", "true"));
    assert_eq!("1", format(DatabaseType::MySQL, "BIT", "1"));
    assert_eq!("0", format(DatabaseType::MySQL, "BIT(1)", "false"));
    assert_eq!("b'1010'", format(DatabaseType::MySQL, "BIT(4)", "1010"));
    assert_eq!(
        "b'1010'",
        format(DatabaseType::MySQL, "MYSQL_TYPE_BIT", "B'1010'")
    );
    assert_eq!("0x0F", format(DatabaseType::MySQL, "BIT(8)", "0X0F"));
    assert_eq!(
        "X'deadbeef'",
        format(DatabaseType::MySQL, "BLOB", "3q2+7w==")
    );
    assert_eq!(
        "X'deadbeef'",
        format(DatabaseType::MySQL, "BLOB", "0xDEADBEEF")
    );
    assert_eq!("X''", format(DatabaseType::MySQL, "BLOB", ""));
    assert_eq!("'1'", format(DatabaseType::MySQL, "VARCHAR(20)", "1"));
    assert_eq!(
        "'1'' OR 1=1'",
        format(DatabaseType::MySQL, "BIT(1)", "1' OR 1=1")
    );
}

#[test]
fn formats_postgres_typed_values() {
    assert_eq!("TRUE", format(DatabaseType::PostgreSQL, "boolean", "1"));
    assert_eq!(
        "B'0101'",
        format(DatabaseType::PostgreSQL, "bit(4)", "0101")
    );
    assert_eq!(
        "decode('deadbeef', 'hex')",
        format(DatabaseType::PostgreSQL, "bytea", "3q2+7w==")
    );
    assert_eq!(
        "decode('deadbeef', 'hex')",
        format(DatabaseType::PostgreSQL, "bytea", "0Xdeadbeef")
    );
    assert_eq!(
        "12.50",
        format(DatabaseType::PostgreSQL, "numeric(10,2)", "12.50")
    );
    assert_eq!("'1'", format(DatabaseType::PostgreSQL, "varchar", "1"));
}

#[test]
fn formats_mssql_typed_values() {
    assert_eq!("0", format(DatabaseType::MSSQL, "bit", "false"));
    assert_eq!(
        "N'中文 O''Brien'",
        format(DatabaseType::MSSQL, "nvarchar(100)", "中文 O'Brien")
    );
    assert_eq!(
        "0xdeadbeef",
        format(DatabaseType::MSSQL, "varbinary(max)", "3q2+7w==")
    );
    assert_eq!(
        "0xdeadbeef",
        format(DatabaseType::MSSQL, "varbinary(max)", "0xDEADBEEF")
    );
    assert_eq!("42", format(DatabaseType::MSSQL, "int", "42"));
    assert_eq!("'1'", format(DatabaseType::MSSQL, "varchar(10)", "1"));
}

#[test]
fn formats_sqlite_typed_values() {
    assert_eq!("1", format(DatabaseType::SQLite, "boolean", "true"));
    assert_eq!(
        "X'deadbeef'",
        format(DatabaseType::SQLite, "blob", "3q2+7w==")
    );
    assert_eq!(
        "X'deadbeef'",
        format(DatabaseType::SQLite, "blob", "0xDEADBEEF")
    );
    assert_eq!("12.5", format(DatabaseType::SQLite, "decimal(5,2)", "12.5"));
    assert_eq!("'001'", format(DatabaseType::SQLite, "text", "001"));
}

#[test]
fn formats_duckdb_typed_values() {
    assert_eq!("FALSE", format(DatabaseType::DuckDB, "bool", "0"));
    assert_eq!(
        "from_hex('deadbeef')",
        format(DatabaseType::DuckDB, "blob", "3q2+7w==")
    );
    assert_eq!(
        "from_hex('deadbeef')",
        format(DatabaseType::DuckDB, "blob", "0xDEADBEEF")
    );
    assert_eq!("42", format(DatabaseType::DuckDB, "int8", "42"));
    assert_eq!("'001'", format(DatabaseType::DuckDB, "varchar", "001"));
}

#[test]
fn formats_oracle_typed_values() {
    assert_eq!("TRUE", format(DatabaseType::Oracle, "boolean", "true"));
    assert_eq!("'1'", format(DatabaseType::Oracle, "boolean", "1"));
    assert_eq!(
        "HEXTORAW('deadbeef')",
        format(DatabaseType::Oracle, "raw(16)", "3q2+7w==")
    );
    assert_eq!(
        "HEXTORAW('deadbeef')",
        format(DatabaseType::Oracle, "raw(16)", "0xDEADBEEF")
    );
    assert_eq!(
        "12.50",
        format(DatabaseType::Oracle, "decimal(10,2)", "12.50")
    );
    assert_eq!(
        "'NULL'",
        format(DatabaseType::Oracle, "varchar2(10)", "NULL")
    );
}

#[test]
fn formats_clickhouse_typed_values() {
    assert_eq!(
        "true",
        format(DatabaseType::ClickHouse, "Nullable(Boolean)", "1")
    );
    assert_eq!(
        "123",
        format(
            DatabaseType::ClickHouse,
            "LowCardinality(Nullable(UInt64))",
            "123"
        )
    );
    assert_eq!(
        "12.50",
        format(DatabaseType::ClickHouse, "Decimal(10, 2)", "12.50")
    );
    assert_eq!("'123'", format(DatabaseType::ClickHouse, "String", "123"));
}

#[test]
fn invalid_special_values_fall_back_to_escaped_strings() {
    for database_type in [
        DatabaseType::MySQL,
        DatabaseType::PostgreSQL,
        DatabaseType::SQLite,
        DatabaseType::DuckDB,
        DatabaseType::MSSQL,
        DatabaseType::Oracle,
        DatabaseType::ClickHouse,
    ] {
        assert_eq!(
            "'1; DROP TABLE x'",
            format(database_type, "INTEGER", "1; DROP TABLE x")
        );
    }
    assert_eq!(
        "'not-base64!''x'",
        format(DatabaseType::SQLite, "blob", "not-base64!'x")
    );
    assert_eq!("'0xABC'", format(DatabaseType::SQLite, "blob", "0xABC"));
    assert_eq!(
        "'0xnothex'",
        format(DatabaseType::MySQL, "blob", "0xnothex")
    );
}

#[test]
fn sql_null_is_distinct_from_text_null() {
    let text_null = format_table_value_for_database(
        &DatabaseType::PostgreSQL,
        &TableCellValue::Text("NULL".to_string()),
        Some(&column("text")),
    );
    let sql_null = format_table_value_for_database(
        &DatabaseType::PostgreSQL,
        &TableCellValue::Null,
        Some(&column("text")),
    );
    assert_eq!("'NULL'", text_null);
    assert_eq!("NULL", sql_null);
}
