use super::*;
use crate::storage::migration::run_migrations;
use std::sync::atomic::{AtomicU64, Ordering};

static DB_COUNTER: AtomicU64 = AtomicU64::new(0);

fn test_repository() -> SqlExecutionHistoryRepository {
    let unique = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let counter = DB_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!(
        "onetcli-sql-execution-history-{}-{unique}-{counter}.db",
        std::process::id(),
    ));
    let _ = std::fs::remove_file(&db_path);
    let conn = SqliteConnection::open_with_pool_size(&db_path, 1).expect("open sqlite");
    conn.with_connection(|conn| run_migrations(conn))
        .expect("run migrations");
    SqlExecutionHistoryRepository::new(conn)
}

fn entry(connection_id: &str, sql: &str) -> SqlExecutionHistoryEntry {
    SqlExecutionHistoryEntry {
        id: None,
        connection_id: connection_id.to_string(),
        database: Some("app".to_string()),
        schema: Some("public".to_string()),
        status: SqlExecutionStatus::Error,
        sql: sql.to_string(),
        summary: "permission denied".to_string(),
        details: vec!["first line".to_string(), "second line".to_string()],
        affected_rows: 3,
        returned_rows: Some(2),
        elapsed_ms: 15,
        executed_at: 123,
    }
}

#[test]
fn record_and_list_round_trip() {
    let repo = test_repository();
    let expected = entry("1", "update users set active = true");
    let id = repo.record(&expected).expect("record history");

    let records = repo.list(&["1".to_string()], 20).expect("list history");
    assert_eq!(1, records.len());
    assert_eq!(Some(id), records[0].id);
    assert_eq!(expected.connection_id, records[0].connection_id);
    assert_eq!(expected.database, records[0].database);
    assert_eq!(expected.schema, records[0].schema);
    assert_eq!(expected.status, records[0].status);
    assert_eq!(expected.sql, records[0].sql);
    assert_eq!(expected.summary, records[0].summary);
    assert_eq!(expected.details, records[0].details);
    assert_eq!(expected.affected_rows, records[0].affected_rows);
    assert_eq!(expected.returned_rows, records[0].returned_rows);
    assert_eq!(expected.elapsed_ms, records[0].elapsed_ms);
    assert_eq!(expected.executed_at, records[0].executed_at);
}

#[test]
fn list_filters_connections_and_returns_newest_first() {
    let repo = test_repository();
    let mut older = entry("a", "select 1");
    older.executed_at = 10;
    repo.record(&older).unwrap();
    let mut other = entry("b", "select 2");
    other.executed_at = 30;
    repo.record(&other).unwrap();
    let mut newer = entry("a", "select 3");
    newer.executed_at = 20;
    repo.record(&newer).unwrap();

    let records = repo.list(&["a".to_string()], 20).unwrap();
    assert_eq!(
        vec!["select 3".to_string(), "select 1".to_string()],
        records
            .into_iter()
            .map(|record| record.sql)
            .collect::<Vec<_>>()
    );
}

#[test]
fn clear_only_removes_requested_connections() {
    let repo = test_repository();
    repo.record(&entry("a", "select 1")).unwrap();
    repo.record(&entry("b", "select 2")).unwrap();

    repo.clear(&["a".to_string()]).unwrap();

    assert!(repo.list(&["a".to_string()], 20).unwrap().is_empty());
    assert_eq!(1, repo.list(&["b".to_string()], 20).unwrap().len());
}
