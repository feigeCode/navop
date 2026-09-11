use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use duckdb::{Connection, types::ValueRef};
use tokio::sync::mpsc;
use tokio::task::spawn_blocking;
use tracing::{debug, error, info};

use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    BinaryCell, ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlErrorInfo, SqlResult,
    SqlSource,
};
use crate::types::FieldType;
use crate::{DatabasePlugin, format_message, truncate_str};
use db_value::{
    CellState, ColumnDescriptor, DbValue, FloatWidth, Nullability, RawPayload, RawRepresentation,
    ResultBatch, ResultRow,
};
use one_core::storage::DbConnectionConfig;

pub struct DuckDbConnection {
    config: DbConnectionConfig,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl DuckDbConnection {
    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// 把 DuckDB 运行时值解码为带类型值和对应 legacy 显示。
    ///
    /// 运行时类型是权威：`Blob` 永远是 [`DbValue::Binary`]（即使字节恰好是
    /// 合法 UTF-8），`Text` 非法 UTF-8 是显式 [`CellState::DecodeError`] 而不是
    /// NULL。尚未建模的 DuckDB 值进入 [`CellState::Undecoded`]，不把 `Debug`
    /// 文本当成原值。
    fn extract_value(value: ValueRef<'_>) -> (CellState, Option<String>) {
        match value {
            ValueRef::Null => (CellState::Decoded(DbValue::Null), None),
            ValueRef::Boolean(v) => (CellState::Decoded(DbValue::Bool(v)), Some(v.to_string())),
            ValueRef::TinyInt(v) => Self::integer_cell(v.to_string()),
            ValueRef::SmallInt(v) => Self::integer_cell(v.to_string()),
            ValueRef::Int(v) => Self::integer_cell(v.to_string()),
            ValueRef::BigInt(v) => Self::integer_cell(v.to_string()),
            ValueRef::HugeInt(v) => Self::integer_cell(v.to_string()),
            ValueRef::UHugeInt(v) => Self::unsigned_cell(v.to_string()),
            ValueRef::UTinyInt(v) => Self::unsigned_cell(v.to_string()),
            ValueRef::USmallInt(v) => Self::unsigned_cell(v.to_string()),
            ValueRef::UInt(v) => Self::unsigned_cell(v.to_string()),
            ValueRef::UBigInt(v) => Self::unsigned_cell(v.to_string()),
            ValueRef::Float(v) => (
                CellState::Decoded(DbValue::Float {
                    value: v.to_string(),
                    width: FloatWidth::F32,
                }),
                Some(v.to_string()),
            ),
            ValueRef::Double(v) => (
                CellState::Decoded(DbValue::Float {
                    value: v.to_string(),
                    width: FloatWidth::F64,
                }),
                Some(v.to_string()),
            ),
            ValueRef::Decimal(v) => Self::decimal_cell(v.to_string()),
            ValueRef::Text(t) => match String::from_utf8(t.to_vec()) {
                Ok(text) => (CellState::Decoded(DbValue::Text(text.clone())), Some(text)),
                Err(error) => {
                    let bytes = t.to_vec();
                    (
                        CellState::DecodeError {
                            native_type: "TEXT".to_string(),
                            raw: Some(RawPayload {
                                bytes: bytes.clone(),
                                representation: RawRepresentation::DatabaseValueBytes,
                            }),
                            diagnostic: format!("invalid UTF-8 text: {}", error.utf8_error()),
                        },
                        Some(format!("0x{}", hex::encode(bytes))),
                    )
                }
            },
            ValueRef::Blob(b) => {
                let bytes = b.to_vec();
                // typed 永远是 Binary；legacy 显示保持兼容：合法 UTF-8 展示原样文本,
                // 否则 hex,以便向后兼容既有消费者。
                let display = match String::from_utf8(bytes.clone()) {
                    Ok(text) => text,
                    Err(_) => format!("0x{}", hex::encode(&bytes)),
                };
                (CellState::Decoded(DbValue::Binary(bytes)), Some(display))
            }
            other => {
                let native_type = format!("{:?}", other.data_type());
                let display = format!("{other:?}");
                (
                    CellState::Undecoded {
                        native_type,
                        raw: None,
                        reason: "no typed decoder for this DuckDB value".to_string(),
                    },
                    Some(display),
                )
            }
        }
    }

    fn integer_cell(value: String) -> (CellState, Option<String>) {
        (
            CellState::Decoded(DbValue::Integer(value.clone())),
            Some(value),
        )
    }

    fn unsigned_cell(value: String) -> (CellState, Option<String>) {
        (
            CellState::Decoded(DbValue::Unsigned(value.clone())),
            Some(value),
        )
    }

    fn decimal_cell(value: String) -> (CellState, Option<String>) {
        (
            CellState::Decoded(DbValue::Decimal(value.clone())),
            Some(value),
        )
    }

    fn build_descriptors(
        columns: &[String],
        column_types: &[Option<String>],
    ) -> Vec<ColumnDescriptor> {
        columns
            .iter()
            .zip(column_types.iter())
            .enumerate()
            .map(|(index, (name, decl_type))| {
                let native_type = decl_type.clone().unwrap_or_else(|| "TEXT".to_string());
                ColumnDescriptor {
                    id: format!("column:{index}"),
                    label: name.clone(),
                    native_type: native_type.clone(),
                    logical_type: format!("{:?}", FieldType::from_db_type(&native_type)),
                    nullable: Nullability::Unknown,
                    charset: None,
                    collation: None,
                    precision: None,
                    scale: None,
                }
            })
            .collect()
    }

    fn collect_binary_cells(rows: &[ResultRow]) -> Vec<BinaryCell> {
        let mut binary_cells = Vec::new();
        for (row_index, row) in rows.iter().enumerate() {
            for (column_index, cell) in row.cells.iter().enumerate() {
                if let CellState::Decoded(DbValue::Binary(bytes)) = cell {
                    binary_cells.push(BinaryCell {
                        row_index,
                        column_index,
                        bytes: bytes.clone(),
                    });
                }
            }
        }
        binary_cells
    }

    fn build_query_result(
        columns: Vec<String>,
        column_types: Vec<Option<String>>,
        cells_rows: Vec<Vec<CellState>>,
        display_rows: Vec<Vec<Option<String>>>,
        sql: String,
        elapsed_ms: u128,
    ) -> SqlResult {
        let column_meta: Vec<QueryColumnMeta> = columns
            .iter()
            .zip(column_types.iter())
            .map(|(name, decl_type)| {
                QueryColumnMeta::new(
                    name.clone(),
                    decl_type.clone().unwrap_or_else(|| "TEXT".to_string()),
                )
            })
            .collect();
        let descriptors = Self::build_descriptors(&columns, &column_types);
        let batch_rows = cells_rows
            .into_iter()
            .enumerate()
            .map(|(row_index, cells)| ResultRow {
                id: row_index as u64,
                cells,
            })
            .collect::<Vec<_>>();
        let binary_cells = Self::collect_binary_cells(&batch_rows);
        let typed_batch = match ResultBatch::try_new(0, descriptors, batch_rows, true) {
            Ok(batch) => Arc::new(batch),
            Err(error) => {
                return SqlResult::Error(SqlErrorInfo {
                    sql,
                    message: format!("failed to build typed result batch: {error}"),
                });
            }
        };

        SqlResult::Query(QueryResult {
            column_meta,
            sql,
            columns,
            rows: display_rows,
            binary_cells,
            elapsed_ms,
            typed_batch: Some(typed_batch),
        })
    }

    fn build_exec_result(sql: String, rows_affected: u64, elapsed_ms: u128) -> SqlResult {
        let message = format_message(&sql, rows_affected);
        SqlResult::Exec(ExecResult {
            sql,
            rows_affected,
            elapsed_ms,
            message: Some(message),
        })
    }

    fn is_query_sql(sql: &str) -> bool {
        let normalized = sql.trim_start().to_ascii_uppercase();
        normalized.starts_with("SELECT")
            || normalized.starts_with("WITH")
            || normalized.starts_with("PRAGMA")
            || normalized.starts_with("SHOW")
            || normalized.starts_with("DESCRIBE")
            || normalized.starts_with("EXPLAIN")
    }

    fn execute_statement(conn: &Connection, sql: &str, start: Instant) -> SqlResult {
        let sql_preview = if sql.len() > 200 {
            format!("{}...", truncate_str(sql, 200))
        } else {
            sql.to_string()
        };

        match conn.prepare(sql) {
            Ok(mut stmt) => {
                if !Self::is_query_sql(sql) {
                    match conn.execute(sql, []) {
                        Ok(rows_affected) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[DuckDB] Execute completed: {} rows affected, {}ms",
                                rows_affected, elapsed_ms
                            );
                            Self::build_exec_result(
                                sql.to_string(),
                                rows_affected as u64,
                                elapsed_ms,
                            )
                        }
                        Err(e) => {
                            error!("[DuckDB] Execute failed: {}, SQL: {}", e, sql_preview);
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql.to_string(),
                                message: e.to_string(),
                            })
                        }
                    }
                } else {
                    let rows_result: Result<
                        (
                            Vec<String>,
                            Vec<Option<String>>,
                            Vec<Vec<CellState>>,
                            Vec<Vec<Option<String>>>,
                        ),
                        duckdb::Error,
                    > = stmt.query([]).and_then(|mut rows| {
                        let stmt_ref = rows
                            .as_ref()
                            .expect("DuckDB rows should retain statement metadata");
                        let column_count = stmt_ref.column_count();
                        let columns = stmt_ref.column_names();
                        let column_types: Vec<Option<String>> = (0..column_count)
                            .map(|idx| Some(format!("{:?}", stmt_ref.column_type(idx))))
                            .collect();
                        let mut cells_rows = Vec::new();
                        let mut display_rows = Vec::new();
                        while let Some(row) = rows.next()? {
                            let mut cells = Vec::with_capacity(column_count);
                            let mut display = Vec::with_capacity(column_count);
                            for i in 0..column_count {
                                let (state, cell_display) = Self::extract_value(row.get_ref(i)?);
                                cells.push(state);
                                display.push(cell_display);
                            }
                            cells_rows.push(cells);
                            display_rows.push(display);
                        }
                        Ok((columns, column_types, cells_rows, display_rows))
                    });

                    match rows_result {
                        Ok((columns, column_types, cells_rows, display_rows)) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[DuckDB] Query completed: {} rows, {} columns, {}ms",
                                display_rows.len(),
                                columns.len(),
                                elapsed_ms
                            );
                            Self::build_query_result(
                                columns,
                                column_types,
                                cells_rows,
                                display_rows,
                                sql.to_string(),
                                elapsed_ms,
                            )
                        }
                        Err(e) => {
                            error!("[DuckDB] Query failed: {}, SQL: {}", e, sql_preview);
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql.to_string(),
                                message: e.to_string(),
                            })
                        }
                    }
                }
            }
            Err(e) => {
                error!("[DuckDB] Prepare failed: {}, SQL: {}", e, sql_preview);
                SqlResult::Error(SqlErrorInfo {
                    sql: sql.to_string(),
                    message: e.to_string(),
                })
            }
        }
    }
}

