use super::*;

fn context() -> ExecutionContext {
    ExecutionContext {
        connection_id: "1".to_string(),
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
    }
}

#[test]
fn query_result_preserves_sql_rows_and_elapsed_time() {
    let result = SqlResult::Query(QueryResult {
        sql: "select 1".to_string(),
        columns: vec![],
        column_meta: vec![],
        rows: vec![vec![Some("1".to_string())], vec![Some("2".to_string())]],
        binary_cells: vec![],
        elapsed_ms: 12,
    });

    let record = ExecutionRecord::from_result(context(), &result);

    assert_eq!("select 1", record.sql);
    assert_eq!(Some(2), record.returned_rows);
    assert_eq!(12, record.elapsed_ms);
    assert_eq!(ExecutionStatus::Success, record.status);
}

#[test]
fn exec_result_preserves_affected_rows_and_message() {
    let result = SqlResult::Exec(ExecResult {
        sql: "update users set active = true".to_string(),
        rows_affected: 3,
        elapsed_ms: 8,
        message: Some("updated".to_string()),
    });

    let record = ExecutionRecord::from_result(context(), &result);

    assert_eq!(3, record.affected_rows);
    assert_eq!("updated", record.summary);
    assert_eq!(ExecutionStatus::Success, record.status);
}

#[test]
fn error_result_preserves_full_server_message() {
    let result = SqlResult::Error(SqlErrorInfo {
        sql: "delete from users".to_string(),
        message: "permission denied for table users".to_string(),
    });

    let record = ExecutionRecord::from_result(context(), &result);

    assert_eq!(ExecutionStatus::Error, record.status);
    assert_eq!(
        vec!["permission denied for table users".to_string()],
        record.details
    );
}

#[test]
fn history_drops_oldest_records_after_reaching_the_limit() {
    let result = SqlResult::Exec(ExecResult {
        sql: "select".to_string(),
        rows_affected: 0,
        elapsed_ms: 0,
        message: None,
    });
    let mut history = ExecutionHistory::default();

    for index in 0..=MAX_EXECUTION_RECORDS {
        history.record_result(
            ExecutionContext {
                connection_id: index.to_string(),
                ..context()
            },
            &result,
        );
    }

    assert_eq!(MAX_EXECUTION_RECORDS, history.records().len());
    assert_eq!(
        "1",
        history
            .records()
            .first()
            .expect("history should retain newest records")
            .context
            .connection_id
    );
}

#[test]
fn aggregated_results_preserve_all_errors_and_totals() {
    let results = vec![
        SqlResult::Query(QueryResult {
            sql: "select".to_string(),
            columns: vec![],
            column_meta: vec![],
            rows: vec![vec![Some("1".to_string())]],
            binary_cells: vec![],
            elapsed_ms: 6,
        }),
        SqlResult::Exec(ExecResult {
            sql: "insert".to_string(),
            rows_affected: 2,
            elapsed_ms: 4,
            message: Some("inserted".to_string()),
        }),
        SqlResult::Error(SqlErrorInfo {
            sql: "update".to_string(),
            message: "first error".to_string(),
        }),
        SqlResult::Error(SqlErrorInfo {
            sql: "delete".to_string(),
            message: "second error".to_string(),
        }),
    ];

    let record = ExecutionRecord::from_results(
        context(),
        &results,
        "batch".to_string(),
        "saved".to_string(),
        |error| format!("failed: {error}"),
    );

    assert_eq!(ExecutionStatus::Error, record.status);
    assert_eq!(2, record.affected_rows);
    assert_eq!(Some(1), record.returned_rows);
    assert_eq!(10, record.elapsed_ms);
    assert_eq!(
        vec![
            "first error".to_string(),
            "second error".to_string(),
            "inserted".to_string()
        ],
        record.details
    );
}
