use std::sync::{Arc, Mutex};
use std::time::Instant;

use async_trait::async_trait;
use rusqlite::{Connection, OpenFlags, types::ValueRef};
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

pub struct SqliteDbConnection {
    config: DbConnectionConfig,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl SqliteDbConnection {
    pub fn new(config: DbConnectionConfig) -> Self {
        Self {
            config,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    /// 把 SQLite 运行时 storage class 值解码为带类型值和对应 legacy 显示。
    ///
    /// 运行时类型是权威：`Blob` 永远是 [`DbValue::Binary`]（即使字节恰好是
    /// 合法 UTF-8），`Text` 非法 UTF-8 是显式 [`CellState::DecodeError`] 而不是
    /// NULL，`Null` 与解码失败严格分开。整数不再按声明的 DATE/TIME 亲和性改写。
    fn extract_value(value: ValueRef<'_>) -> (CellState, Option<String>) {
        match value {
            ValueRef::Null => (CellState::Decoded(DbValue::Null), None),
            ValueRef::Integer(i) => (
                CellState::Decoded(DbValue::Integer(i.to_string())),
                Some(i.to_string()),
            ),
            ValueRef::Real(f) => (
                CellState::Decoded(DbValue::Float {
                    value: f.to_string(),
                    width: FloatWidth::F64,
                }),
                Some(f.to_string()),
            ),
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
        }
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

    /// 从 typed batch 派生 legacy `binary_cells`，保证字节与类型权威一致。
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

    fn execute_statement(conn: &Connection, sql: &str, start: Instant) -> SqlResult {
        let sql_preview = if sql.len() > 200 {
            format!("{}...", truncate_str(sql, 200))
        } else {
            sql.to_string()
        };

        match conn.prepare(sql) {
            Ok(mut stmt) => {
                let column_count = stmt.column_count();

                if column_count == 0 {
                    match conn.execute(sql, []) {
                        Ok(rows_affected) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[SQLite] Execute completed: {} rows affected, {}ms",
                                rows_affected, elapsed_ms
                            );
                            Self::build_exec_result(
                                sql.to_string(),
                                rows_affected as u64,
                                elapsed_ms,
                            )
                        }
                        Err(e) => {
                            error!("[SQLite] Execute failed: {}, SQL: {}", e, sql_preview);
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql.to_string(),
                                message: e.to_string(),
                            })
                        }
                    }
                } else {
                    let stmt_columns = stmt.columns();
                    let columns: Vec<String> =
                        stmt_columns.iter().map(|c| c.name().to_string()).collect();

                    let column_types: Vec<Option<String>> = stmt_columns
                        .iter()
                        .map(|c| c.decl_type().map(|s| s.to_string()))
                        .collect();

                    let rows_result: Result<
                        (Vec<Vec<CellState>>, Vec<Vec<Option<String>>>),
                        rusqlite::Error,
                    > = stmt.query([]).and_then(|mut rows| {
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
                        Ok((cells_rows, display_rows))
                    });

                    match rows_result {
                        Ok((cells_rows, display_rows)) => {
                            let elapsed_ms = start.elapsed().as_millis();
                            debug!(
                                "[SQLite] Query completed: {} rows, {} columns, {}ms",
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
                            error!("[SQLite] Query failed: {}, SQL: {}", e, sql_preview);
                            SqlResult::Error(SqlErrorInfo {
                                sql: sql.to_string(),
                                message: e.to_string(),
                            })
                        }
                    }
                }
            }
            Err(e) => {
                error!("[SQLite] Prepare failed: {}, SQL: {}", e, sql_preview);
                SqlResult::Error(SqlErrorInfo {
                    sql: sql.to_string(),
                    message: e.to_string(),
                })
            }
        }
    }
}

#[async_trait]
impl DbConnection for SqliteDbConnection {
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
                .ok_or_else(|| DbError::connection("database path is required for SQLite"))?
        };

        info!("[SQLite] Connecting to {}", database_path);

        let conn = spawn_blocking(move || {
            Connection::open_with_flags(
                &database_path,
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
            )
        })
        .await
        .map_err(|e| {
            error!("[SQLite] Task join error: {}", e);
            DbError::Internal(format!("task join error: {}", e))
        })?
        .map_err(|e| {
            error!("[SQLite] Connection failed: {}", e);
            DbError::connection_with_source("failed to connect", e)
        })?;

        debug!("[SQLite] Setting pragmas...");
        conn.execute_batch(
            "PRAGMA journal_mode = WAL;
             PRAGMA foreign_keys = ON;
             PRAGMA busy_timeout = 5000;",
        )
        .map_err(|e| {
            error!("[SQLite] Failed to set pragmas: {}", e);
            DbError::connection_with_source("failed to set pragmas", e)
        })?;