#[async_trait]
impl DbConnection for DuckDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    fn supports_database_switch(&self) -> bool {
        false
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let config = self.config.clone();
        let database_path = if !config.host.is_empty() {
            config.host.clone()
        } else {
            config
                .database
                .clone()
                .ok_or_else(|| DbError::connection("database path is required for DuckDB"))?
        };

        info!("[DuckDB] Connecting to {}", database_path);

        let conn = spawn_blocking(move || Connection::open(database_path))
            .await
            .map_err(|e| {
                error!("[DuckDB] Task join error: {}", e);
                DbError::Internal(format!("task join error: {}", e))
            })?
            .map_err(|e| {
                error!("[DuckDB] Connection failed: {}", e);
                DbError::connection_with_source("failed to connect", e)
            })?;

        {
            let mut guard = self
                .connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            *guard = Some(conn);
        }

        info!("[DuckDB] Connected successfully");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        let conn_opt = {
            let mut guard = self
                .connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            guard.take()
        };

        if let Some(conn) = conn_opt {
            spawn_blocking(move || drop(conn)).await.map_err(|e| {
                error!("[DuckDB] Disconnect failed: {}", e);
                DbError::Internal(format!("task join error: {}", e))
            })?;
        }

        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        let parser = plugin
            .create_parser(SqlSource::Script(script.to_string()))
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;
        let statements: Vec<String> = parser
            .collect::<std::io::Result<Vec<_>>>()
            .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
            .into_iter()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if statements.is_empty() {
            return Ok(Vec::new());
        }

