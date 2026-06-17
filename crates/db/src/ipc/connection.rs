//! 通过 v2 wire 协议(`extension-protocol`)访问外部驱动子进程的数据库连接实现。
//!
//! 与 P3.4 之前的实现相比,核心差异:
//!
//! - **conn_id 化**:`conn/open` 返回的 `conn_id` 会贯穿连接的整个生命周期,
//!   被注入到所有后续 wire 调用的 `params` 里。
//! - **查询流式化**:`query/start` + `cursor/fetch` 多次拉取 + `cursor/close`
//!   组合,本层把多次 fetch buffer 起来后还原成宿主侧 [`SqlResult`]。
//! - **exec 单独走 `exec/run`**:DDL / DML 在客户端就分流,不再混在 `query`。
//! - **wire 透传**:`ExternalDatabasePlugin` 把 schema/* 等方法包成
//!   `/*onetcli-ipc-wire*/ {json}` 注入到 `query`,这里解码再调原 wire 方法,
//!   连接内 method 会自动注入 `conn_id`,纯工具 method 保持 connless。

use crate::connection::{DbConnection, DbError, StreamingProgress};
use crate::executor::{
    ExecOptions, ExecResult, QueryColumnMeta, QueryResult, SqlResult, SqlSource,
};
use crate::ipc::client::JsonRpcClient;
use crate::ipc::method_support::{MethodSet, MethodSupport};
use crate::ipc::protocol::{
    conn_only_params, conn_use_params, cursor_close_params, cursor_fetch_params,
    driver_config_value, exec_batch_params, exec_run_params, query_start_params,
};
use crate::ipc::registry::IpcDriverManifest;
use crate::{DatabasePlugin, SqlErrorInfo, truncate_str};
use async_trait::async_trait;
use extension_protocol::conn::ConnId;
use extension_protocol::method;
use extension_protocol::row::{CellValue, ColumnSpec, Row};
use one_core::storage::DbConnectionConfig;
use serde::Deserialize;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::Instant;
use tokio::sync::{Mutex, mpsc};
use tracing::{debug, error, warn};

/// 单次 cursor/fetch 拉取的行数上限——稍大一些减少 round-trip,但避免一次 buffer 太多。
const DEFAULT_FETCH_SIZE: u32 = 2_000;

/// wire 透传信封前缀:plugin 把同步方法包装成「伪 SQL」注入到 `query`,connection
/// 识别此前缀后解码并改调真正的 wire 方法。
pub(crate) const WIRE_PREFIX: &str = "/*onetcli-ipc-wire*/ ";

pub struct ExternalDbConnection {
    config: DbConnectionConfig,
    driver: IpcDriverManifest,
    /// `Arc` 让 `request` 能短锁拿 clone 后立刻释放,允许多 caller 并发调用
    /// `JsonRpcClient` 的 request 接口。
    client: Mutex<Option<Arc<JsonRpcClient>>>,
    /// `conn/open` 返回的连接 id;disconnect 时 `conn/close`。
    /// 用 std Mutex 是因为它只在「连接级」短时持有,await 时不持锁。
    conn_id: StdMutex<Option<ConnId>>,
    /// 生效的 wire method 集合(`init` 动态声明优先,manifest 静态声明回退)。
    /// connect 时解析,wire 透传时用于决定调外部实现还是直接走宿主 fallback。
    method_set: StdMutex<MethodSet>,
}

impl ExternalDbConnection {
    pub fn new(config: DbConnectionConfig, driver: IpcDriverManifest) -> Self {
        Self {
            config,
            driver,
            client: Mutex::new(None),
            conn_id: StdMutex::new(None),
            method_set: StdMutex::new(MethodSet::legacy()),
        }
    }

    /// 共享 client(短锁)。
    async fn client(&self) -> Result<Arc<JsonRpcClient>, DbError> {
        let guard = self.client.lock().await;
        guard.as_ref().cloned().ok_or(DbError::NotConnected)
    }

    /// 当前 conn_id;disconnect 后为 None。
    fn current_conn_id(&self) -> Result<ConnId, DbError> {
        self.conn_id
            .lock()
            .expect("conn_id mutex poisoned")
            .ok_or(DbError::NotConnected)
    }

    fn set_conn_id(&self, id: Option<ConnId>) {
        *self.conn_id.lock().expect("conn_id mutex poisoned") = id;
    }

    /// 包装 wire 调用,并处理 reader-task 已退出后的 client eviction。
    async fn call<T>(&self, method: &str, params: Value) -> Result<T, DbError>
    where
        T: serde::de::DeserializeOwned,
    {
        let client = self.client().await?;
        let result = client.request::<T>(method, params).await;
        self.evict_if_closed(&client).await;
        result
    }

    /// 把 raw value 直接返回(用于 wire 透传场景)。
    async fn call_value(&self, method: &str, params: Value) -> Result<Value, DbError> {
        let client = self.client().await?;
        let result = client.request_value(method, params).await;
        self.evict_if_closed(&client).await;
        result
    }

