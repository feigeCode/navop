//! DuckDB 驱动接入共享运行时([`extension_driver`])。
//!
//! - [`DuckDbDriver`] 是控制面/工厂:`init` / `conn/open` / `ddl/build_*`。
//! - [`DuckDbConnection`] 是每连接执行体,独占一个 [`DuckDbSession`] 及其游标,
//!   跑在专属 worker 线程上;`interrupt_hook` 暴露 DuckDB 的跨线程中断句柄。
//!
//! 业务逻辑仍复用 [`crate::handlers`] 里的同步 handler,这里只做工厂 + 路由。

// ProtocolError 作为 Err 类型较大,协议层固定如此。
#![allow(clippy::result_large_err)]

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};

use extension_driver::{Driver, DriverConnection, InterruptHook, OpenedConnection};
use extension_protocol::conn::{ConnId, ConnOpenParams};
use extension_protocol::error::{ProtocolError, error_codes};
use extension_protocol::method;
use serde_json::{Value, json};

use crate::duckdb_session::{DbConnectionConfig, DuckDbSession};
use crate::handlers;
use crate::server::{invalid_params, params_deserialize_error, protocol_error_from_anyhow};
use crate::state::ConnectionState;

/// DuckDB 驱动控制面。`conn_id` 由本进程统一分配,保证跨连接全局唯一。
pub struct DuckDbDriver {
    initialized: AtomicI64,
    next_conn_id: AtomicU64,
}

impl DuckDbDriver {
    pub fn new() -> Self {
        Self {
            initialized: AtomicI64::new(0),
            next_conn_id: AtomicU64::new(1),
        }
    }
}

impl Default for DuckDbDriver {
    fn default() -> Self {
        Self::new()
    }
}

impl Driver for DuckDbDriver {
    fn init(&self, params: &Value) -> Result<Value, ProtocolError> {
        handlers::handle_init(&self.initialized, params)
    }

    fn open_connection(&self, params: &Value) -> Result<OpenedConnection, ProtocolError> {
        let p: ConnOpenParams =
            serde_json::from_value(params.clone()).map_err(params_deserialize_error)?;
        if p.driver_id != "duckdb" {
            return Err(invalid_params(format!(
                "unsupported driver_id `{}` (this driver only handles `duckdb`)",
                p.driver_id
            )));
        }
        let cfg: DbConnectionConfig =
            serde_json::from_value(p.config.clone()).map_err(params_deserialize_error)?;

        let mut session = DuckDbSession::new();
        session
            .connect(cfg)
            .map_err(|e| protocol_error_from_anyhow(error_codes::IO_CONNECTION_REFUSED, e))?;

        let conn_id = self.next_conn_id.fetch_add(1, Ordering::SeqCst);
        let open_result = json!({
            "conn_id": conn_id,
            "server_info": {
                "version": handlers::duckdb_version(),
                "features": ["embedded", "single_file"],
            }
        });

        Ok(OpenedConnection {
            conn_id,
            open_result,
            connection: Box::new(DuckDbConnection {
                conn_id,
                state: ConnectionState::with_conn(conn_id, session),
            }),
        })
    }

    fn call_connless(&self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::CONN_TEST => handlers::handle_conn_test(params),
            method::DDL_BUILD_CREATE_TABLE => handlers::handle_ddl_build_create_table(params),
            method::DDL_BUILD_ALTER_TABLE => handlers::handle_ddl_build_alter_table(params),
            method::DDL_BUILD_DROP => handlers::handle_ddl_build_drop(params),
            other => Err(method_not_found(other)),
        }
    }
}

/// 单个 DuckDB 连接的执行体,跑在专属 worker 线程上。
pub struct DuckDbConnection {
    conn_id: ConnId,
    state: ConnectionState,
}

impl DriverConnection for DuckDbConnection {
    fn call(&mut self, method_name: &str, params: &Value) -> Result<Value, ProtocolError> {
        match method_name {
            method::CONN_PING => handlers::handle_conn_ping(&mut self.state, params),
            method::CONN_USE => handlers::handle_conn_use(&mut self.state, params),
            method::QUERY_START => handlers::handle_query_start(&mut self.state, params),
            method::CURSOR_FETCH => handlers::handle_cursor_fetch(&mut self.state, params),
            method::CURSOR_CLOSE => handlers::handle_cursor_close(&mut self.state, params),
            method::CURSOR_CANCEL => handlers::handle_cursor_cancel(&mut self.state, params),
            method::EXEC_RUN => handlers::handle_exec_run(&mut self.state, params),
            method::EXEC_BATCH => handlers::handle_exec_batch(&mut self.state, params),
            method::TX_BEGIN => handlers::handle_tx_begin(&mut self.state, params),
            method::TX_COMMIT => handlers::handle_tx_commit(&mut self.state, params),
            method::TX_ROLLBACK => handlers::handle_tx_rollback(&mut self.state, params),
            method::DATA_EXPORT => handlers::handle_data_export(&mut self.state, params),
            method::DATA_IMPORT_BEGIN => {
                handlers::handle_data_import_begin(&mut self.state, params)
            }
            method::DATA_IMPORT_CHUNK => {
                handlers::handle_data_import_chunk(&mut self.state, params)
            }
            method::DATA_IMPORT_COMMIT => {
                handlers::handle_data_import_commit(&mut self.state, params)
            }
            method::DATA_IMPORT_ABORT => {
                handlers::handle_data_import_abort(&mut self.state, params)
            }
            method::STREAM_READ => handlers::handle_stream_read(&mut self.state, params),
            method::STREAM_CLOSE => handlers::handle_stream_close(&mut self.state, params),
            method::SCHEMA_DATABASES => handlers::handle_schema_databases(&mut self.state, params),
            method::SCHEMA_SCHEMAS => handlers::handle_schema_schemas(&mut self.state, params),
            method::SCHEMA_OBJECTS => handlers::handle_schema_objects(&mut self.state, params),
            method::SCHEMA_COLUMNS => handlers::handle_schema_columns(&mut self.state, params),
            method::SCHEMA_VIEWS => handlers::handle_schema_views(&mut self.state, params),
            method::SCHEMA_INDEXES => handlers::handle_schema_indexes(&mut self.state, params),
            method::SCHEMA_CHECKS => handlers::handle_schema_checks(&mut self.state, params),
            // ddl/build_* 是纯方法,但 host 的 call_declared_wire_method 会注入 conn_id,
            // 因而可能带 conn_id 路由到这里——一并接受。
            method::DDL_BUILD_CREATE_TABLE => handlers::handle_ddl_build_create_table(params),
            method::DDL_BUILD_ALTER_TABLE => handlers::handle_ddl_build_alter_table(params),
            method::DDL_BUILD_DROP => handlers::handle_ddl_build_drop(params),
            other => Err(method_not_found(other)),
        }
    }

    fn interrupt_hook(&self) -> Option<InterruptHook> {
        let handle = self.state.get_conn(self.conn_id)?.interrupt_handle()?;
        Some(Arc::new(move || handle.interrupt()))
    }

    fn close(&mut self) {
        if let Some(session) = self.state.get_conn_mut(self.conn_id) {
            session.disconnect();
        }
    }
}

fn method_not_found(method_name: &str) -> ProtocolError {
    ProtocolError::new(
        error_codes::METHOD_NOT_FOUND,
        format!("method `{method_name}` is not implemented in DuckDB v2 driver"),
    )
}