        {
            let mut guard = self
                .connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            *guard = Some(conn);
        }

        info!("[SQLite] Connected successfully");
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        debug!("[SQLite] Disconnecting...");
        let conn_opt = {
            let mut guard = self
                .connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            guard.take()
        };

        if let Some(conn) = conn_opt {
            spawn_blocking(move || drop(conn)).await.map_err(|e| {
                error!("[SQLite] Disconnect failed: {}", e);
                DbError::Internal(format!("task join error: {}", e))
            })?;
        }

        info!("[SQLite] Disconnected");
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        debug!(
            "[SQLite] execute() called, stop_on_error={}",
            options.stop_on_error
        );
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
        debug!("[SQLite] Split into {} statement(s)", statements.len());
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
        let results = spawn_blocking(move || {
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
        .map_err(|e| {
            error!("[SQLite] Task join error: {}", e);
            DbError::Internal(format!("task join error: {}", e))
        })??;

        debug!(
            "[SQLite] execute() completed with {} result(s)",
            results.len()
        );
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        debug!("[SQLite] query() called");
        let start = Instant::now();
        let query_owned = query.to_string();
        let connection = Arc::clone(&self.connection);

        let result = spawn_blocking(move || {
            let guard = connection
                .lock()
                .map_err(|e| DbError::Internal(format!("lock poisoned: {}", e)))?;
            let conn = guard.as_ref().ok_or(DbError::NotConnected)?;

            Ok(Self::execute_statement(conn, &query_owned, start))
        })
        .await
        .map_err(|e| {
            error!("[SQLite] Query task join error: {}", e);
            DbError::Internal(format!("task join error: {}", e))
        })??;

        Ok(result)
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        Ok(self.config.database.clone())
    }

    async fn switch_database(&self, _database: &str) -> Result<(), DbError> {
        Err(DbError::NotSupported(
            "SQLite does not support switching databases. Each database is a separate file connection.".to_string()
        ))
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        debug!(
            "[SQLite] execute_streaming() called, transactional={}, streaming={}",
            options.transactional, options.streaming
        );

        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();

        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;

            if options.transactional {
                debug!("[SQLite] Starting transaction for streaming...");
                {
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
                }

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
                    debug!("[SQLite] Streaming TX statement {}", current);
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

                {
                    let connection = Arc::clone(&self.connection);
                    let command = if has_error { "ROLLBACK" } else { "COMMIT" };
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
                }
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
                    debug!("[SQLite] Streaming statement {}", current);
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
            debug!("[SQLite] Streaming {} statement(s)", total);

            if options.transactional {
                debug!("[SQLite] Starting transaction for streaming...");
                {
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
                }

                let mut has_error = false;

                for (index, sql) in statements.into_iter().enumerate() {
                    let current = index + 1;
                    debug!("[SQLite] Streaming TX statement {}/{}", current, total);
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

                    let progress = StreamingProgress::new(current, total, result);
                    if sender.send(progress).await.is_err() {
                        has_error = true;
                        break;
                    }

                    if is_error {
                        break;
                    }
                }

                {
                    let connection = Arc::clone(&self.connection);
                    let command = if has_error { "ROLLBACK" } else { "COMMIT" };
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
                }
            } else {
                for (index, sql) in statements.into_iter().enumerate() {
                    let current = index + 1;
                    debug!("[SQLite] Streaming statement {}/{}", current, total);
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
                    let progress = StreamingProgress::new(current, total, result);
                    if sender.send(progress).await.is_err() {
                        break;
                    }

                    if is_error && options.stop_on_error {
                        break;
                    }
                }
            }
        }

        debug!("[SQLite] execute_streaming() completed");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::SqliteDbConnection;
    use crate::connection::DbConnection;
    use crate::executor::{ExecOptions, SqlResult};
    use crate::sqlite::SqlitePlugin;
    use db_value::{CellState, DbValue};
    use one_core::storage::{DatabaseType, DbConnectionConfig};

    fn test_connection(db_path: &std::path::Path, id: &str) -> SqliteDbConnection {
        SqliteDbConnection::new(DbConnectionConfig {
            id: id.to_string(),
            name: id.to_string(),
            database_type: DatabaseType::SQLite,
            host: db_path.to_string_lossy().to_string(),
            port: 0,
            workspace_id: None,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            proxy: None,
            credential_reference: None,
            extra_params: Default::default(),
        })
    }

    #[tokio::test]
    async fn execute_limits_rows_but_preserves_original_query_sql() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("sqlite-row-limit-test.db");
        let mut connection = test_connection(&db_path, "sqlite-row-limit-test");
        connection.connect().await.expect("sqlite should connect");
        {
            let guard = connection.connection.lock().expect("lock should succeed");
            let sqlite = guard.as_ref().expect("sqlite should be connected");
            sqlite
                .execute_batch(
                    "CREATE TABLE users (id INTEGER PRIMARY KEY);
                     WITH RECURSIVE ids(id) AS (
                         SELECT 1
                         UNION ALL
                         SELECT id + 1 FROM ids WHERE id < 1001
                     )
                     INSERT INTO users SELECT id FROM ids;",
                )
                .expect("fixture should be created");
        }

        let original_sql = "SELECT id FROM users ORDER BY id";
        let results = connection
            .execute(
                &SqlitePlugin::new(),
                original_sql,
                ExecOptions {
                    max_rows: Some(2),
                    ..Default::default()
                },
            )
            .await
            .expect("query should execute");

        match results.as_slice() {
            [SqlResult::Query(result)] => {
                assert_eq!(2, result.rows.len());
                assert_eq!(original_sql, result.sql);
            }
            other => panic!("expected one query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("sqlite should disconnect");
    }

    #[tokio::test]
    async fn transactional_execute_rolls_back_after_error_when_continuing() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("sqlite-transaction-rollback-test.db");
        let mut connection = test_connection(&db_path, "sqlite-transaction-rollback-test");
        connection.connect().await.expect("sqlite should connect");
        connection
            .query("CREATE TABLE items (id INTEGER PRIMARY KEY)")
            .await
            .expect("fixture table should be created");

        let results = connection
            .execute(
                &SqlitePlugin::new(),
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
            .expect("sqlite should disconnect");
    }

    #[tokio::test]
    async fn runtime_blob_is_binary_even_when_bytes_are_valid_utf8() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("sqlite-blob-binary-test.db");
        let mut connection = test_connection(&db_path, "sqlite-blob-binary-test");
        connection.connect().await.expect("sqlite should connect");

        {
            let guard = connection.connection.lock().expect("lock should succeed");
            let sqlite = guard.as_ref().expect("sqlite should be connected");
            sqlite
                .execute_batch(
                    "CREATE TABLE blobs (id INTEGER PRIMARY KEY, payload BLOB);
                     INSERT INTO blobs VALUES (1, x'68656C6C6F');",
                )
                .expect("fixture should be created");
        }

        match connection
            .query("SELECT payload FROM blobs WHERE id = 1")
            .await
            .expect("blob query should succeed")
        {
            SqlResult::Query(result) => {
                let batch = result
                    .typed_batch()
                    .expect("sqlite result should carry a typed batch");
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
            .expect("sqlite should disconnect");
    }

    #[tokio::test]
    async fn null_is_distinct_from_empty_and_integer_is_not_date_rewritten() {
        let temp_dir = tempfile::tempdir().expect("temp dir should be created");
        let db_path = temp_dir.path().join("sqlite-null-typed-test.db");
        let mut connection = test_connection(&db_path, "sqlite-null-typed-test");
        connection.connect().await.expect("sqlite should connect");

        {
            let guard = connection.connection.lock().expect("lock should succeed");
            let sqlite = guard.as_ref().expect("sqlite should be connected");
            sqlite
                .execute_batch(
                    "CREATE TABLE samples (id INTEGER PRIMARY KEY, declared_date DATE, txt TEXT);
                     INSERT INTO samples VALUES (1, 0, NULL);
                     INSERT INTO samples VALUES (2, 42, '');",
                )
                .expect("fixture should be created");
        }

        match connection
            .query("SELECT declared_date, txt FROM samples ORDER BY id")
            .await
            .expect("typed query should succeed")
        {
            SqlResult::Query(result) => {
                let batch = result
                    .typed_batch()
                    .expect("sqlite result should carry a typed batch");
                assert_eq!(
                    batch.rows[0].cells,
                    vec![
                        CellState::Decoded(DbValue::Integer("0".to_string())),
                        CellState::Decoded(DbValue::Null),
                    ]
                );
                assert_eq!(
                    batch.rows[1].cells,
                    vec![
                        CellState::Decoded(DbValue::Integer("42".to_string())),
                        CellState::Decoded(DbValue::Text(String::new())),
                    ]
                );
                assert_eq!(
                    result.rows,
                    vec![
                        vec![Some("0".to_string()), None],
                        vec![Some("42".to_string()), Some(String::new())],
                    ]
                );
            }
            other => panic!("expected query result, got {other:?}"),
        }

        connection
            .disconnect()
            .await
            .expect("sqlite should disconnect");
    }
}