    /// 若底层 reader task 已退出,把当前 broken client evict 出去,让下次调用
    /// 立刻报 NotConnected → 上层触发重连。
    async fn evict_if_closed(&self, client: &Arc<JsonRpcClient>) {
        if client.is_closed() {
            let mut guard = self.client.lock().await;
            if let Some(current) = guard.as_ref() {
                if Arc::ptr_eq(current, client) {
                    *guard = None;
                    self.set_conn_id(None);
                }
            }
        }
    }

    /// 跑一条非查询语句(DDL/DML)→ `exec/run`。
    async fn exec_run(&self, sql: &str, start: Instant) -> Result<SqlResult, DbError> {
        let conn_id = self.current_conn_id()?;
        let result: ExecRunOutput = match self
            .call::<ExecRunOutput>(method::EXEC_RUN, exec_run_params(conn_id, sql))
            .await
        {
            Ok(value) => value,
            Err(error) => {
                // SQL 错误回成 SqlResult::Error,而不是 DbError
                if let Some(message) = sql_error_message(&error) {
                    return Ok(SqlResult::Error(SqlErrorInfo {
                        sql: sql.to_string(),
                        message,
                    }));
                }
                return Err(error);
            }
        };
        Ok(SqlResult::Exec(ExecResult {
            sql: sql.to_string(),
            rows_affected: result.affected_rows,
            elapsed_ms: start.elapsed().as_millis(),
            message: Some(crate::executor::format_message(sql, result.affected_rows)),
        }))
    }

    /// 跑一批非查询语句 → `exec/batch`。查询结果仍走逐条 `query/start`。
    async fn exec_batch(
        &self,
        statements: &[String],
        options: &ExecOptions,
        start: Instant,
    ) -> Result<Vec<SqlResult>, DbError> {
        let conn_id = self.current_conn_id()?;
        let output: ExecBatchOutput = self
            .call(
                method::EXEC_BATCH,
                exec_batch_params(
                    conn_id,
                    statements,
                    options.stop_on_error,
                    options.transactional,
                ),
            )
            .await?;
        batch_output_to_results(statements, output, options.stop_on_error, start)
    }

    fn should_try_exec_batch(&self, statements: &[String], options: &ExecOptions) -> bool {
        if statements.is_empty() || (!options.transactional && statements.len() < 2) {
            return false;
        }
        if statements
            .iter()
            .any(|sql| is_query_sql(sql) || sql.starts_with(WIRE_PREFIX))
        {
            return false;
        }
        !matches!(
            self.method_set
                .lock()
                .expect("method_set mutex poisoned")
                .support(method::EXEC_BATCH),
            MethodSupport::Unsupported
        )
    }

    /// 跑一条查询语句 → `query/start` + 多次 `cursor/fetch` + `cursor/close`。
    async fn query_select(
        &self,
        sql: &str,
        start: Instant,
        max_rows: Option<usize>,
    ) -> Result<SqlResult, DbError> {
        let conn_id = self.current_conn_id()?;
        let start_resp: QueryStartOutput = match self
            .call::<QueryStartOutput>(
                method::QUERY_START,
                query_start_params(conn_id, sql, max_rows_to_u64(max_rows)),
            )
            .await
        {
            Ok(v) => v,
            Err(error) => {
                if let Some(message) = sql_error_message(&error) {
                    return Ok(SqlResult::Error(SqlErrorInfo {
                        sql: sql.to_string(),
                        message,
                    }));
                }
                return Err(error);
            }
        };

        let cursor_id = start_resp.cursor_id.clone();
        let columns: Vec<String> = start_resp.columns.iter().map(|c| c.name.clone()).collect();
        let column_meta: Vec<QueryColumnMeta> =
            start_resp.columns.iter().map(column_spec_to_meta).collect();

        // 先把当前结果集 fetch 出来(可能中途出错),但无论成败都必须 close,否则 driver 端
        // 的 cursor 状态会一直占着资源直到连接关闭。
        let fetch_outcome = self.fetch_all_rows(conn_id, &cursor_id).await;

        // 显式 close cursor;失败不致命(driver 在 disconnect 时也会清理)。
        if let Err(error) = self
            .call_value(
                method::CURSOR_CLOSE,
                cursor_close_params(conn_id, &cursor_id),
            )
            .await
        {
            warn!(cursor = %cursor_id, error = %error, "cursor/close failed (non-fatal)");
        }

        // close 之后再传播 fetch 错误,保证错误路径也释放了 cursor。
        let rows = fetch_outcome?;

        Ok(SqlResult::Query(QueryResult {
            sql: sql.to_string(),
            columns,
            column_meta,
            rows,
            elapsed_ms: start.elapsed().as_millis(),
        }))
    }

