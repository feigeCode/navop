use super::*;
use crate::connection::{DbError, StreamingProgress};
use crate::executor::{BinaryCell, ExecOptions, QueryColumnMeta, SqlSource};
use crate::mssql::MsSqlPlugin;
use crate::mysql::MySqlPlugin;
use crate::oracle::OraclePlugin;
use crate::postgresql::PostgresPlugin;
use crate::sqlite::{SqliteDbConnection, SqlitePlugin};
use async_trait::async_trait;
use one_core::storage::{DatabaseType, DbConnectionConfig};
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

struct PagedConnection {
    config: DbConnectionConfig,
    queries: Arc<Mutex<Vec<String>>>,
    pages: Arc<Mutex<Vec<Vec<Vec<Option<String>>>>>>,
}

impl PagedConnection {
    fn new(pages: Vec<Vec<Vec<Option<String>>>>) -> Self {
        Self {
            config: test_config(),
            queries: Arc::new(Mutex::new(Vec::new())),
            pages: Arc::new(Mutex::new(pages)),
        }
    }

    fn queries(&self) -> Vec<String> {
        self.queries.lock().unwrap().clone()
    }
}

#[async_trait]
impl DbConnection for PagedConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn disconnect(&mut self) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn execute(
        &self,
        _plugin: &dyn DatabasePlugin,
        _script: &str,
        _options: ExecOptions,
    ) -> std::result::Result<Vec<SqlResult>, DbError> {
        Ok(Vec::new())
    }

    async fn query(&self, query: &str) -> std::result::Result<SqlResult, DbError> {
        self.queries.lock().unwrap().push(query.to_string());
        let rows = self.pages.lock().unwrap().remove(0);
        Ok(SqlResult::Query(QueryResult {
            sql: query.to_string(),
            columns: vec!["id".to_string(), "name".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("id", "BIGINT"),
                QueryColumnMeta::new("name", "VARCHAR"),
            ],
            rows,
            binary_cells: vec![],
            elapsed_ms: 1,
        }))
    }

    async fn current_database(&self) -> std::result::Result<Option<String>, DbError> {
        Ok(Some("app".to_string()))
    }

    async fn switch_database(&self, _database: &str) -> std::result::Result<(), DbError> {
        Ok(())
    }

    async fn execute_streaming(
        &self,
        _plugin: &dyn DatabasePlugin,
        _source: SqlSource,
        _options: ExecOptions,
        _sender: mpsc::Sender<StreamingProgress>,
    ) -> std::result::Result<(), DbError> {
        Ok(())
    }
}

fn row(id: usize) -> Vec<Option<String>> {
    vec![Some(id.to_string()), Some(format!("user'{id}"))]
}

fn test_config() -> DbConnectionConfig {
    DbConnectionConfig {
        id: "test".to_string(),
        database_type: DatabaseType::MySQL,
        name: "mysql".to_string(),
        host: "localhost".to_string(),
        port: 3306,
        username: "root".to_string(),
        password: String::new(),
        database: Some("app".to_string()),
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        credential_reference: None,
        extra_params: Default::default(),
    }
}

#[tokio::test]
async fn sql_export_streams_table_data_in_pages() {
    let first_page = (0..SQL_EXPORT_PAGE_SIZE).map(row).collect::<Vec<_>>();
    let second_page = vec![row(SQL_EXPORT_PAGE_SIZE)];
    let connection = PagedConnection::new(vec![first_page, second_page]);
    let plugin = MySqlPlugin::new();
    let config = ExportConfig {
        database: "app".to_string(),
        tables: vec!["users".to_string()],
        ..ExportConfig::default()
    };
    let mut output = String::new();
    let events = Mutex::new(Vec::new());

    let rows = export_table_data_in_pages(
        &plugin,
        &connection,
        &config,
        "users",
        true,
        &mut output,
        &|event| events.lock().unwrap().push(event),
    )
    .await
    .expect("paged export should succeed");

    assert_eq!(1001, rows);
    assert!(output.is_empty());
    assert_eq!(
        vec![
            "SELECT * FROM `app`.`users` LIMIT 1000 OFFSET 0",
            "SELECT * FROM `app`.`users` LIMIT 1000 OFFSET 1000",
        ],
        connection.queries()
    );
    let events = events.lock().unwrap();
    assert_eq!(2, events.len());
    assert!(matches!(
        &events[0],
        ExportProgressEvent::DataExported { rows: 1000, data, .. }
            if data.contains("-- Data for table users") && data.contains("'user''0'")
    ));
    assert!(matches!(
        &events[1],
        ExportProgressEvent::DataExported { rows: 1, data, .. }
            if !data.contains("-- Data for table users") && data.contains("'user''1000'")
    ));
}

