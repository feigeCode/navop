use one_core::storage::DatabaseType;

use super::execution::{
    SqlDocumentSnapshot, SqlExecutionRequest, SqlExecutionResultSource, SqlExecutionTarget,
    SqlMetadataScope, SqlTransactionMode,
};
use super::execution_error::{
    SqlExecutionErrorLocation, extract_error_location, map_execution_error_range,
};
use super::statement_ranges::{SqlDialect, SqlTextRange};

fn make_source() -> SqlExecutionResultSource {
    SqlExecutionResultSource {
        request_id: 1,
        document_revision: 5,
        source_range: Some(SqlTextRange {
            start_byte: 10,
            end_byte: 30,
        }),
        sql_fingerprint: 0,
        statement_index: Some(0),
    }
}

/// 构造一个把 `executed_sql` 放在 source_range 起始字节处的文档。
fn aligned_document(executed_sql: &str) -> String {
    let mut document = String::from("0123456789");
    document.push_str(executed_sql);
    document
}

#[test]
fn postgres_line_parses() {
    let location = extract_error_location(
        &DatabaseType::PostgreSQL,
        "ERROR:  column \"bogus\" does not exist\nLINE 2: SELECT bogus FROM t",
    )
    .unwrap();
    assert_eq!(
        SqlExecutionErrorLocation {
            line: Some(2),
            column: None,
            byte_offset: None,
        },
        location
    );
}

#[test]
fn postgres_at_character_parses() {
    let location = extract_error_location(
        &DatabaseType::PostgreSQL,
        "ERROR: syntax error at or near \"FROM\"\n at character 17",
    )
    .unwrap();
    assert_eq!(Some(16), location.byte_offset);
}

#[test]
fn mysql_at_line_parses() {
    let location = extract_error_location(
        &DatabaseType::MySQL,
        "You have an error in your SQL syntax ... near 'SELECT' at line 3",
    )
    .unwrap();
    assert_eq!(Some(3), location.line);
}

#[test]
fn sql_server_line_parses() {
    let location = extract_error_location(
        &DatabaseType::MSSQL,
        "Line 4: Incorrect syntax near 'FROM'.",
    )
    .unwrap();
    assert_eq!(Some(4), location.line);
}

#[test]
fn oracle_line_column_parses() {
    let location = extract_error_location(
        &DatabaseType::Oracle,
        "ORA-06550: line 2, column 7:\nPLS-00103: Encountered the symbol \"FROM\"",
    )
    .unwrap();
    assert_eq!(Some(2), location.line);
    assert_eq!(Some(7), location.column);
}

#[test]
fn sqlite_near_has_no_location() {
    let location = extract_error_location(
        &DatabaseType::SQLite,
        "near \"FROM\": syntax error",
    );
    assert!(location.is_none());
}

#[test]
fn generic_parenthesized_location_parses() {
    let location = extract_error_location(
        &DatabaseType::External {
            driver_id: "x".to_string(),
        },
        "syntax error (line 3, column 5)",
    )
    .unwrap();
    assert_eq!(Some(3), location.line);
    assert_eq!(Some(5), location.column);
}

#[test]
fn unrecognized_message_returns_none() {
    let location = extract_error_location(&DatabaseType::PostgreSQL, "connection refused");
    assert!(location.is_none());
}

#[test]
fn maps_line_column_to_document_offset() {
    let mut source = make_source();
    let executed_sql = "SELECT bogus FROM t";
    source.sql_fingerprint = super::execution::sql_fingerprint(executed_sql);
    let document = aligned_document(executed_sql);
    // LINE 1 列 8 在 "bogus" 处。
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: Some(1),
            column: Some(8),
            byte_offset: None,
        },
        &document,
        5,
    )
    .unwrap();
    // base=10, local offset: column 8 -> executed_sql[7] = "bogus" 首字符。
    assert_eq!(17, range.start_byte);
    assert!(range.end_byte > range.start_byte);
}

#[test]
fn maps_byte_offset_to_document() {
    let mut source = make_source();
    let executed_sql = "SELECT 1";
    source.sql_fingerprint = super::execution::sql_fingerprint(executed_sql);
    let document = aligned_document(executed_sql);
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: None,
            column: None,
            byte_offset: Some(7),
        },
        &document,
        5,
    )
    .unwrap();
    // executed_sql[7]='1'，base 10 -> 17。
    assert_eq!(17, range.start_byte);
}

#[test]
fn rejects_wrong_document_revision() {
    let mut source = make_source();
    let executed_sql = "SELECT 1";
    source.sql_fingerprint = super::execution::sql_fingerprint(executed_sql);
    let document = aligned_document(executed_sql);
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: None,
            column: None,
            byte_offset: Some(0),
        },
        &document,
        999, // 不匹配 revision 5
    );
    assert!(range.is_none());
}

#[test]
fn rejects_fingerprint_mismatch() {
    let source = make_source(); // fingerprint 0
    let executed_sql = "SELECT different_sql";
    let document = aligned_document(executed_sql);
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: Some(1),
            column: Some(1),
            byte_offset: None,
        },
        &document,
        5,
    );
    assert!(range.is_none());
}

#[test]
fn rejects_offset_outside_source_range() {
    let mut source = make_source();
    source.source_range = Some(SqlTextRange {
        start_byte: 10,
        end_byte: 15,
    });
    let executed_sql = "SELECT abcdefghijk";
    source.sql_fingerprint = super::execution::sql_fingerprint(executed_sql);
    let document = aligned_document(executed_sql);
    // column 20 远超 source range 长度。
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: Some(1),
            column: Some(20),
            byte_offset: None,
        },
        &document,
        5,
    );
    assert!(range.is_none());
}

#[test]
fn maps_multibyte_line_columns_correctly() {
    let mut source = make_source();
    let executed_sql = "SELECT '中文' FROM t";
    source.sql_fingerprint = super::execution::sql_fingerprint(executed_sql);
    let document = aligned_document(executed_sql);
    // LINE 1，column 指向 FROM 首字符（在多字节文本之后）。
    let range = map_execution_error_range(
        &source,
        executed_sql,
        SqlExecutionErrorLocation {
            line: Some(1),
            column: Some(17),
            byte_offset: None,
        },
        &document,
        5,
    )
    .unwrap();
    assert!(range.start_byte >= 10);
    assert_eq!("FROM", source_text(&document, &range));
}

fn source_text<'a>(document: &'a str, range: &SqlTextRange) -> &'a str {
    &document[range.start_byte..range.end_byte]
}

#[test]
fn result_source_to_request_source_consistency() {
    let scope = SqlMetadataScope::new("conn-1", DatabaseType::PostgreSQL, 1);
    let document = SqlDocumentSnapshot::new(
        5,
        std::sync::Arc::<str>::from("SELECT *\nSELECT 1"),
        SqlDialect::PostgreSql,
        scope,
    );
    let request = SqlExecutionRequest::new(
        1,
        document,
        SqlExecutionTarget::ExactRange(SqlTextRange {
            start_byte: 10,
            end_byte: 20,
        }),
        std::sync::Arc::<str>::from("SELECT 1"),
        Some(0),
        SqlTransactionMode::Auto,
    );
    let source = request.result_source();
    assert_eq!(5, source.document_revision);
    assert_eq!(Some(10), source.source_range.map(|r| r.start_byte));
}