    /// 把游标里所有行 fetch 出来并转成宿主侧字符串行。
    ///
    /// 抽成独立方法是为了让 [`query_select`] 能在 fetch 出错时仍统一走 `cursor/close`
    /// (RAII 式清理),对标 `JsonRpcClient` 的 `PendingGuard`——避免 `?` 提前返回
    /// 跳过 close 导致 driver 端 cursor buffer 泄漏。
    async fn fetch_all_rows(
        &self,
        conn_id: ConnId,
        cursor_id: &str,
    ) -> Result<Vec<Vec<Option<String>>>, DbError> {
        let mut rows: Vec<Vec<Option<String>>> = Vec::new();
        loop {
            let fetch_resp: CursorFetchOutput = self
                .call::<CursorFetchOutput>(
                    method::CURSOR_FETCH,
                    cursor_fetch_params(conn_id, cursor_id, Some(DEFAULT_FETCH_SIZE)),
                )
                .await?;

            for row in fetch_resp.rows {
                rows.push(row_to_strings(row));
            }
            if fetch_resp.done {
                break;
            }
        }
        Ok(rows)
    }

    /// wire 透传分发:解析信封 → 按需注入 conn_id → 按 [`MethodSupport`] 决定调外部
    /// 实现还是直接走宿主 fallback。这是外部驱动 method 门控的唯一权威入口。
    async fn dispatch_wire(&self, original: &str, request: &str) -> Result<SqlResult, DbError> {
        let value: Value = serde_json::from_str(request)
            .map_err(|error| DbError::query_with_source("invalid wire request", error))?;
        let method_name = value
            .get("method")
            .and_then(Value::as_str)
            .ok_or_else(|| DbError::query("wire request method is required"))?
            .to_string();
        let mut params = value.get("params").cloned().unwrap_or_else(|| json!({}));

        // 宿主预先算好的 fallback_sql(供 Unsupported / NOT_FOUND 时本地执行)。
        let fallback_sql = params
            .get("fallback_sql")
            .and_then(Value::as_str)
            .map(str::to_string);

        // 仅连接内 method 自动注入 conn_id。DDL / SQL 工具等 connless method
        // 必须保持无 conn_id,避免被 runtime 路由到连接 worker 后排队阻塞。
        if method_requires_auto_conn_id(&method_name) {
            if let Some(obj) = params.as_object_mut() {
                if !obj.contains_key("conn_id") {
                    let conn_id = self.current_conn_id()?;
                    obj.insert("conn_id".to_string(), json!(conn_id));
                }
            }
        }

        let support = self
            .method_set
            .lock()
            .expect("method_set mutex poisoned")
            .support(&method_name);

        // 驱动显式声明「不支持」→ 不发 round-trip,直接走宿主 fallback。
        if matches!(support, MethodSupport::Unsupported) {
            return self
                .run_wire_fallback(&method_name, fallback_sql.as_deref())
                .await;
        }

        match self.call_value(&method_name, params).await {
            Ok(result) => Ok(wire_result_for_method(original, &method_name, result)),
            // 已声明却 NOT_FOUND(驱动 bug)或 legacy 未声明 → 有 fallback 就回退。
            Err(error) if is_method_not_found(&error) => {
                if matches!(support, MethodSupport::Supported) {
                    warn!(
                        method = %method_name,
                        "driver declared method but returned METHOD_NOT_FOUND; falling back to host"
                    );
                }
                match fallback_sql {
                    Some(sql) => self.run_fallback_sql(&sql).await,
                    None => Err(error),
                }
            }
            Err(error) => Err(error),
        }
    }

    /// `Unsupported` 分支:有 fallback_sql 就本地执行,否则报 NotSupported。
    async fn run_wire_fallback(
        &self,
        method_name: &str,
        fallback_sql: Option<&str>,
    ) -> Result<SqlResult, DbError> {
        match fallback_sql {
            Some(sql) => self.run_fallback_sql(sql).await,
            None => Err(DbError::NotSupported(format!(
                "external driver does not implement `{method_name}` and no host fallback is available"
            ))),
        }
    }

    /// 在本地连接上执行宿主 fallback SQL(SELECT-like 走 query/start,否则 exec/run)。
    async fn run_fallback_sql(&self, fallback_sql: &str) -> Result<SqlResult, DbError> {
        let start = Instant::now();
        if is_query_sql(fallback_sql) {
            self.query_select(fallback_sql, start, None).await
        } else {
            self.exec_run(fallback_sql, start).await
        }
    }

    async fn query_with_max_rows(
        &self,
        query: &str,
        max_rows: Option<usize>,
    ) -> Result<SqlResult, DbError> {
        // wire 透传:把 plugin 的 schema/* 等包成伪 SQL 直送,这里解码改调真 wire 方法。
        if let Some(request) = query.strip_prefix(WIRE_PREFIX) {
            return self.dispatch_wire(query, request).await;
        }

        // 普通 SQL:按前缀分流到 query/start 或 exec/run。
        let start = Instant::now();
        if is_query_sql(query) {
            self.query_select(query, start, max_rows).await
        } else {
            self.exec_run(query, start).await
        }
    }
}

