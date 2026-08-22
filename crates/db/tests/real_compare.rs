use std::collections::{HashMap, HashSet};

use db::compare::{
    DataCompareOptions, DiffStatus, SchemaCompareOptions, SyncStatementKind,
    build_data_sync_plan_with_plugin, compare_data_rows, compare_schemas, rows_from_query_result,
    table_schema_from_columns,
};
use db::connection::DbConnection;
use db::executor::{ExecOptions, SqlResult};
use db::plugin::DatabasePlugin;
use db::sqlite::{SqliteDbConnection, SqlitePlugin};
use one_core::storage::{DatabaseType, DbConnectionConfig};

const SOURCE_SQL: &str = r#"
DROP TABLE IF EXISTS people;
CREATE TABLE people (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    score REAL,
    payload BLOB,
    height NUMERIC
);
CREATE UNIQUE INDEX idx_people_name ON people(name);
INSERT INTO people VALUES
    (1, 'Alice', 91.5, X'000102FF', 170.20),
    (2, '中文 🚀', NULL, X'', NULL),
    (3, 'O''Reilly', 72.0, X'FF', -3.25);
"#;

const TARGET_SQL: &str = r#"
DROP TABLE IF EXISTS people;
CREATE TABLE people (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    score REAL,
    payload BLOB,
    height NUMERIC,
    active BOOLEAN DEFAULT 0
);
CREATE UNIQUE INDEX idx_people_name ON people(name);
INSERT INTO people VALUES
    (1, 'Alice', 91.5, X'000102FF', 170.20, 1),
    (2, '中文 🚀', 88.0, X'', NULL, 0),
    (4, 'Deleted', NULL, NULL, NULL, 0);
DROP TABLE IF EXISTS obsolete;
CREATE TABLE obsolete (id INTEGER PRIMARY KEY);
"#;

fn config(id: &str, path: &std::path::Path) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.to_string(),
        database_type: DatabaseType::SQLite,
        name: id.to_string(),
        host: path.to_string_lossy().to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        credential_reference: None,
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        extra_params: HashMap::new(),
    }
}

async fn execute(connection: &SqliteDbConnection, sql: &str) {
    let results = connection
        .execute(&SqlitePlugin::new(), sql, ExecOptions::default())
        .await
        .expect("SQLite compare setup should execute");
    assert!(
        !results
            .iter()
            .any(|result| matches!(result, SqlResult::Error(_))),
        "SQLite compare setup should not report SQL errors"
    );
}

async fn query_rows(connection: &SqliteDbConnection, sql: &str) -> Vec<db::compare::RowData> {
    let result = connection.query(sql).await.expect("compare query");
    let SqlResult::Query(result) = result else {
        panic!("compare query should return rows: {sql}")
    };
    rows_from_query_result(&result).expect("query result should convert to compare rows")
}

