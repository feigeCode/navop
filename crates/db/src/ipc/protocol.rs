use one_core::storage::DbConnectionConfig;
use serde_json::{Value, json};

pub use ipc::protocol::{JsonRpcError, JsonRpcRequest, JsonRpcResponse};

pub fn connection_config_params(config: &DbConnectionConfig) -> Value {
    json!({
        "config": {
            "id": config.id,
            "database_type": config.database_type.as_str(),
            "name": config.name,
            "host": config.host,
            "port": config.port,
            "username": config.username,
            "password": config.password,
            "database": config.database,
            "service_name": config.service_name,
            "sid": config.sid,
            "extra_params": config.extra_params,
        }
    })
}

pub fn empty_params() -> Value {
    json!({})
}

pub fn sql_params(sql: &str) -> Value {
    json!({ "sql": sql })
}

pub fn database_params(database: &str) -> Value {
    json!({ "database": database })
}

pub fn schema_params(schema: &str) -> Value {
    json!({ "schema": schema })
}

pub fn table_metadata_params(database: &str, schema: Option<String>, table: &str) -> Value {
    json!({ "database": database, "schema": schema, "table": table })
}

pub fn database_metadata_params(database: &str, schema: Option<String>) -> Value {
    json!({ "database": database, "schema": schema })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_json_rpc_request() {
        let request = JsonRpcRequest::new(7, "ping", empty_params());
        let value = serde_json::to_value(request).unwrap();
        assert_eq!(value["jsonrpc"], "2.0");
        assert_eq!(value["id"], 7);
        assert_eq!(value["method"], "ping");
    }
}