fn max_rows_to_u64(max_rows: Option<usize>) -> Option<u64> {
    max_rows.map(|rows| u64::try_from(rows).unwrap_or(u64::MAX))
}

/// 用 SQL 前缀判断是 SELECT-like 还是 DDL/DML。
fn is_query_sql(sql: &str) -> bool {
    let normalized = sql.trim_start().to_ascii_uppercase();
    normalized.starts_with("SELECT")
        || normalized.starts_with("WITH")
        || normalized.starts_with("PRAGMA")
        || normalized.starts_with("SHOW")
        || normalized.starts_with("DESCRIBE")
        || normalized.starts_with("EXPLAIN")
}

/// 是否由宿主自动补 `conn_id`。
///
/// `ddl/*` 和多数 `sql/*` 是纯工具方法,即使有打开的连接也不应默认排进
/// per-connection worker,否则长查询会阻塞表设计器预览 / DDL 生成等 UI 路径。
fn method_requires_auto_conn_id(method_name: &str) -> bool {
    matches!(
        method::namespace(method_name),
        "schema" | "query" | "cursor" | "exec" | "tx" | "data" | "stream"
    ) || matches!(
        method_name,
        method::CONN_PING | method::CONN_USE | method::CONN_CLOSE | method::SQL_EXPLAIN
    )
}

/// 把 v2 wire 的列描述转成宿主侧的 `QueryColumnMeta`。
fn column_spec_to_meta(spec: &ColumnSpec) -> QueryColumnMeta {
    let meta = QueryColumnMeta::new(spec.name.clone(), spec.type_str.clone());
    match spec.nullable {
        Some(nullable) => meta.with_nullable(nullable),
        None => meta,
    }
}

/// 把一行 `Row` 翻译成宿主侧的 `Vec<Option<String>>`(每列一个可空字符串)。
fn row_to_strings(row: Row) -> Vec<Option<String>> {
    row.into_iter().map(cell_to_string).collect()
}

/// 单个 `CellValue` → 显示字符串。
///
/// 设计考虑:宿主层(`QueryResult`)把列值统一存为 `Option<String>`,这是历史
/// 选择(参考 SqlitePlugin / MySqlPlugin)。本翻译尽量保持「人可读」+ 「数据无损」:
/// - 数值:`to_string()`
/// - decimal/uuid/datetime:已经是字符串,直接透传
/// - bytes:`0x...` hex 化,保持宿主表格中的二进制展示稳定
/// - json / array / map / geo / custom:`to_string()` 让 caller 看到原 JSON
fn cell_to_string(cell: CellValue) -> Option<String> {
    use base64::Engine;
    match cell {
        CellValue::Null => None,
        CellValue::Bool { value } => Some(value.to_string()),
        CellValue::I64 { value } => Some(value.to_string()),
        CellValue::U64 { value } => Some(value.to_string()),
        CellValue::F64 { value } => Some(value.to_string()),
        CellValue::Decimal { value }
        | CellValue::Text { value }
        | CellValue::Uuid { value }
        | CellValue::Date { value }
        | CellValue::Time { value }
        | CellValue::Datetime { value }
        | CellValue::Duration { value } => Some(value),
        CellValue::Bytes { value } => {
            // wire 上是 base64,宿主查询表格统一展示为 hex(`0x...`)。
            match base64::engine::general_purpose::STANDARD.decode(value.as_bytes()) {
                Ok(bytes) => Some(format!("0x{}", hex::encode(&bytes))),
                Err(_) => Some(value), // 解码失败,原 string 显示
            }
        }
        CellValue::Json { value } => Some(value.to_string()),
        CellValue::Array { value, .. } => Some(json!(value).to_string()),
        CellValue::Map { value } => Some(Value::Object(value).to_string()),
        CellValue::Geo { value, .. } => Some(value),
        CellValue::Custom { subtype, raw } => Some(format!("custom:{subtype}({raw})")),
    }
}

/// 把 SQL/查询错误的 [`DbError`] 提取出来给 `SqlResult::Error`;别的错误返回 None
/// 让 caller 继续按 `Err` 处理。
fn sql_error_message(error: &DbError) -> Option<String> {
    match error {
        DbError::Query { message, .. } if message.contains("sql error") => Some(message.clone()),
        // host_error_to_db_error 对 SQL_* 错误码统一加 "external driver sql error:" 前缀
        DbError::Query { message, .. } if message.contains("external driver sql error") => {
            Some(message.clone())
        }
        _ => None,
    }
}

/// `exec/run` 响应。
#[derive(Debug, Deserialize)]
struct ExecRunOutput {
    affected_rows: u64,
    #[serde(default)]
    #[allow(dead_code)]
    last_insert_id: Option<CellValue>,
    #[serde(default)]
    #[allow(dead_code)]
    warnings: Vec<String>,
}

