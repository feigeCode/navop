use db::executor::{QueryCellRef, QueryResult};

pub fn assert_no_sql_errors(results: &[db::executor::SqlResult], context: &str) {
    for result in results {
        if let db::executor::SqlResult::Error(error) = result {
            panic!("{context} failed: {}", error.message);
        }
    }
}

pub fn assert_columns(result: &QueryResult, expected: &[&str]) {
    assert_eq!(
        result.columns,
        expected.iter().map(|v| v.to_string()).collect::<Vec<_>>()
    );
}

pub fn assert_cell(result: &QueryResult, row: usize, column: usize, expected: &str) {
    let view = result.typed_view().expect("query result should be valid");
    match view.cell(row, column).expect("cell should exist") {
        QueryCellRef::Text(value) => assert_eq!(value, expected),
        QueryCellRef::Null => panic!("expected text at ({row},{column}), got NULL"),
        QueryCellRef::Binary(bytes) => panic!("expected text at ({row},{column}), got {bytes:?}"),
    }
}

pub fn assert_null(result: &QueryResult, row: usize, column: usize) {
    let view = result.typed_view().expect("query result should be valid");
    assert!(matches!(view.cell(row, column), Some(QueryCellRef::Null)));
}

pub fn assert_binary(result: &QueryResult, row: usize, column: usize, expected: &[u8]) {
    let view = result.typed_view().expect("query result should be valid");
    match view.cell(row, column).expect("cell should exist") {
        QueryCellRef::Binary(bytes) => assert_eq!(bytes, expected),
        other => panic!("expected binary at ({row},{column}), got {other:?}"),
    }
}
