//! Wire 协议参数构造助手。
//!
//! 这一层把 `DbConnectionConfig` 等业务结构翻译成 v2 wire 期望的 JSON `params`,
//! 让 `connection.rs` / `plugin.rs` 可以一行调用 wire 方法。
//!
//! v2 wire 方法常量统一从 `extension_protocol::method` re-export 出来,避免业务
//! 层硬编码字面量(打错字编译都过)。
//!
use extension_protocol::conn::ConnId;
use extension_protocol::schema::ObjectKind;
use one_core::storage::DbConnectionConfig;
use serde_json::{Value, json};

/// re-export wire 方法常量,业务层走它们(避免直接 `import extension_protocol::method`)。
pub use extension_protocol::method;

/// 构造 `conn/open` 的 `config` 字段。
///
/// driver 子进程读到这份 JSON 后自行反序列化为自己内部的 config struct
/// (例如 `duckdb_driver::DbConnectionConfig` 只关心 host / database / extra_params)。
pub fn driver_config_value(config: &DbConnectionConfig) -> Value {
    json!({
        "id": config.id,
        "database_type": config.database_type.as_str(),
        "database_type_key": config.database_type.as_str(),
        "driver_id": config.get_param(crate::ipc::registry::EXTERNAL_DRIVER_ID_PARAM),
        "name": config.name,
        "host": config.host,
        "port": config.port,
        "username": config.username,
        "password": config.password,
        "database": config.database,
        "service_name": config.service_name,
        "sid": config.sid,
        "extra_params": config.extra_params,
    })
}

/// `query/start` 参数。
pub fn query_start_params(conn_id: ConnId, sql: &str, max_rows: Option<u64>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    obj.insert("sql".into(), json!(sql));
    if let Some(max_rows) = max_rows {
        obj.insert("max_rows".into(), json!(max_rows));
    }
    Value::Object(obj)
}

/// `exec/run` 参数。
pub fn exec_run_params(conn_id: ConnId, sql: &str) -> Value {
    json!({ "conn_id": conn_id, "sql": sql })
}

/// `exec/batch` 参数。
pub fn exec_batch_params(
    conn_id: ConnId,
    statements: &[String],
    stop_on_error: bool,
    in_transaction: bool,
) -> Value {
    json!({
        "conn_id": conn_id,
        "statements": statements,
        "stop_on_error": stop_on_error,
        "in_transaction": in_transaction,
    })
}

/// `cursor/fetch` 参数。
///
/// 带上 `conn_id`:driver 运行时按 `conn_id` 把请求路由到拥有该游标的连接 worker。
/// driver 端解析 `CursorFetchParams` 时会忽略多出的 `conn_id` 字段。
pub fn cursor_fetch_params(conn_id: ConnId, cursor_id: &str, n: Option<u32>) -> Value {
    match n {
        Some(n) => json!({ "conn_id": conn_id, "cursor_id": cursor_id, "n": n }),
        None => json!({ "conn_id": conn_id, "cursor_id": cursor_id }),
    }
}

/// `cursor/close` 参数(同样带 `conn_id` 供运行时路由)。
pub fn cursor_close_params(conn_id: ConnId, cursor_id: &str) -> Value {
    json!({ "conn_id": conn_id, "cursor_id": cursor_id })
}

/// `conn/ping` / `conn/close` 参数。
pub fn conn_only_params(conn_id: ConnId) -> Value {
    json!({ "conn_id": conn_id })
}

/// `conn/use` 参数。
pub fn conn_use_params(conn_id: ConnId, database: Option<&str>, schema: Option<&str>) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    if let Some(db) = database {
        obj.insert("database".into(), json!(db));
    }
    if let Some(s) = schema {
        obj.insert("schema".into(), json!(s));
    }
    Value::Object(obj)
}

/// Legacy metadata helper retained for current plugin call sites during v2 migration.
pub fn database_metadata_params(database: &str, schema: Option<String>) -> Value {
    json!({ "database": database, "schema": schema })
}

/// Legacy metadata helper retained for current plugin call sites during v2 migration.
pub fn table_metadata_params(database: &str, schema: Option<String>, table: &str) -> Value {
    json!({ "database": database, "schema": schema, "table": table })
}