/// `exec/batch` 响应。
#[derive(Debug, Deserialize)]
struct ExecBatchOutput {
    results: Vec<ExecRunOutput>,
    #[serde(default)]
    errors: Vec<BatchErrorOutput>,
}

#[derive(Debug, Deserialize)]
struct BatchErrorOutput {
    index: u32,
    #[allow(dead_code)]
    code: i32,
    message: String,
}

/// `query/start` 响应。
#[derive(Debug, Deserialize)]
struct QueryStartOutput {
    cursor_id: String,
    columns: Vec<ColumnSpec>,
    #[serde(default)]
    #[allow(dead_code)]
    row_count_known: bool,
    #[serde(default)]
    #[allow(dead_code)]
    row_count_estimate: Option<u64>,
}

/// `cursor/fetch` 响应。
#[derive(Debug, Deserialize)]
struct CursorFetchOutput {
    rows: Vec<Row>,
    #[serde(default)]
    done: bool,
}

/// `conn/open` 响应。
#[derive(Debug, Deserialize)]
struct ConnOpenOutput {
    conn_id: ConnId,
    #[serde(default)]
    #[allow(dead_code)]
    server_info: Option<Value>,
}

fn batch_output_to_results(
    statements: &[String],
    output: ExecBatchOutput,
    stop_on_error: bool,
    start: Instant,
) -> Result<Vec<SqlResult>, DbError> {
    validate_batch_output(statements.len(), &output, stop_on_error)?;
    let errors = batch_errors_by_index(output.errors)?;
    let elapsed_ms = start.elapsed().as_millis();
    Ok(output
        .results
        .into_iter()
        .enumerate()
        .map(|(index, result)| match errors.get(&index) {
            Some(error) => SqlResult::Error(SqlErrorInfo {
                sql: statements[index].clone(),
                message: error.clone(),
            }),
            None => exec_output_to_sql_result(&statements[index], result, elapsed_ms),
        })
        .collect())
}

fn validate_batch_output(
    statement_count: usize,
    output: &ExecBatchOutput,
    stop_on_error: bool,
) -> Result<(), DbError> {
    if output.results.len() > statement_count {
        return invalid_exec_batch_response("too many exec/batch results");
    }
    let expects_all = !stop_on_error || output.errors.is_empty();
    if expects_all && output.results.len() != statement_count {
        return invalid_exec_batch_response("exec/batch result count mismatch");
    }
    for error in &output.errors {
        let index = error.index as usize;
        if index >= statement_count || index >= output.results.len() {
            return invalid_exec_batch_response("exec/batch error index out of range");
        }
    }
    Ok(())
}

fn batch_errors_by_index(errors: Vec<BatchErrorOutput>) -> Result<HashMap<usize, String>, DbError> {
    let mut by_index = HashMap::new();
    for error in errors {
        if by_index
            .insert(error.index as usize, error.message.clone())
            .is_some()
        {
            return invalid_exec_batch_response("duplicate exec/batch error index");
        }
    }
    Ok(by_index)
}

fn exec_output_to_sql_result(sql: &str, output: ExecRunOutput, elapsed_ms: u128) -> SqlResult {
    SqlResult::Exec(ExecResult {
        sql: sql.to_string(),
        rows_affected: output.affected_rows,
        elapsed_ms,
        message: Some(crate::executor::format_message(sql, output.affected_rows)),
    })
}

fn invalid_exec_batch_response<T>(message: &str) -> Result<T, DbError> {
    Err(DbError::query(format!(
        "invalid external driver exec/batch response: {message}"
    )))
}

#[async_trait]
impl DbConnection for ExternalDbConnection {
    fn config(&self) -> &DbConnectionConfig {
        &self.config
    }

    fn set_config_database(&mut self, database: Option<String>) {
        self.config.database = database;
    }

    async fn connect(&mut self) -> Result<(), DbError> {
        let client = JsonRpcClient::start(&self.driver).await?;

        // 解析生效 method 集合:init 动态声明优先,manifest 静态声明回退,都无则 legacy。
        let init_methods: Vec<String> = client.session().methods.iter().cloned().collect();
        *self.method_set.lock().expect("method_set mutex poisoned") =
            MethodSet::resolve(&init_methods, &self.driver.methods);

        // conn/open
        let open_params = json!({
            "driver_id": self.driver.id,
            "config": driver_config_value(&self.config),
        });
        let open_result: ConnOpenOutput = client.request(method::CONN_OPEN, open_params).await?;

        self.set_conn_id(Some(open_result.conn_id));
        *self.client.lock().await = Some(Arc::new(client));
        Ok(())
    }