        let executable = statements
            .into_iter()
            .map(|sql| {
                let executable_sql = plugin.apply_query_max_rows(&sql, options.max_rows);
                (sql, executable_sql)
            })
            .collect::<Vec<_>>();
        let connection = Arc::clone(&self.connection);
        let stop_on_error = options.stop_on_error;
        let transactional = options.transactional;
        spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
            if transactional {
                conn.execute("BEGIN", []).map_err(|e| {
                    DbError::transaction_with_source("failed to begin transaction", e)
                })?;
            }

            let mut results = Vec::new();
            let mut has_error = false;
            for (sql, executable_sql) in executable {
                let result = Self::execute_statement(conn, &executable_sql, Instant::now())
                    .with_original_sql(&sql);
                let is_error = result.is_error();
                has_error |= is_error;
                results.push(result);
                if is_error && stop_on_error {
                    break;
                }
            }

            if transactional {
                let command = if has_error { "ROLLBACK" } else { "COMMIT" };
                conn.execute(command, []).map_err(|e| {
                    DbError::transaction_with_source(
                        format!("failed to {}", command.to_lowercase()),
                        e,
                    )
                })?;
            }
            Ok::<_, DbError>(results)
        })
        .await
        .map_err(|e| DbError::Internal(format!("task join error: {}", e)))?
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        let start = Instant::now();
        let query_owned = query.to_string();
        let connection = Arc::clone(&self.connection);

        spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
            Ok(Self::execute_statement(conn, &query_owned, start))
        })
        .await
        .map_err(|e| DbError::Internal(format!("task join error: {}", e)))?
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        Ok(Some("main".to_string()))
    }

    async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
        Err(DbError::NotSupported(
            "DuckDB does not support switching databases within one file connection".to_string(),
        ))
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();
        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;

            if options.transactional {
                let connection = Arc::clone(&self.connection);
                spawn_blocking(move || {
                    let guard = connection
                        .lock()
                        .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
                    let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
                    conn.execute("BEGIN", []).map_err(|e| {
                        DbError::transaction_with_source("failed to begin transaction", e)
                    })?;
                    Ok::<_, DbError>(())
                })
                .await
                .map_err(|e| DbError::Internal(format!("task join error: {}", e)))??;

                let mut has_error = false;
                while let Some(stmt_result) = parser.next() {
                    let bytes_read = parser.bytes_read();
                    let sql = match stmt_result {
                        Ok(s) if !s.trim().is_empty() => s,
                        Ok(_) => continue,
                        Err(e) => {
                            let progress = StreamingProgress::with_file_progress(
                                current,
                                SqlResult::Error(SqlErrorInfo {
                                    sql: String::new(),
                                    message: format!("Parse error: {}", e),
                                }),
                                bytes_read,
                                total_size,
                            );
                            let _ = sender.send(progress).await;
                            has_error = true;
                            break;
                        }
                    };

                    current += 1;
                    let start = Instant::now();
                    let sql_owned = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let connection = Arc::clone(&self.connection);
                    let result = spawn_blocking(move || {
                        let guard = connection
                            .lock()
                            .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
                        let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
                        Ok(Self::execute_statement(conn, &sql_owned, start))
                    })
                    .await
                    .map_err(|e| DbError::Internal(format!("task join error: {}", e)))??;

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    if is_error {
                        has_error = true;
                    }

                    let progress = StreamingProgress::with_file_progress(
                        current, result, bytes_read, total_size,
                    );
                    if sender.send(progress).await.is_err() {
                        has_error = true;
                        break;
                    }
                    if is_error {
                        break;
                    }
                }

                let command = if has_error { "ROLLBACK" } else { "COMMIT" };
                let connection = Arc::clone(&self.connection);
                spawn_blocking(move || {
                    let guard = connection
                        .lock()
                        .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
                    let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
                    conn.execute(command, []).map_err(|e| {
                        DbError::transaction_with_source(
                            format!("failed to {}", command.to_lowercase()),
                            e,
                        )
                    })?;
                    Ok::<_, DbError>(())
                })
                .await
                .map_err(|e| DbError::Internal(format!("task join error: {}", e)))??;
            } else {
                while let Some(stmt_result) = parser.next() {
                    let bytes_read = parser.bytes_read();
                    let sql = match stmt_result {
                        Ok(s) if !s.trim().is_empty() => s,
                        Ok(_) => continue,
                        Err(e) => {
                            let progress = StreamingProgress::with_file_progress(
                                current,
                                SqlResult::Error(SqlErrorInfo {
                                    sql: String::new(),
                                    message: format!("Parse error: {}", e),
                                }),
                                bytes_read,
                                total_size,
                            );
                            let _ = sender.send(progress).await;
                            if options.stop_on_error {
                                break;
                            }
                            continue;
                        }
                    };

                    current += 1;
                    let start = Instant::now();
                    let sql_owned = plugin.apply_query_max_rows(&sql, options.max_rows);
                    let connection = Arc::clone(&self.connection);
                    let result = spawn_blocking(move || {
                        let guard = connection
                            .lock()
                            .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
                        let conn = guard.as_ref().ok_or(DbError::NotConnected)?;
                        Ok(Self::execute_statement(conn, &sql_owned, start))
                    })
                    .await
                    .map_err(|e| DbError::Internal(format!("task join error: {}", e)))??;

                    let result = result.with_original_sql(sql.as_str());
                    let is_error = result.is_error();
                    let progress = StreamingProgress::with_file_progress(
                        current, result, bytes_read, total_size,
                    );
                    if sender.send(progress).await.is_err() {
                        break;
                    }
                    if is_error && options.stop_on_error {
                        break;
                    }
                }
            }
        } else {
            let statements: Vec<String> = parser
                .collect::<std::io::Result<Vec<_>>>()
                .map_err(|e| DbError::query_with_source("failed to parse SQL script", e))?
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            let total = statements.len();
            for (idx, sql) in statements.into_iter().enumerate() {
                let result = self.execute(plugin, &sql, options.clone()).await?;
                for item in result {
                    let progress = StreamingProgress::new(idx + 1, total, item);
                    if sender.send(progress).await.is_err() {
                        return Ok(());
                    }
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::DuckDbConnection;
    use crate::connection::DbConnection;
    use crate::executor::{ExecOptions, SqlResult};
    use db_value::{CellState, DbValue};
    use one_core::storage::{DatabaseType, DbConnectionConfig};

    fn test_connection(db_path: &std::path::Path, id: &str) -> DuckDbConnection {
        DuckDbConnection::new(DbConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            database_type: DatabaseType::DuckDB,
            host: db_path.to_string_lossy().to_string(),
            port: 0,
            workspace_id: None,
            credential_reference: None,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            proxy: None,
            extra_params: Default::default(),
        })
    }

    #[tokio::test]
    async fn test_duckdb_connection_can_query_temp_database() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("duckdb-basic-test.duckdb");

        let mut connection = test_connection(&db_path, "duckdb-test");

        connection.connect().await.expect("duckdb should connect");

        let result = connection
            .query("select 42 as answer")
            .await
            .expect("query should succeed");

        match result {
            SqlResult::Query(query) => {
                assert_eq!(query.columns, vec!["answer".to_string()]);
                assert_eq!(query.rows, vec![vec![Some("42".to_string())]]);
            }
            other => panic!("expected query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("duckdb should disconnect");
    }

    #[tokio::test]
    async fn transactional_execute_rolls_back_after_error_when_continuing() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir
            .path()
            .join("duckdb-transaction-rollback-test.duckdb");
        let mut connection = test_connection(&db_path, "duckdb-transaction-rollback-test");
        connection.connect().await.expect("duckdb should connect");
        connection
            .query("CREATE TABLE items (id INTEGER PRIMARY KEY)")
            .await
            .expect("fixture table should be created");

        let results = connection
            .execute(
                &crate::duckdb::DuckDbPlugin::new(),
                "INSERT INTO items VALUES (1);
                 INSERT INTO missing_table VALUES (2);
                 INSERT INTO items VALUES (3);",
                ExecOptions {
                    stop_on_error: false,
                    transactional: true,
                    ..Default::default()
                },
            )
            .await
            .expect("script execution should return statement results");

        assert_eq!(3, results.len());
        assert!(matches!(results[0], SqlResult::Exec(_)));
        assert!(matches!(results[1], SqlResult::Error(_)));
        assert!(matches!(results[2], SqlResult::Exec(_)));

        match connection
            .query("SELECT COUNT(*) AS count FROM items")
            .await
            .expect("row count should be queryable")
        {
            SqlResult::Query(result) => {
                assert_eq!(vec![vec![Some("0".to_string())]], result.rows);
            }
            other => panic!("expected query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("duckdb should disconnect");
    }

    #[tokio::test]
    async fn runtime_blob_is_binary_even_when_bytes_are_valid_utf8() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("duckdb-blob-binary-test.duckdb");
        let mut connection = test_connection(&db_path, "duckdb-blob-binary-test");
        connection.connect().await.expect("duckdb should connect");
        connection
            .query("CREATE TABLE blobs (payload BLOB)")
            .await
            .expect("fixture table should be created");
        connection
            .query("INSERT INTO blobs VALUES (CAST('hello' AS BLOB))")
            .await
            .expect("blob fixture should be inserted");

        match connection
            .query("SELECT payload FROM blobs")
            .await
            .expect("blob query should succeed")
        {
            SqlResult::Query(result) => {
                let batch = result
                    .typed_batch()
                    .expect("duckdb result should carry a typed batch");
                assert_eq!(
                    batch.rows[0].cells[0],
                    CellState::Decoded(DbValue::Binary(b"hello".to_vec()))
                );
                assert_eq!(result.rows, vec![vec![Some("hello".to_string())]]);
                assert_eq!(result.binary_cells[0].bytes, b"hello".to_vec());
            }
            other => panic!("expected query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("duckdb should disconnect");
    }

    #[tokio::test]
    async fn null_is_distinct_from_empty_text_and_integer_is_typed() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("duckdb-null-typed-test.duckdb");
        let mut connection = test_connection(&db_path, "duckdb-null-typed-test");
        connection.connect().await.expect("duckdb should connect");
        connection
            .query("CREATE TABLE values (id INTEGER, txt TEXT)")
            .await
            .expect("fixture table should be created");
        connection
            .query("INSERT INTO values VALUES (1, NULL), (2, '')")
            .await
            .expect("fixtures should be inserted");

        match connection
            .query("SELECT id, txt FROM values ORDER BY id")
            .await
            .expect("typed query should succeed")
        {
            SqlResult::Query(result) => {
                let batch = result
                    .typed_batch()
                    .expect("duckdb result should carry a typed batch");
                assert_eq!(
                    batch.rows[0].cells,
                    vec![
                        CellState::Decoded(DbValue::Integer("1".to_string())),
                        CellState::Decoded(DbValue::Null),
                    ]
                );
                assert_eq!(
                    batch.rows[1].cells,
                    vec![
                        CellState::Decoded(DbValue::Integer("2".to_string())),
                        CellState::Decoded(DbValue::Text(String::new())),
                    ]
                );
            }
            other => panic!("expected query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("duckdb should disconnect");
    }
}