/// `schema/databases` 参数。
pub fn schema_databases_params(conn_id: ConnId) -> Value {
    json!({ "conn_id": conn_id })
}

/// `schema/schemas` 参数。
pub fn schema_schemas_params(conn_id: ConnId, database: &str) -> Value {
    json!({ "conn_id": conn_id, "database": database })
}

/// `schema/objects` 参数。
pub fn schema_objects_params(
    conn_id: ConnId,
    database: Option<&str>,
    schema: Option<&str>,
    kinds: &[ObjectKind],
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    if let Some(db) = database {
        obj.insert("database".into(), json!(db));
    }
    if let Some(s) = schema {
        obj.insert("schema".into(), json!(s));
    }
    if !kinds.is_empty() {
        obj.insert(
            "kinds".into(),
            Value::Array(kinds.iter().map(|k| json!(k.as_str())).collect()),
        );
    }
    Value::Object(obj)
}

/// `schema/columns` 参数。
pub fn schema_columns_params(
    conn_id: ConnId,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    if let Some(db) = database {
        obj.insert("database".into(), json!(db));
    }
    if let Some(s) = schema {
        obj.insert("schema".into(), json!(s));
    }
    obj.insert("table".into(), json!(table));
    Value::Object(obj)
}

/// `schema/indexes` / `schema/foreign_keys` 参数(同 shape)。
pub fn schema_table_scoped_params(
    conn_id: ConnId,
    database: Option<&str>,
    schema: Option<&str>,
    table: &str,
) -> Value {
    schema_columns_params(conn_id, database, schema, table)
}

/// `schema/views` / `schema/functions` / `schema/procedures` / `schema/sequences` 参数。
pub fn schema_db_scoped_params(
    conn_id: ConnId,
    database: Option<&str>,
    schema: Option<&str>,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    if let Some(db) = database {
        obj.insert("database".into(), json!(db));
    }
    if let Some(s) = schema {
        obj.insert("schema".into(), json!(s));
    }
    Value::Object(obj)
}

/// `schema/triggers` 参数(可指定 table 过滤)。
pub fn schema_triggers_params(
    conn_id: ConnId,
    database: Option<&str>,
    schema: Option<&str>,
    table: Option<&str>,
) -> Value {
    let mut obj = serde_json::Map::new();
    obj.insert("conn_id".into(), json!(conn_id));
    if let Some(db) = database {
        obj.insert("database".into(), json!(db));
    }
    if let Some(s) = schema {
        obj.insert("schema".into(), json!(s));
    }
    if let Some(t) = table {
        obj.insert("table".into(), json!(t));
    }
    Value::Object(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_databases_only_carries_conn_id() {
        let v = schema_databases_params(7);
        assert_eq!(v, json!({"conn_id": 7}));
    }

    #[test]
    fn schema_objects_kinds_serialize_as_snake_case() {
        let v = schema_objects_params(
            17,
            Some("db1"),
            None,
            &[ObjectKind::Table, ObjectKind::View],
        );
        assert_eq!(v["kinds"], json!(["table", "view"]));
        assert_eq!(v["database"], json!("db1"));
        assert!(v.get("schema").is_none());
    }

    #[test]
    fn schema_columns_requires_table() {
        let v = schema_columns_params(1, None, None, "users");
        assert_eq!(v["table"], json!("users"));
        assert_eq!(v["conn_id"], json!(1));
    }

    #[test]
    fn conn_use_omits_empty_fields() {
        let v = conn_use_params(2, None, None);
        assert_eq!(v, json!({"conn_id": 2}));
        let v2 = conn_use_params(2, Some("db"), Some("public"));
        assert_eq!(v2["database"], json!("db"));
        assert_eq!(v2["schema"], json!("public"));
    }

    #[test]
    fn cursor_fetch_with_and_without_n() {
        assert_eq!(
            cursor_fetch_params(3, "c-1", Some(500)),
            json!({"conn_id": 3, "cursor_id": "c-1", "n": 500})
        );
        assert_eq!(
            cursor_fetch_params(3, "c-1", None),
            json!({"conn_id": 3, "cursor_id": "c-1"})
        );
    }

    #[test]
    fn cursor_close_carries_conn_id() {
        assert_eq!(
            cursor_close_params(3, "c-1"),
            json!({"conn_id": 3, "cursor_id": "c-1"})
        );
    }
}