    async fn disconnect(&mut self) -> Result<(), DbError> {
        let client_arc = self.client.lock().await.take();
        if let Some(client_arc) = client_arc {
            // conn/close;允许失败(driver 端可能已断)。
            // 先把 conn_id take 出来,释放 std Mutex,再 await(避免跨 await 持锁)。
            let conn_id = self.conn_id.lock().expect("conn_id mutex").take();
            if let Some(conn_id) = conn_id {
                let _: Result<Value, DbError> = client_arc
                    .request_value(method::CONN_CLOSE, conn_only_params(conn_id))
                    .await;
            }
            // shutdown 子进程:graceful shutdown RPC + abort reader + kill child。
            client_arc.shutdown().await;
        }
        Ok(())
    }

    async fn execute(
        &self,
        plugin: &dyn DatabasePlugin,
        script: &str,
        options: ExecOptions,
    ) -> Result<Vec<SqlResult>, DbError> {
        let statements = plugin.split_sql_statements(script);
        if self.should_try_exec_batch(&statements, &options) {
            match self.exec_batch(&statements, &options, Instant::now()).await {
                Ok(results) => return Ok(results),
                Err(error) if is_method_not_found(&error) => {
                    warn!("driver does not implement exec/batch; falling back to per-statement");
                }
                Err(error) => return Err(error),
            }
        }

        let mut results = Vec::with_capacity(statements.len());
        for statement in statements {
            let result = self
                .query_with_max_rows(&statement, options.max_rows)
                .await?;
            let should_stop = options.stop_on_error && result.is_error();
            results.push(result);
            if should_stop {
                break;
            }
        }
        Ok(results)
    }

    async fn query(&self, query: &str) -> Result<SqlResult, DbError> {
        self.query_with_max_rows(query, None).await
    }

    async fn driver_request_value(
        &self,
        method_name: &str,
        mut params: Value,
    ) -> Result<Value, DbError> {
        let support = self
            .method_set
            .lock()
            .expect("method_set mutex poisoned")
            .support(method_name);
        if matches!(support, MethodSupport::Unsupported) {
            return Err(DbError::NotSupported(format!(
                "external driver does not implement `{method_name}`"
            )));
        }
        if method_requires_auto_conn_id(method_name) {
            if let Some(obj) = params.as_object_mut() {
                if !obj.contains_key("conn_id") {
                    obj.insert("conn_id".to_string(), json!(self.current_conn_id()?));
                }
            }
        }
        self.call_value(method_name, params).await
    }

    async fn ping(&self) -> Result<(), DbError> {
        let conn_id = self.current_conn_id()?;
        let _: Value = self
            .call(method::CONN_PING, conn_only_params(conn_id))
            .await?;
        Ok(())
    }

    async fn current_database(&self) -> Result<Option<String>, DbError> {
        // v2 wire 不暴露 "current_database",但宿主自己持有 config,直接返回即可。
        Ok(self.config.database.clone())
    }

    async fn switch_database(&self, database: &str) -> Result<(), DbError> {
        let conn_id = self.current_conn_id()?;
        let _: Value = self
            .call(
                method::CONN_USE,
                conn_use_params(conn_id, Some(database), None),
            )
            .await?;
        Ok(())
    }

    async fn switch_schema(&self, schema: &str) -> Result<(), DbError> {
        let conn_id = self.current_conn_id()?;
        let _: Value = self
            .call(
                method::CONN_USE,
                conn_use_params(conn_id, None, Some(schema)),
            )
            .await?;
        Ok(())
    }

