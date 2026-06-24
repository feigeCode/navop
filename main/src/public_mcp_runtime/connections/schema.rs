use super::input::required_str;
use serde_json::{Value, json};
use tool_runtime::ToolError;

pub(super) fn list_kinds() -> Value {
    json!({
        "kinds": [
            { "kind": "database", "database_types": ["MySQL", "PostgreSQL", "SQLite", "DuckDB", "MSSQL", "Oracle", "ClickHouse"] },
            { "kind": "ssh_sftp" },
            { "kind": "redis" },
            { "kind": "mongodb" },
            { "kind": "serial" },
            { "kind": "port_forwarding" },
            { "kind": "rdp" },
            { "kind": "vnc" }
        ]
    })
}

pub(super) fn schema_for(input: Value) -> Result<Value, ToolError> {
    let kind = required_str(&input, "kind")?;
    let fields = match kind {
        "database" => database_schema(),
        "ssh_sftp" => ssh_schema(),
        "redis" => redis_schema(),
        "mongodb" => mongodb_schema(),
        "serial" => serial_schema(),
        "port_forwarding" => port_forwarding_schema(),
        "rdp" => remote_desktop_schema(3389),
        "vnc" => remote_desktop_schema(5900),
        other => {
            return Err(ToolError::Failed {
                message: format!("unknown connection kind: {other}"),
            });
        }
    };
    Ok(json!({ "schema_version": 1, "kind": kind, "fields": fields }))
}

fn database_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(3306)),
        field("username", "string", true, Value::Null),
        secret_field("password"),
        field("database", "string", false, Value::Null)
    ])
}

fn ssh_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(22)),
        field("username", "string", true, Value::Null),
        secret_field("password"),
        field("default_directory", "string", false, Value::Null)
    ])
}

fn redis_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(6379)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("db_index", "integer", false, json!(0))
    ])
}

fn mongodb_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("connection_string", "string", false, Value::Null),
        field("host", "string", false, Value::Null),
        field("port", "integer", false, json!(27017)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("database", "string", false, Value::Null)
    ])
}

fn serial_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("port_name", "string", true, Value::Null),
        field("baud_rate", "integer", false, json!(115200)),
        field("data_bits", "integer", false, json!(8)),
        field("stop_bits", "integer", false, json!(1)),
        enum_field("parity", &["None", "Odd", "Even"], false, json!("None")),
        enum_field(
            "flow_control",
            &["None", "Software", "Hardware"],
            false,
            json!("None"),
        )
    ])
}

fn port_forwarding_schema() -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("ssh_connection_id", "integer", true, Value::Null),
        enum_field("kind", &["Local", "Dynamic"], false, json!("Local")),
        field("bind_host", "string", false, json!("127.0.0.1")),
        field("bind_port", "integer", true, Value::Null),
        field("target_host", "string", false, Value::Null),
        field("target_port", "integer", false, Value::Null)
    ])
}

fn remote_desktop_schema(default_port: u16) -> Value {
    json!([
        field("name", "string", true, Value::Null),
        field("host", "string", true, Value::Null),
        field("port", "integer", false, json!(default_port)),
        field("username", "string", false, Value::Null),
        secret_field("password"),
        field("domain", "string", false, Value::Null),
        field("read_only", "boolean", false, json!(false))
    ])
}

fn field(name: &str, field_type: &str, required: bool, default: Value) -> Value {
    json!({ "name": name, "type": field_type, "required": required, "default": default })
}

fn enum_field(name: &str, values: &[&str], required: bool, default: Value) -> Value {
    json!({
        "name": name,
        "type": "string",
        "required": required,
        "enum": values,
        "default": default
    })
}

fn secret_field(name: &str) -> Value {
    json!({ "name": name, "type": "string", "required": false, "secret": true })
}
