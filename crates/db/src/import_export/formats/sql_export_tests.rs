use super::*;
use crate::connection::{DbError, StreamingProgress};
use crate::executor::{BinaryCell, ExecOptions, QueryColumnMeta, SqlSource};
use crate::import_export::FormatHandler;
use crate::import_export::formats::SqlFormatHandler;
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

struct MySqlBinaryTextConnection {
    config: DbConnectionConfig,
    queries: Arc<Mutex<Vec<String>>>,
    include_binary_sidecars: bool,
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

impl MySqlBinaryTextConnection {
    fn new(include_binary_sidecars: bool) -> Self {
        Self {
            config: test_config(),
            queries: Arc::new(Mutex::new(Vec::new())),
            include_binary_sidecars,
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
        if query.contains("INFORMATION_SCHEMA.COLUMNS") {
            let schema_columns = [
                ("id", "BIGINT", "NO", "PRI", None, None),
                ("name", "VARCHAR", "YES", "", None, Some("utf8mb4")),
            ];
            let rows = schema_columns
                .into_iter()
                .map(|(name, data_type, nullable, key, default, charset)| {
                    vec![
                        Some(name.to_string()),
                        Some(data_type.to_string()),
                        Some(nullable.to_string()),
                        Some(key.to_string()),
                        default.map(str::to_string),
                        Some(String::new()),
                        charset.map(str::to_string),
                        charset.map(|charset| format!("{charset}_general_ci")),
                    ]
                })
                .collect();
            return Ok(SqlResult::Query(QueryResult {
                sql: query.to_string(),
                columns: [
                    "COLUMN_NAME",
                    "COLUMN_TYPE",
                    "IS_NULLABLE",
                    "COLUMN_KEY",
                    "COLUMN_DEFAULT",
                    "COLUMN_COMMENT",
                    "CHARACTER_SET_NAME",
                    "COLLATION_NAME",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
                column_meta: [
                    "COLUMN_NAME",
                    "COLUMN_TYPE",
                    "IS_NULLABLE",
                    "COLUMN_KEY",
                    "COLUMN_DEFAULT",
                    "COLUMN_COMMENT",
                    "CHARACTER_SET_NAME",
                    "COLLATION_NAME",
                ]
                .into_iter()
                .map(|name| QueryColumnMeta::new(name, "VARCHAR"))
                .collect(),
                rows,
                binary_cells: vec![],
                elapsed_ms: 1,
            }));
        }
        let mut rows = self.pages.lock().unwrap().remove(0);
        let mut columns = vec!["id".to_string(), "name".to_string()];
        let mut column_meta = vec![
            QueryColumnMeta::new("id", "BIGINT"),
            QueryColumnMeta::new("name", "VARCHAR"),
        ];
        if query.contains("__navop_pagination_rownum__") {
            columns.push("__navop_pagination_rownum__".to_string());
            column_meta.push(QueryColumnMeta::new(
                "__navop_pagination_rownum__",
                "NUMBER",
            ));
            for (index, row) in rows.iter_mut().enumerate() {
                row.push(Some((index + 1).to_string()));
            }
        }
        Ok(SqlResult::Query(QueryResult {
            sql: query.to_string(),
            columns,
            column_meta,
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

#[async_trait]
impl DbConnection for MySqlBinaryTextConnection {
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

        if query.contains("INFORMATION_SCHEMA.COLUMNS") {
            return Ok(SqlResult::Query(QueryResult {
                sql: query.to_string(),
                columns: [
                    "COLUMN_NAME",
                    "COLUMN_TYPE",
                    "IS_NULLABLE",
                    "COLUMN_KEY",
                    "COLUMN_DEFAULT",
                    "COLUMN_COMMENT",
                    "CHARACTER_SET_NAME",
                    "COLLATION_NAME",
                ]
                .into_iter()
                .map(str::to_string)
                .collect(),
                column_meta: [
                    "COLUMN_NAME",
                    "COLUMN_TYPE",
                    "IS_NULLABLE",
                    "COLUMN_KEY",
                    "COLUMN_DEFAULT",
                    "COLUMN_COMMENT",
                    "CHARACTER_SET_NAME",
                    "COLLATION_NAME",
                ]
                .into_iter()
                .map(|name| QueryColumnMeta::new(name, "VARCHAR"))
                .collect(),
                rows: vec![
                    vec![
                        Some("id".to_string()),
                        Some("BIGINT".to_string()),
                        Some("NO".to_string()),
                        Some("PRI".to_string()),
                        None,
                        Some(String::new()),
                        None,
                        None,
                    ],
                    vec![
                        Some("payload".to_string()),
                        Some("LONGTEXT".to_string()),
                        Some("YES".to_string()),
                        Some(String::new()),
                        None,
                        Some(String::new()),
                        Some("utf8mb3".to_string()),
                        Some("utf8mb3_bin".to_string()),
                    ],
                    vec![
                        Some("raw".to_string()),
                        Some("LONGBLOB".to_string()),
                        Some("YES".to_string()),
                        Some(String::new()),
                        None,
                        Some(String::new()),
                        None,
                        Some("binary".to_string()),
                    ],
                ],
                binary_cells: vec![],
                elapsed_ms: 1,
            }));
        }

        let (payload, raw, binary_cells) = if self.include_binary_sidecars {
            (
                "0x74727565".to_string(),
                "0x000102ff".to_string(),
                vec![
                    BinaryCell {
                        row_index: 0,
                        column_index: 1,
                        bytes: b"true".to_vec(),
                    },
                    BinaryCell {
                        row_index: 0,
                        column_index: 2,
                        bytes: vec![0x00, 0x01, 0x02, 0xff],
                    },
                ],
            )
        } else {
            (
                r#"{"metric":"sales"}"#.to_string(),
                "0x000102ff".to_string(),
                vec![],
            )
        };

        Ok(SqlResult::Query(QueryResult {
            sql: query.to_string(),
            columns: vec!["id".to_string(), "payload".to_string(), "raw".to_string()],
            column_meta: vec![
                QueryColumnMeta::new("id", "MYSQL_TYPE_LONGLONG"),
                QueryColumnMeta::new(
                    "payload",
                    if self.include_binary_sidecars {
                        "MYSQL_TYPE_LONG_BLOB"
                    } else {
                        "MYSQL_TYPE_BLOB"
                    },
                )
                .with_result_encoding(
                    Some(if self.include_binary_sidecars {
                        "binary"
                    } else {
                        "utf8mb4"
                    }),
                    Some(if self.include_binary_sidecars {
                        "binary"
                    } else {
                        "utf8mb4_general_ci"
                    }),
                    Some(if self.include_binary_sidecars { 63 } else { 45 }),
                ),
                QueryColumnMeta::new("raw", "MYSQL_TYPE_LONG_BLOB").with_result_encoding(
                    Some("binary"),
                    Some("binary"),
                    Some(63),
                ),
            ],
            rows: vec![vec![Some("1".to_string()), Some(payload), Some(raw)]],
            binary_cells,
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
async fn sql_export_output_starts_with_database_header() {
    let connection = PagedConnection::new(Vec::new());
    let config = ExportConfig {
        database: "comi_ai_manager".to_string(),
        tables: Vec::new(),
        ..ExportConfig::default()
    };

    let result = SqlFormatHandler
        .export(&MySqlPlugin::new(), &connection, &config)
        .await
        .expect("SQL export should succeed");

    let lines = result.output.lines().collect::<Vec<_>>();
    assert_eq!(Some(&"-- Database export: comi_ai_manager"), lines.first());
    assert!(
        lines
            .get(1)
            .is_some_and(|line| line.starts_with("-- Date: "))
    );
    assert_eq!(Some(&"-- Generated by Navop"), lines.get(2));
    assert_eq!(Some(&""), lines.get(3));
    chrono::NaiveDateTime::parse_from_str(
        lines[1].trim_start_matches("-- Date: "),
        "%Y-%m-%d %H:%M:%S",
    )
    .expect("export date should use YYYY-MM-DD HH:MM:SS");
}

#[tokio::test]
async fn streaming_sql_export_sends_header_before_table_events() {
    let connection = PagedConnection::new(Vec::new());
    let config = ExportConfig {
        database: "comi_ai_manager".to_string(),
        tables: Vec::new(),
        ..ExportConfig::default()
    };
    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();

    let result = SqlFormatHandler
        .export_with_progress(&MySqlPlugin::new(), &connection, &config, Some(progress_tx))
        .await
        .expect("streaming SQL export should succeed");

    assert!(result.output.is_empty());
    let first_event = progress_rx
        .recv()
        .await
        .expect("streaming export should emit a header");
    assert!(matches!(
        first_event,
        ExportProgressEvent::HeaderExported { data }
            if data.starts_with("-- Database export: comi_ai_manager\n-- Date: ")
                && data.ends_with("\n-- Generated by Navop\n\n")
    ));
    assert!(matches!(
        progress_rx.recv().await,
        Some(ExportProgressEvent::Finished { total_rows: 0, .. })
    ));
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
        connection
            .queries()
            .into_iter()
            .filter(|query| !query.contains("INFORMATION_SCHEMA.COLUMNS"))
            .collect::<Vec<_>>()
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

#[tokio::test]
async fn oracle_sql_export_uses_11g_pagination_without_exporting_internal_rownum() {
    let first_page = (0..SQL_EXPORT_PAGE_SIZE).map(row).collect::<Vec<_>>();
    let second_page = vec![row(SQL_EXPORT_PAGE_SIZE)];
    let connection = PagedConnection::new(vec![first_page, second_page]);
    let plugin = OraclePlugin::new();
    let config = ExportConfig {
        database: "APP".to_string(),
        schema: Some("APP".to_string()),
        tables: vec!["USERS".to_string()],
        ..ExportConfig::default()
    };
    let mut output = String::new();

    let rows = export_table_data_in_pages(
        &plugin,
        &connection,
        &config,
        "USERS",
        false,
        &mut output,
        &|_| {},
    )
    .await
    .expect("Oracle paged export should succeed");

    assert_eq!(1001, rows);
    let queries = connection.queries();
    assert_eq!(2, queries.len());
    assert_eq!(
        "SELECT * FROM (SELECT * FROM \"APP\".\"USERS\") WHERE ROWNUM <= 1000",
        queries[0]
    );
    assert!(!queries[1].contains(" LIMIT "));
    assert!(!queries[1].contains(" OFFSET "));
    assert!(!queries[1].contains("FETCH NEXT"));
    assert!(queries[1].contains("WHERE ROWNUM <= 2000"));
    assert!(queries[1].contains("\"__navop_pagination_rownum__\" > 1000"));
    assert!(!output.contains("__navop_pagination_rownum__"));
    assert!(output.contains("INSERT INTO \"APP\".\"USERS\" (\"id\", \"name\")"));
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
    )
    .expect("valid query result should render");

    assert!(output.contains("VALUES (X'0001ff', '0x0001ff');"));
}

#[tokio::test]
async fn mysql_sql_export_normalizes_longtext_and_preserves_longblob_binary_literals() {
    let connection = MySqlBinaryTextConnection::new(true);
    let config = ExportConfig {
        database: "app".to_string(),
        tables: vec!["binary_text".to_string()],
        ..ExportConfig::default()
    };
    let mut output = String::new();

    let rows = export_table_data_in_pages(
        &MySqlPlugin::new(),
        &connection,
        &config,
        "binary_text",
        false,
        &mut output,
        &|_| {},
    )
    .await
    .expect("MySQL SQL export should reconcile text and binary schema semantics");

    assert_eq!(rows, 1);
    assert!(output.contains("VALUES (1, 'true', X'000102ff');"));
    assert!(!output.contains("X'74727565'"));
    assert_eq!(
        connection.queries(),
        vec![
            "SELECT * FROM `app`.`binary_text` LIMIT 1000 OFFSET 0",
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT, CHARACTER_SET_NAME, COLLATION_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = 'app' AND TABLE_NAME = 'binary_text' ORDER BY ORDINAL_POSITION",
        ]
    );
}

#[tokio::test]
async fn mysql_sql_export_reconciles_longtext_metadata_without_binary_sidecars() {
    let connection = MySqlBinaryTextConnection::new(false);
    let config = ExportConfig {
        database: "app".to_string(),
        tables: vec!["binary_text".to_string()],
        ..ExportConfig::default()
    };
    let mut output = String::new();

    let rows = export_table_data_in_pages(
        &MySqlPlugin::new(),
        &connection,
        &config,
        "binary_text",
        false,
        &mut output,
        &|_| {},
    )
    .await
    .expect("MySQL SQL export should reconcile wire BLOB metadata with LONGTEXT schema");

    assert_eq!(rows, 1);
    assert!(output.contains(r#"VALUES (1, '{"metric":"sales"}', X'000102ff');"#));
    assert!(!output.contains("X'7b226d6574726963223a2273616c6573227d'"));
    assert_eq!(
        connection.queries(),
        vec![
            "SELECT * FROM `app`.`binary_text` LIMIT 1000 OFFSET 0",
            "SELECT COLUMN_NAME, COLUMN_TYPE, IS_NULLABLE, COLUMN_KEY, COLUMN_DEFAULT, COLUMN_COMMENT, CHARACTER_SET_NAME, COLLATION_NAME FROM INFORMATION_SCHEMA.COLUMNS WHERE TABLE_SCHEMA = 'app' AND TABLE_NAME = 'binary_text' ORDER BY ORDINAL_POSITION",
        ]
    );
}

#[test]
fn sql_dump_preserves_null_empty_text_and_empty_binary() {
    let query_result = QueryResult {
        sql: "SELECT nullable, empty_text, empty_binary FROM t".to_string(),
        columns: vec![
            "nullable".to_string(),
            "empty_text".to_string(),
            "empty_binary".to_string(),
        ],
        column_meta: vec![
            QueryColumnMeta::new("nullable", "TEXT"),
            QueryColumnMeta::new("empty_text", "TEXT"),
            QueryColumnMeta::new("empty_binary", "BLOB"),
        ],
        rows: vec![vec![None, Some(String::new()), None]],
        binary_cells: vec![BinaryCell {
            row_index: 0,
            column_index: 2,
            bytes: Vec::new(),
        }],
        elapsed_ms: 1,
    };

    let output = render_insert_statements(&MySqlPlugin::new(), "`t`", &query_result)
        .expect("valid query result should render");

    assert!(output.contains("VALUES (NULL, '', X'');"));
}

#[test]
fn sql_dump_rejects_malformed_query_result() {
    let query_result = QueryResult {
        sql: "SELECT a, b FROM t".to_string(),
        columns: vec!["a".to_string(), "b".to_string()],
        column_meta: vec![
            QueryColumnMeta::new("a", "TEXT"),
            QueryColumnMeta::new("b", "TEXT"),
        ],
        rows: vec![vec![Some("only one cell".to_string())]],
        binary_cells: vec![],
        elapsed_ms: 1,
    };

    let error = render_insert_statements(&MySqlPlugin::new(), "`t`", &query_result)
        .expect_err("short rows must fail instead of being treated as NULL");

    assert!(
        error
            .to_string()
            .contains("Invalid query result for SQL export")
    );
    assert!(error.to_string().contains("row 0 has width 1, expected 2"));
}

#[test]
fn mysql_sql_dump_formats_bit_values_as_unquoted_literals() {
    let query_result = QueryResult {
        sql: "SELECT id, bit_name FROM test_bit".to_string(),
        columns: vec!["id".to_string(), "bit_name".to_string()],
        column_meta: vec![
            QueryColumnMeta::new("id", "INT"),
            QueryColumnMeta::new("bit_name", "BIT(1)"),
        ],
        rows: vec![
            vec![Some("1".to_string()), Some("1".to_string())],
            vec![Some("2".to_string()), Some("0".to_string())],
        ],
        binary_cells: vec![],
        elapsed_ms: 1,
    };

    let output = render_insert_statements(&MySqlPlugin::new(), "`test_bit`", &query_result)
        .expect("valid query result should render");

    assert_eq!(
        concat!(
            "INSERT INTO `test_bit` (`id`, `bit_name`) VALUES (1, 1);\n",
            "INSERT INTO `test_bit` (`id`, `bit_name`) VALUES (2, 0);\n",
        ),
        output
    );
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