    async fn execute_streaming(
        &self,
        plugin: &dyn DatabasePlugin,
        source: SqlSource,
        options: ExecOptions,
        sender: mpsc::Sender<StreamingProgress>,
    ) -> Result<(), DbError> {
        debug!(
            "[external] execute_streaming() called, transactional={}, streaming={}",
            options.transactional, options.streaming
        );

        let total_size = source.file_size().unwrap_or(0);
        let is_file_source = source.is_file();

        let mut parser = plugin
            .create_parser(source)
            .map_err(|e| DbError::query(format!("Failed to create parser: {}", e)))?;

        if options.streaming || is_file_source {
            let mut current = 0usize;

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
                debug!("[external] Streaming statement {}", current);

                let result = match self.query_with_max_rows(&sql, options.max_rows).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[external] Streaming statement {} failed: {}, SQL: {}",
                            current, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

                let is_error = result.is_error();
                let progress =
                    StreamingProgress::with_file_progress(current, result, bytes_read, total_size);
                if sender.send(progress).await.is_err() {
                    break;
                }

                if is_error && options.stop_on_error {
                    break;
                }
            }
        } else {
            let statements: Vec<String> = parser
                .filter_map(|r| r.ok())
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let total = statements.len();
            debug!("[external] Streaming {} statement(s)", total);

            if total == 0 {
                debug!("[external] No statements to execute");
                return Ok(());
            }

            for (index, sql) in statements.into_iter().enumerate() {
                let current = index + 1;
                debug!("[external] Streaming statement {}/{}", current, total);

                let result = match self.query_with_max_rows(&sql, options.max_rows).await {
                    Ok(r) => r,
                    Err(e) => {
                        let sql_preview = if sql.len() > 200 {
                            format!("{}...", truncate_str(&sql, 200))
                        } else {
                            sql.clone()
                        };
                        error!(
                            "[external] Streaming statement {}/{} failed: {}, SQL: {}",
                            current, total, e, sql_preview
                        );
                        SqlResult::Error(SqlErrorInfo {
                            sql: sql.clone(),
                            message: e.to_string(),
                        })
                    }
                };

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

        debug!("[external] execute_streaming() completed");
        Ok(())
    }
}

/// 把 wire 返回的 JSON value 包装成 `SqlResult::Query` 单元格,让
/// `ExternalDatabasePlugin` 能通过 `DbConnection` 这层 trait 解码 wire 结果。
fn wire_value_result(sql: &str, value: Value) -> SqlResult {
    SqlResult::Query(QueryResult {
        sql: sql.to_string(),
        columns: vec!["json".to_string()],
        column_meta: vec![QueryColumnMeta::new("json", "JSON")],
        rows: vec![vec![Some(value.to_string())]],
        elapsed_ms: 0,
    })
}

/// wire 响应 → 宿主结果的解码表(按 method 索引)。新增策略①方法时在此登记专属解码器,
/// 默认把整个 JSON 包成单 cell。
fn wire_result_for_method(sql: &str, method_name: &str, value: Value) -> SqlResult {
    match method_name {
        method::SQL_EXPLAIN => sql_explain_value_result(sql, value.clone())
            .unwrap_or_else(|| wire_value_result(sql, value)),
        _ => wire_value_result(sql, value),
    }
}

fn sql_explain_value_result(sql: &str, value: Value) -> Option<SqlResult> {
    let result: extension_protocol::sql::SqlExplainResult = serde_json::from_value(value).ok()?;
    Some(SqlResult::Query(QueryResult {
        sql: sql.to_string(),
        columns: vec!["explain".to_string()],
        column_meta: vec![QueryColumnMeta::new("explain", "TEXT")],
        rows: vec![vec![Some(result.content)]],
        elapsed_ms: 0,
    }))
}

/// 驱动返回的错误是否为「方法不存在」(`METHOD_NOT_FOUND` / `NotImplemented` 都映射成
/// [`DbError::NotSupported`]),用于决定是否回退到宿主 fallback。
fn is_method_not_found(error: &DbError) -> bool {
    matches!(error, DbError::NotSupported(_))
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_protocol::row::ColumnTypeKind;
    #[test]
    fn is_query_sql_handles_common_prefixes() {
        assert!(is_query_sql("SELECT 1"));
        assert!(is_query_sql("  with cte as (...)"));
        assert!(is_query_sql("EXPLAIN SELECT 1"));
        assert!(is_query_sql("show tables"));
        assert!(!is_query_sql("INSERT INTO t VALUES (1)"));
        assert!(!is_query_sql("CREATE TABLE t (x INT)"));
        assert!(!is_query_sql("DROP TABLE t"));
    }

    #[test]
    fn is_method_not_found_matches_not_supported_only() {
        assert!(is_method_not_found(&DbError::NotSupported("x".into())));
        assert!(!is_method_not_found(&DbError::NotConnected));
        assert!(!is_method_not_found(&DbError::query("boom")));
    }

    #[test]
    fn auto_conn_id_is_not_required_for_connless_methods() {
        assert!(!method_requires_auto_conn_id(
            method::DDL_BUILD_CREATE_TABLE
        ));
        assert!(!method_requires_auto_conn_id(method::DDL_BUILD_ALTER_TABLE));
        assert!(!method_requires_auto_conn_id(method::DDL_BUILD_DROP));
        assert!(!method_requires_auto_conn_id(method::CONN_TEST));
        assert!(!method_requires_auto_conn_id(method::SQL_FORMAT));
        assert!(!method_requires_auto_conn_id(method::SQL_PARSE));
    }

    #[test]
    fn auto_conn_id_is_required_for_connection_methods() {
        assert!(method_requires_auto_conn_id(method::SCHEMA_COLUMNS));
        assert!(method_requires_auto_conn_id(method::QUERY_START));
        assert!(method_requires_auto_conn_id(method::CURSOR_FETCH));
        assert!(method_requires_auto_conn_id(method::EXEC_RUN));
        assert!(method_requires_auto_conn_id(method::EXEC_BATCH));
        assert!(method_requires_auto_conn_id(method::SQL_EXPLAIN));
        assert!(method_requires_auto_conn_id(method::TX_BEGIN));
        assert!(method_requires_auto_conn_id(method::TX_COMMIT));
        assert!(method_requires_auto_conn_id(method::TX_ROLLBACK));
        assert!(method_requires_auto_conn_id(method::CONN_PING));
        assert!(method_requires_auto_conn_id(method::CONN_USE));
    }

    #[test]
    fn column_spec_to_meta_uses_type_str() {
        let spec = ColumnSpec::new("name", "VARCHAR(255)", ColumnTypeKind::Text);
        let meta = column_spec_to_meta(&spec);
        assert_eq!(meta.name, "name");
        assert_eq!(meta.db_type, "VARCHAR(255)");
    }

    #[test]
    fn column_spec_to_meta_respects_nullable() {
        let spec = ColumnSpec::new("id", "BIGINT", ColumnTypeKind::I64).nullable(false);
        let meta = column_spec_to_meta(&spec);
        assert!(!meta.nullable);
    }

    #[test]
    fn cell_to_string_handles_basic_kinds() {
        assert_eq!(cell_to_string(CellValue::Null), None);
        assert_eq!(
            cell_to_string(CellValue::Bool { value: true }),
            Some("true".to_string())
        );
        assert_eq!(
            cell_to_string(CellValue::I64 { value: 42 }),
            Some("42".to_string())
        );
        assert_eq!(
            cell_to_string(CellValue::U64 { value: u64::MAX }),
            Some(u64::MAX.to_string())
        );
        assert_eq!(
            cell_to_string(CellValue::Decimal {
                value: "1.50000".into()
            }),
            Some("1.50000".to_string())
        );
        assert_eq!(
            cell_to_string(CellValue::Text {
                value: "hello".into()
            }),
            Some("hello".to_string())
        );
        assert_eq!(
            cell_to_string(CellValue::Uuid {
                value: "550e8400-e29b-41d4-a716-446655440000".into()
            }),
            Some("550e8400-e29b-41d4-a716-446655440000".to_string())
        );
    }

    #[test]
    fn cell_to_string_decodes_bytes_to_hex() {
        // base64 of [1,2,3] = "AQID"
        let s = cell_to_string(CellValue::Bytes {
            value: "AQID".into(),
        });
        assert_eq!(s.as_deref(), Some("0x010203"));
    }

    #[test]
    fn cell_to_string_bytes_invalid_base64_returns_raw() {
        let s = cell_to_string(CellValue::Bytes {
            value: "not_base64!".into(),
        });
        assert_eq!(s.as_deref(), Some("not_base64!"));
    }

    #[test]
    fn cell_to_string_serializes_complex_kinds() {
        let s = cell_to_string(CellValue::Array {
            element_type: ColumnTypeKind::I64,
            value: vec![CellValue::I64 { value: 1 }, CellValue::I64 { value: 2 }],
        });
        assert!(s.unwrap().contains(r#""i64""#));

        let mut m = serde_json::Map::new();
        m.insert("k".into(), json!("v"));
        let s = cell_to_string(CellValue::Map { value: m });
        assert_eq!(s.as_deref(), Some(r#"{"k":"v"}"#));
    }

    #[test]
    fn row_to_strings_preserves_nulls() {
        let row = vec![
            CellValue::I64 { value: 1 },
            CellValue::Null,
            CellValue::Text { value: "x".into() },
        ];
        let strings = row_to_strings(row);
        assert_eq!(strings.len(), 3);
        assert_eq!(strings[0].as_deref(), Some("1"));
        assert!(strings[1].is_none());
        assert_eq!(strings[2].as_deref(), Some("x"));
    }

    #[test]
    fn sql_error_message_recognizes_external_driver_sql_error() {
        let err = DbError::query("external driver sql error: bad table");
        assert_eq!(
            sql_error_message(&err).as_deref(),
            Some("external driver sql error: bad table")
        );
    }

    #[test]
    fn sql_error_message_ignores_non_sql_errors() {
        assert!(sql_error_message(&DbError::NotConnected).is_none());
        assert!(sql_error_message(&DbError::connection("network down")).is_none());
    }

    #[test]
    fn wire_value_result_wraps_value_as_single_cell() {
        let v = json!({"name": "main"});
        let result = wire_value_result("SELECT", v.clone());
        match result {
            SqlResult::Query(q) => {
                assert_eq!(q.columns, vec!["json"]);
                assert_eq!(q.rows.len(), 1);
                assert_eq!(q.rows[0].len(), 1);
                assert_eq!(q.rows[0][0].as_deref(), Some(v.to_string().as_str()));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn sql_explain_wire_result_uses_content_column() {
        let result = wire_result_for_method(
            "/*onetcli-ipc-wire*/ {}",
            method::SQL_EXPLAIN,
            json!({
                "format": "text",
                "content": "physical_plan",
                "plan": null,
            }),
        );

        match result {
            SqlResult::Query(q) => {
                assert_eq!(q.columns, vec!["explain"]);
                assert_eq!(q.column_meta[0].db_type, "TEXT");
                assert_eq!(q.rows[0][0].as_deref(), Some("physical_plan"));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