#[tokio::test]
async fn sqlite_real_schema_and_data_compare_then_selected_sync_round_trip() {
    let source_dir = tempfile::tempdir().expect("source temp dir");
    let target_dir = tempfile::tempdir().expect("target temp dir");
    let plugin = SqlitePlugin::new();
    let mut source = SqliteDbConnection::new(config(
        "compare-source",
        &source_dir.path().join("source.db"),
    ));
    let mut target = SqliteDbConnection::new(config(
        "compare-target",
        &target_dir.path().join("target.db"),
    ));
    source.connect().await.expect("source should connect");
    target.connect().await.expect("target should connect");
    execute(&source, SOURCE_SQL).await;
    execute(&target, TARGET_SQL).await;

    let source_columns = plugin
        .list_columns(&source, "main", None, "people")
        .await
        .expect("source columns");
    let target_columns = plugin
        .list_columns(&target, "main", None, "people")
        .await
        .expect("target columns");
    let source_schema = table_schema_from_columns("people", &source_columns);
    let target_schema = table_schema_from_columns("people", &target_columns);
    assert_eq!(source_schema.columns[0].name, "id");
    assert!(source_schema.columns[0].nullable);
    assert!(source_schema.indexes.iter().any(|index| {
        index.name == "PRIMARY" && index.columns == vec!["id".to_string()] && index.unique
    }));

    let schema_diff = compare_schemas(
        vec![source_schema.clone()],
        vec![target_schema],
        SchemaCompareOptions::default(),
    )
    .expect("schema comparison should succeed");
    assert_eq!(schema_diff.table_diffs.len(), 1);
    assert_eq!(schema_diff.table_diffs[0].status, DiffStatus::Modified);
    assert_eq!(schema_diff.table_diffs[0].column_diffs.len(), 1);
    assert_eq!(schema_diff.table_diffs[0].column_diffs[0].name, "active");
    assert_eq!(
        schema_diff.table_diffs[0].column_diffs[0].status,
        DiffStatus::Removed
    );

    let columns = source_schema
        .columns
        .iter()
        .map(|column| column.name.clone())
        .collect::<Vec<_>>();
    let source_rows = query_rows(
        &source,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let target_rows = query_rows(
        &target,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let mut data_diff = compare_data_rows(
        source_rows,
        target_rows,
        DataCompareOptions {
            source_table: "people".to_string(),
            target_table: "people".to_string(),
            key_columns: vec!["id".to_string()],
            columns: columns.clone(),
        },
    )
    .expect("data comparison should succeed");
    data_diff.column_types = source_columns
        .iter()
        .map(|column| (column.name.clone(), column.data_type.clone()))
        .collect();
    assert_eq!(data_diff.added.len(), 1);
    assert_eq!(
        data_diff.added[0]
            .get("id")
            .and_then(|value| value.as_i64()),
        Some(3)
    );
    assert_eq!(data_diff.removed.len(), 1);
    assert_eq!(
        data_diff.removed[0]
            .get("id")
            .and_then(|value| value.as_i64()),
        Some(4)
    );
    assert_eq!(data_diff.modified.len(), 1);
    assert_eq!(
        data_diff.modified[0]
            .key_values
            .get("id")
            .and_then(|value| value.as_i64()),
        Some(2)
    );
    assert_eq!(data_diff.modified[0].changes.len(), 1);
    assert!(data_diff.modified[0].changes.contains_key("score"));

    let sync_plan = build_data_sync_plan_with_plugin(&data_diff, "main", None, &plugin);
    assert_eq!(sync_plan.summary.insert_count, 1);
    assert_eq!(sync_plan.summary.update_count, 1);
    assert_eq!(sync_plan.summary.delete_count, 1);
    assert_eq!(sync_plan.summary.total_count, 3);
    let insert_sql = sync_plan
        .statements
        .iter()
        .map(|statement| statement.sql.as_str())
        .find(|sql| sql.starts_with("INSERT INTO"))
        .expect("data sync plan should contain INSERT");
    assert!(insert_sql.contains("72"));
    assert!(insert_sql.contains("X'ff'"));
    assert!(sync_plan.sql_text.contains("UPDATE"));
    assert!(sync_plan.sql_text.contains("DELETE"));

    let selected_ids = sync_plan
        .statements
        .iter()
        .filter(|statement| statement.selected_by_default)
        .map(|statement| statement.id.clone())
        .collect::<HashSet<_>>();
    let selected_sql = sync_plan
        .statements
        .iter()
        .filter(|statement| selected_ids.contains(&statement.id))
        .map(|statement| statement.sql.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert_eq!(selected_ids.len(), 2);
    assert!(!selected_sql.contains("DELETE"));

    for statement in &sync_plan.statements {
        let is_delete = matches!(statement.kind, SyncStatementKind::Delete);
        if is_delete {
            assert!(statement.destructive);
            assert!(!statement.selected_by_default);
            assert!(statement.sql.contains("WHERE"));
        } else {
            assert!(statement.selected_by_default);
            assert!(!statement.destructive);
        }
    }

    execute(&target, &selected_sql).await;
    let after_target_rows = query_rows(
        &target,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let after_source_rows = query_rows(
        &source,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let after_diff = compare_data_rows(
        after_source_rows,
        after_target_rows,
        DataCompareOptions {
            source_table: "people".to_string(),
            target_table: "people".to_string(),
            key_columns: vec!["id".to_string()],
            columns: columns.clone(),
        },
    )
    .expect("post-sync comparison should succeed");
    assert!(after_diff.added.is_empty());
    assert!(after_diff.modified.is_empty());
    assert_eq!(after_diff.removed.len(), 1);

    let delete_sql = sync_plan
        .statements
        .iter()
        .find(|statement| matches!(statement.kind, SyncStatementKind::Delete))
        .map(|statement| statement.sql.clone())
        .expect("plan should retain the destructive delete statement");
    execute(&target, &delete_sql).await;
    let final_target_rows = query_rows(
        &target,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let final_source_rows = query_rows(
        &source,
        "SELECT id, name, score, payload, height FROM people ORDER BY id",
    )
    .await;
    let final_diff = compare_data_rows(
        final_source_rows,
        final_target_rows,
        DataCompareOptions {
            source_table: "people".to_string(),
            target_table: "people".to_string(),
            key_columns: vec!["id".to_string()],
            columns,
        },
    )
    .expect("final comparison should succeed");
    assert!(final_diff.added.is_empty());
    assert!(final_diff.removed.is_empty());
    assert!(final_diff.modified.is_empty());

    source.disconnect().await.expect("source disconnect");
    target.disconnect().await.expect("target disconnect");
}
