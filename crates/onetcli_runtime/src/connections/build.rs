use super::extended_build;
use super::input::{
    optional_bool, optional_i64, optional_str, optional_u16, optional_u64, optional_value_str,
    required_object, required_str, required_value_str, tool_error,
};
use one_core::storage::{
    ConnectionType, DatabaseType, DbConnectionConfig, MongoDBParams, RedisMode, RedisParams,
    SshAuthMethod, SshParams, StoredConnection,
};
use serde_json::{Map, Value, json};
use std::collections::HashMap;
use tool_runtime::ToolError;

pub(super) fn build_connection(input: &Value) -> Result<StoredConnection, ToolError> {
    match required_str(input, "kind")? {
        "database" => build_database(input),
        "ssh_sftp" => build_ssh(input),
        "redis" => build_redis(input),
        "mongodb" => build_mongodb(input),
        "serial" => extended_build::build_serial(input),
        "port_forwarding" => extended_build::build_port_forwarding(input),
        "rdp" => extended_build::build_remote_desktop(input, "rdp"),
        "vnc" => extended_build::build_remote_desktop(input, "vnc"),
        other => Err(ToolError::Failed {
            message: format!("unknown connection kind: {other}"),
        }),
    }
}

fn build_database(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let database_type =
        parse_database_type(optional_str(input, "database_type").unwrap_or("MySQL"))?;
    let name = required_value_str(values, "name")?.to_string();
    let config = DbConnectionConfig {
        id: String::new(),
        database_type: database_type.clone(),
        name: name.clone(),
        host: required_value_str(values, "host")?.to_string(),
        port: optional_u16(values, "port").unwrap_or_else(|| default_database_port(&database_type)),
        username: optional_value_str(values, "username")
            .unwrap_or_default()
            .to_string(),
        password: optional_value_str(values, "password")
            .unwrap_or_default()
            .to_string(),
        database: optional_value_str(values, "database").map(str::to_string),
        service_name: optional_value_str(values, "service_name").map(str::to_string),
        sid: optional_value_str(values, "sid").map(str::to_string),
        workspace_id: optional_i64(input, "workspace_id"),
        proxy: None,
        credential_reference: None,
        extra_params: database_extra_params(values),
    };
    Ok(with_common_fields(
        StoredConnection::new_database(name, config, optional_i64(input, "workspace_id")),
        input,
    ))
}

