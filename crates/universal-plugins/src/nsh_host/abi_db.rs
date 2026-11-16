//! `navop.db` 能力面（模块 `db`）——以 nsh 的 nativeModule 暴露。
//!
//! 页面侧入口（要求清单声明 `modules: ["db"]` + 对应 `db:*` 权限）：
//!
//! ```ts
//! const db = nativeModule<{ Db: NavopDb }>("navop").Db;
//! await db.listConnections();                              // { connections: [...] }
//! await db.openSession({ connectionId, database? });       // { sessionId }
//! await db.execute({ sessionId, sql, maxRows? });          // { columns, rows, nextCursor }
//! await db.listDatabases({ sessionId });                   // { databases: [...] }
//! await db.listSchemas({ sessionId, database });           // { schemas: [...] }
//! await db.closeSession({ sessionId });                    // { ok: true }
//! await db.refreshTree({ connectionId? });                 // { ok: true, refreshed }
//! ```
//!
//! 会话句柄对页面是不透明字符串；宿主侧表挂在 [`DbPlane`]（随页面生命周期
//! 存在，页面关了句柄即失效——句柄表随 `HostAbi` 的 Arc 丢弃）。
//!
//! SQL 的读写分级（`db:read:` / `db:write:` …）与 WASM 侧共用
//! [`extension_runtime::extension_db_gateway::ExtensionDbGateway`]：权限
//! 判定、语句分级、会话归属校验都只有一份实现。模块方法本体在
//! `native_api::Db` 里，这里只剩沿用的宿主侧实现。

use std::collections::HashMap;
use std::sync::Mutex;

use db::GlobalDbState;
use extension_component::PermissionSet;
use extension_component::protocol::{ConnectionInfo, DbError};
use extension_component::resources::DbSessionResource;
use serde_json::{Value, json};

use super::abi::HostAbi;

/// 一个页面挂载的 navop.db 状态：会话句柄表。
pub(crate) struct DbPlane {
    sessions: Mutex<HashMap<String, DbSessionResource>>,
    counter: Mutex<u64>,
}

impl DbPlane {
    pub(crate) fn new() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
            counter: Mutex::new(0),
        }
    }

    pub(crate) fn next_session_id(&self) -> String {
        let mut counter = self.counter.lock().unwrap_or_else(|e| e.into_inner());
        *counter += 1;
        format!("dbs-{counter}")
    }

    pub(crate) fn insert(&self, resource: DbSessionResource) -> String {
        let handle = self.next_session_id();
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(handle.clone(), resource);
        handle
    }

    /// 取会话的克隆（execute 等只读操作用）。
    pub(crate) fn get(&self, handle: &str) -> Option<DbSessionResource> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(handle)
            .cloned()
    }

    /// 取出会话（closeSession 用，取出后从表里移除）。
    pub(crate) fn take(&self, handle: &str) -> Option<DbSessionResource> {
        self.sessions
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(handle)
    }
}
/// 数据库网关：一次调用建一个（权限来自 HostAbi 里存的清单声明）。
pub(crate) fn gateway(
    abi: &HostAbi,
    db_state: GlobalDbState,
) -> extension_runtime::extension_db_gateway::ExtensionDbGateway {
    let permissions = PermissionSet::new(abi.permissions.iter());
    extension_runtime::extension_db_gateway::ExtensionDbGateway::new(
        abi.extension_id.clone(),
        permissions,
        db_state,
    )
}

pub(crate) fn db_failure(error: DbError) -> String {
    format!("{}: {}", error.code, error.message)
}
pub(crate) fn connections_json(connections: Vec<ConnectionInfo>) -> Value {
    json!({
        "connections": connections
            .into_iter()
            .map(|connection| {
                json!({
                    "id": connection.id,
                    "name": connection.name,
                    "driver": connection.driver,
                    "database": connection.database,
                })
            })
            .collect::<Vec<_>>(),
    })
}
pub(crate) fn batch_json(batch: extension_component::protocol::RowBatch) -> Value {
    let columns: Vec<Value> = batch
        .columns
        .iter()
        .map(|column| {
            json!({
                "name": column.name,
                "typeName": column.type_name,
                "nullable": column.nullable,
            })
        })
        .collect();
    let rows: Vec<Value> = batch
        .rows
        .into_iter()
        .map(|row| {
            Value::Array(
                row.into_iter()
                    .map(|value| match value {
                        extension_component::protocol::DbValue::Null => Value::Null,
                        extension_component::protocol::DbValue::Bool(v) => Value::Bool(v),
                        extension_component::protocol::DbValue::Integer(v) => json!(v),
                        extension_component::protocol::DbValue::Float(v) => json!(v),
                        extension_component::protocol::DbValue::Text(v) => Value::String(v),
                        extension_component::protocol::DbValue::Bytes(v) => {
                            json!(v.into_iter().map(|byte| byte as i64).collect::<Vec<_>>())
                        }
                    })
                    .collect(),
            )
        })
        .collect();
    json!({
        "columns": columns,
        "rows": rows,
        "nextCursor": batch.next_cursor,
    })
}
/// 宿主注入的“刷新已打开的 DB 树”实现（navop app 在 init 里注入）。
///
/// 树属于 `db_view`，而本 crate 不依赖它：这里只留一个函数指针，真正的
/// 枚举与刷新由宿主提供（返回实际刷新了几个连接根）。
static TREE_REFRESH: Mutex<Option<fn(&mut gpui::App, Option<&str>) -> usize>> = Mutex::new(None);

/// 宿主 init 时注入树刷新实现（与 [`set_db_state`] 同一处调用）。
pub fn set_tree_refresh_hook(hook: fn(&mut gpui::App, Option<&str>) -> usize) {
    *TREE_REFRESH.lock().unwrap_or_else(|e| e.into_inner()) = Some(hook);
}

pub(crate) fn refresh_db_trees(cx: &mut gpui::App, connection_id: Option<&str>) -> usize {
    let hook = *TREE_REFRESH.lock().unwrap_or_else(|e| e.into_inner());
    match hook {
        Some(hook) => hook(cx, connection_id),
        None => 0,
    }
}

/// 当前进程的数据库状态。
///
/// `GlobalDbState` 是 gpui 全局；navop.db 的调用都发生在 tokio 线程，
/// 不能直接读 gpui 的 global 存储。宿主在 init 时把它克隆进这里的
/// 静态槽（[`set_db_state`]）。
static DB_STATE: Mutex<Option<GlobalDbState>> = Mutex::new(None);

/// 宿主 init 时注入数据库状态（navop app 的 app_init 调一次）。
pub fn set_db_state(state: GlobalDbState) {
    *DB_STATE.lock().unwrap_or_else(|e| e.into_inner()) = Some(state);
}

pub(crate) fn current_db_state() -> Option<GlobalDbState> {
    DB_STATE.lock().unwrap_or_else(|e| e.into_inner()).clone()
}