#[test]
fn sql_dump_prefers_binary_sidecar_without_guessing_from_display_text() {
    let query_result = QueryResult {
        sql: "SELECT payload, marker FROM binary_data".to_string(),
        columns: vec!["payload".to_string(), "marker".to_string()],
        column_meta: vec![
            QueryColumnMeta::new("payload", "BLOB"),
            QueryColumnMeta::new("marker", "TEXT"),
        ],
        rows: vec![vec![
            Some("0x0001ff".to_string()),
            Some("0x0001ff".to_string()),
        ]],
        binary_cells: vec![BinaryCell {
            row_index: 0,
            column_index: 0,
            bytes: vec![0x00, 0x01, 0xff],
        }],
        elapsed_ms: 1,
    };
    let mut wrote_header = false;

    let output = sql_dump_page(
        &MySqlPlugin::new(),
        "`binary_data`",
        "binary_data",
        &query_result,
        &mut wrote_header,
    );

    assert!(output.contains("VALUES (X'0001ff', '0x0001ff');"));
}

#[test]
fn binary_literals_follow_database_dialects() {
    let bytes = [0x00, 0x01, 0xff];

    assert_eq!(
        "X'0001ff'",
        MySqlPlugin::new().format_binary_literal(&bytes)
    );
    assert_eq!(
        "X'0001ff'",
        SqlitePlugin::new().format_binary_literal(&bytes)
    );
    assert_eq!(
        "decode('0001ff', 'hex')",
        PostgresPlugin::new().format_binary_literal(&bytes)
    );
    assert_eq!("0x0001ff", MsSqlPlugin::new().format_binary_literal(&bytes));
    assert_eq!(
        "HEXTORAW('0001ff')",
        OraclePlugin::new().format_binary_literal(&bytes)
    );
    assert_eq!(
        "from_hex('0001ff')",
        crate::plugin::format_binary_literal_for_database(&DatabaseType::DuckDB, &bytes)
    );
    assert_eq!(
        "unhex('0001ff')",
        crate::plugin::format_binary_literal_for_database(&DatabaseType::ClickHouse, &bytes)
    );
    assert_eq!("X''", SqlitePlugin::new().format_binary_literal(&[]));
}

fn sqlite_config(id: &str, path: &std::path::Path) -> DbConnectionConfig {
    DbConnectionConfig {
        id: id.to_string(),
        database_type: DatabaseType::SQLite,
        name: id.to_string(),
        host: path.to_string_lossy().to_string(),
        port: 0,
        username: String::new(),
        password: String::new(),
        database: None,
        service_name: None,
        sid: None,
        workspace_id: None,
        proxy: None,
        credential_reference: None,
        extra_params: Default::default(),
    }
}

fn assert_execute_succeeded(results: &[SqlResult]) {
    if let Some(SqlResult::Error(error)) = results.iter().find(|result| result.is_error()) {
        panic!("SQL execution failed: {}", error.message);
    }
}

#[tokio::test]
async fn sqlite_sql_export_round_trips_binary_bytes_and_preserves_hex_like_text() {
    let temp_dir = tempfile::tempdir().expect("temp dir should be created");
    let source_path = temp_dir.path().join("source.db");
    let target_path = temp_dir.path().join("target.db");
    let plugin = SqlitePlugin::new();
    let mut source = SqliteDbConnection::new(sqlite_config("source", &source_path));
    source
        .connect()
        .await
        .expect("source SQLite should connect");

    let source_results = source
        .execute(
            &plugin,
            "CREATE TABLE binary_data (
                id INTEGER PRIMARY KEY,
                payload BLOB,
                marker TEXT
            );
            INSERT INTO binary_data (id, payload, marker)
            VALUES (1, X'0001ff', '0x0001ff');",
            ExecOptions::default(),
        )
        .await
        .expect("source fixture should execute");
    assert_execute_succeeded(&source_results);

    let config = ExportConfig {
        database: "main".to_string(),
        tables: vec!["binary_data".to_string()],
        ..ExportConfig::default()
    };
    let mut dump = String::new();
    let rows = export_table_data_in_pages(
        &plugin,
        &source,
        &config,
        "binary_data",
        false,
        &mut dump,
        &|_| {},
    )
    .await
    .expect("SQL export should succeed");
    assert_eq!(1, rows);
    assert!(dump.contains("X'0001ff'"));
    assert!(dump.contains("'0x0001ff'"));

    let mut target = SqliteDbConnection::new(sqlite_config("target", &target_path));
    target
        .connect()
        .await
        .expect("target SQLite should connect");
    let target_schema = target
        .execute(
            &plugin,
            "CREATE TABLE binary_data (
                id INTEGER PRIMARY KEY,
                payload BLOB,
                marker TEXT
            );",
            ExecOptions::default(),
        )
        .await
        .expect("target schema should execute");
    assert_execute_succeeded(&target_schema);
    let restore_results = target
        .execute(&plugin, &dump, ExecOptions::default())
        .await
        .expect("SQL dump should execute");
    assert_execute_succeeded(&restore_results);

    let result = target
        .query(
            "SELECT hex(payload), typeof(payload), marker, typeof(marker)
             FROM binary_data",
        )
        .await
        .expect("restored row should be queryable");
    let SqlResult::Query(result) = result else {
        panic!("expected restored query result");
    };
    assert_eq!(
        vec![
            Some("0001FF".to_string()),
            Some("blob".to_string()),
            Some("0x0001ff".to_string()),
            Some("text".to_string()),
        ],
        result.rows[0]
    );

    source
        .disconnect()
        .await
        .expect("source SQLite should disconnect");
    target
        .disconnect()
        .await
        .expect("target SQLite should disconnect");
}
