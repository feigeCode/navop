use std::sync::Arc;

use one_core::storage::DatabaseType;

use super::execution::{
    SqlDocumentSnapshot, SqlExecutionRequest, SqlExecutionTarget, SqlMetadataScope,
    SqlTransactionMode, sql_fingerprint,
};
use super::statement_ranges::{SqlDialect, SqlTextRange};

#[test]
fn execution_request_preserves_revision_scope_target_and_result_source() {
    let scope = SqlMetadataScope::new("connection-1", DatabaseType::PostgreSQL, 9)
        .with_database(Some("app".to_string()))
        .with_schema(Some("public".to_string()));
    let document = SqlDocumentSnapshot::new(
        17,
        Arc::<str>::from("select 1;\nselect 2;"),
        SqlDialect::PostgreSql,
        scope.clone(),
    );
    let range = SqlTextRange {
        start_byte: 10,
        end_byte: 18,
    };
    let request = SqlExecutionRequest::new(
        23,
        document,
        SqlExecutionTarget::ExactRange(range),
        Arc::<str>::from("select 2"),
        Some(1),
        SqlTransactionMode::Manual,
    );

    assert_eq!(17, request.document_revision);
    assert_eq!(scope, request.scope);
    assert_eq!(Some(range), request.source_range);
    assert_eq!(Some(1), request.statement_index);
    assert_eq!(sql_fingerprint("select 2"), request.sql_fingerprint);

    let source = request.result_source();
    assert_eq!(23, source.request_id);
    assert_eq!(17, source.document_revision);
    assert_eq!(Some(range), source.source_range);
    assert_eq!(Some(1), source.statement_index);
    assert_eq!(request.sql_fingerprint, source.sql_fingerprint);
}

#[test]
fn sql_fingerprint_is_deterministic_and_content_sensitive() {
    assert_eq!(sql_fingerprint("select 1"), sql_fingerprint("select 1"));
    assert_ne!(sql_fingerprint("select 1"), sql_fingerprint("select 2"));
}

use std::sync::Arc as StdArc;

use super::execution::{
    SqlExecutionSourceMap, SqlExecutionStatementSource,
};

fn source_map() -> SqlExecutionSourceMap {
    SqlExecutionSourceMap {
        request_id: 7,
        document_revision: 3,
        statements: StdArc::from([
            SqlExecutionStatementSource {
                statement_index: 0,
                source_range: SqlTextRange { start_byte: 0, end_byte: 10 },
                sql_fingerprint: sql_fingerprint("select 1"),
                execution_sql: StdArc::<str>::from("select 1"),
            },
            SqlExecutionStatementSource {
                statement_index: 1,
                source_range: SqlTextRange { start_byte: 11, end_byte: 20 },
                sql_fingerprint: sql_fingerprint("select 2"),
                execution_sql: StdArc::<str>::from("select 2"),
            },
        ]),
    }
}

#[test]
fn source_map_resolves_by_statement_index() {
    let map = source_map();
    let source = map.resolve(Some(1), 0).unwrap();
    assert_eq!(1, source.statement_index);
    assert_eq!(11, source.source_range.start_byte);
}

#[test]
fn source_map_resolves_by_unique_fingerprint_when_index_missing() {
    let map = source_map();
    let source = map.resolve(None, sql_fingerprint("select 1")).unwrap();
    assert_eq!(0, source.statement_index);
}

#[test]
fn source_map_does_not_resolve_when_fingerprint_is_not_unique() {
    let map = SqlExecutionSourceMap {
        request_id: 7,
        document_revision: 3,
        statements: StdArc::from([
            SqlExecutionStatementSource {
                statement_index: 0,
                source_range: SqlTextRange { start_byte: 0, end_byte: 10 },
                sql_fingerprint: sql_fingerprint("select 1"),
                execution_sql: StdArc::<str>::from("select 1"),
            },
            SqlExecutionStatementSource {
                statement_index: 1,
                source_range: SqlTextRange { start_byte: 11, end_byte: 20 },
                sql_fingerprint: sql_fingerprint("select 1"),
                execution_sql: StdArc::<str>::from("select 1"),
            },
        ]),
    };
    assert!(map.resolve(None, sql_fingerprint("select 1")).is_none());
}

#[test]
fn source_map_result_source_carries_identity() {
    let map = source_map();
    let source = map.resolve(Some(1), 0).unwrap();
    let result = source.result_source(7, 3);
    assert_eq!(7, result.request_id);
    assert_eq!(3, result.document_revision);
    assert_eq!(Some(1), result.statement_index);
    assert_eq!(Some(SqlTextRange { start_byte: 11, end_byte: 20 }), result.source_range);
}

#[test]
fn source_map_out_of_range_index_falls_back_to_fingerprint() {
    let map = source_map();
    // 序号 99 不存在；fingerprint 唯一 -> 匹配到语句 0。
    let source = map.resolve(Some(99), sql_fingerprint("select 1")).unwrap();
    assert_eq!(0, source.statement_index);
}