fn build_ssh(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let password = optional_value_str(values, "password").map(str::to_string);
    let params = SshParams {
        sftp_default_directory: None,
        disabled_jump_server: None,
        sftp_account: None,
        host: required_value_str(values, "host")?.to_string(),
        port: optional_u16(values, "port").unwrap_or(22),
        username: required_value_str(values, "username")?.to_string(),
        auth_method: password.map_or(SshAuthMethod::AutoPublicKey, |password| {
            SshAuthMethod::Password { password }
        }),
        prompt_username: None,
        prompt_password: None,
        keyboard_interactive: None,
        terminal_encoding: Default::default(),
        terminal_type: Default::default(),
        connect_timeout: optional_u64(values, "connect_timeout"),
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: optional_value_str(values, "default_directory").map(str::to_string),
        init_script: None,
        disable_shell_integration: None,
        x11_forwarding: None,
        allow_legacy_algorithms: None,
        jump_server: None,
        proxy: None,
        credential_reference: None,
        os_id: None,
        icon: None,
        icon_file_path: None,
        account_expect: Default::default(),
    };
    Ok(with_common_fields(
        StoredConnection::new_ssh(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

fn build_redis(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let params = RedisParams {
        host: required_value_str(values, "host")?.to_string(),
        port: optional_u16(values, "port").unwrap_or(6379),
        password: optional_value_str(values, "password").map(str::to_string),
        username: optional_value_str(values, "username").map(str::to_string),
        db_index: optional_u64(values, "db_index").unwrap_or(0) as u8,
        mode: RedisMode::Standalone,
        use_tls: optional_bool(values, "use_tls").unwrap_or(false),
        connect_timeout: optional_u64(values, "connect_timeout"),
        sentinel: None,
        cluster: None,
        ssh_tunnel: None,
        credential_reference: None,
    };
    Ok(with_common_fields(
        StoredConnection::new_redis(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

fn build_mongodb(input: &Value) -> Result<StoredConnection, ToolError> {
    let values = required_object(input, "values")?;
    let params = MongoDBParams {
        driver_variant: match optional_value_str(values, "driver_variant").unwrap_or("modern") {
            "modern" => one_core::storage::MongoDriverVariant::Modern,
            "legacy" => one_core::storage::MongoDriverVariant::Legacy,
            "legacy_3_2" => one_core::storage::MongoDriverVariant::Legacy32,
            value => {
                return Err(tool_error(format!(
                    "unsupported MongoDB driver_variant `{value}`"
                )));
            }
        },
        connection_string: optional_value_str(values, "connection_string")
            .unwrap_or_default()
            .to_string(),
        host: optional_value_str(values, "host")
            .unwrap_or_default()
            .to_string(),
        port: optional_u16(values, "port"),
        database: optional_value_str(values, "database").map(str::to_string),
        username: optional_value_str(values, "username").map(str::to_string),
        password: optional_value_str(values, "password").map(str::to_string),
        auth_source: optional_value_str(values, "auth_source").map(str::to_string),
        replica_set: optional_value_str(values, "replica_set").map(str::to_string),
        read_preference: optional_value_str(values, "read_preference").map(str::to_string),
        use_srv_record: optional_bool(values, "use_srv_record").unwrap_or(false),
        direct_connection: optional_bool(values, "direct_connection").unwrap_or(false),
        use_tls: optional_bool(values, "use_tls").unwrap_or(false),
        connect_timeout_seconds: optional_u64(values, "connect_timeout_seconds"),
        application_name: optional_value_str(values, "application_name").map(str::to_string),
        ssh_tunnel: None,
        credential_reference: None,
    };
    Ok(with_common_fields(
        StoredConnection::new_mongodb(
            required_value_str(values, "name")?.to_string(),
            params,
            optional_i64(input, "workspace_id"),
        ),
        input,
    ))
}

fn with_common_fields(mut connection: StoredConnection, input: &Value) -> StoredConnection {
    connection.remark = optional_str(input, "remark").map(str::to_string);
    if let Some(sync_enabled) = optional_bool(input, "sync_enabled") {
        connection.sync_enabled = sync_enabled;
    }
    connection.team_id = optional_str(input, "team_id").map(str::to_string);
    connection
}

pub(super) fn connection_summary_with_options(
    connection: &StoredConnection,
    workspace_name: Option<&str>,
    include_summary: bool,
) -> Result<Value, ToolError> {
    let params: Value = serde_json::from_str(&connection.params).map_err(tool_error)?;
    let mut summary = Map::from_iter([
        ("id".to_string(), json!(connection.id)),
        ("name".to_string(), json!(connection.name)),
        (
            "kind".to_string(),
            json!(mcp_kind(connection.connection_type)),
        ),
        ("workspace_id".to_string(), json!(connection.workspace_id)),
        ("workspace_name".to_string(), json!(workspace_name)),
        ("remark".to_string(), json!(connection.remark)),
        ("sync_enabled".to_string(), json!(connection.sync_enabled)),
        ("team_id".to_string(), json!(connection.team_id)),
    ]);
    if include_summary {
        summary.insert("summary".to_string(), redacted_values(&params));
        add_database_summary(&mut summary, &params);
    }
    Ok(Value::Object(summary))
}

pub(super) fn mcp_kind(connection_type: ConnectionType) -> &'static str {
    match connection_type {
        ConnectionType::Database => "database",
        ConnectionType::SshSftp => "ssh_sftp",
        ConnectionType::Redis => "redis",
        ConnectionType::MongoDB => "mongodb",
        ConnectionType::Serial => "serial",
        ConnectionType::PortForwarding => "port_forwarding",
        ConnectionType::Rdp => "rdp",
        ConnectionType::Vnc => "vnc",
        _ => "unsupported",
    }
}

pub(super) fn database_extra_params(values: &Value) -> HashMap<String, String> {
    values
        .as_object()
        .into_iter()
        .flat_map(|object| object.iter())
        .filter(|(key, _)| !database_core_field(key))
        .map(|(key, value)| (key.clone(), extra_param_string(value)))
        .collect()
}

pub(super) fn database_core_field(key: &str) -> bool {
    matches!(
        key,
        "name" | "host" | "port" | "username" | "password" | "database" | "service_name" | "sid"
    )
}

pub(super) fn extra_param_string(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn add_database_summary(summary: &mut Map<String, Value>, params: &Value) {
    let Ok(config) = serde_json::from_value::<DbConnectionConfig>(params.clone()) else {
        return;
    };
    summary.insert(
        "database_type".to_string(),
        json!(config.database_type.storage_key()),
    );
    summary.insert(
        "stored_extra_params".to_string(),
        redacted_values(&json!(config.extra_params)),
    );
    summary.insert(
        "effective_values".to_string(),
        redacted_values(&database_effective_values(&config)),
    );
}

fn database_effective_values(config: &DbConnectionConfig) -> Value {
    let mut values = Map::new();
    for (key, value) in &config.extra_params {
        values.insert(key.clone(), json!(value));
    }
    values
        .entry("connect_timeout".to_string())
        .or_insert_with(|| json!("30"));
    if config.database_type == DatabaseType::MSSQL {
        values
            .entry("encrypt".to_string())
            .or_insert_with(|| json!("off"));
        values
            .entry("trust_cert".to_string())
            .or_insert_with(|| json!("false"));
    }
    Value::Object(values)
}

pub(super) fn redacted_values(value: &Value) -> Value {
    match value {
        Value::Object(object) => Value::Object(
            object
                .iter()
                .map(|(key, value)| {
                    if secret_key(key) {
                        (key.clone(), Value::String("<redacted>".to_string()))
                    } else {
                        (key.clone(), redacted_values(value))
                    }
                })
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(redacted_values).collect()),
        value => value.clone(),
    }
}

fn secret_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase();
    key.contains("password") || key.contains("passphrase") || key.contains("secret")
}

fn parse_database_type(value: &str) -> Result<DatabaseType, ToolError> {
    DatabaseType::from_storage_key(value).ok_or_else(|| ToolError::Failed {
        message: format!("unknown database type: {value}"),
    })
}

fn default_database_port(database_type: &DatabaseType) -> u16 {
    match database_type {
        DatabaseType::PostgreSQL => 5432,
        DatabaseType::SQLite | DatabaseType::DuckDB => 0,
        DatabaseType::MSSQL => 1433,
        DatabaseType::Oracle => 1521,
        DatabaseType::ClickHouse => 8123,
        // TDengine 经 taosAdapter 的 WebSocket 端口。
        DatabaseType::TDengine => 6041,
        _ => 3306,
    }
}
