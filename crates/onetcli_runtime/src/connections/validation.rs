use super::build::redacted_values;
use serde_json::{Value, json};

pub(super) fn validate(input: Value) -> Value {
    let missing = missing_required(&input);
    let invalid = invalid_fields(&input);
    let ok = missing.is_empty() && invalid.is_empty();
    json!({
        "ok": ok,
        "can_apply": ok,
        "missing_required": missing,
        "invalid_fields": invalid,
        "redacted_preview": redacted_values(input.get("values").unwrap_or(&Value::Null)),
    })
}

fn missing_required(input: &Value) -> Vec<String> {
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let values = input.get("values").unwrap_or(&Value::Null);
    let required = match kind {
        "database" => database_required(input),
        "ssh_sftp" => vec!["name", "host", "username"],
        "redis" => vec!["name", "host"],
        "mongodb" => mongodb_required(values),
        "serial" => vec!["name", "port_name"],
        "port_forwarding" => port_forwarding_required(values),
        "rdp" | "vnc" => vec!["name", "host"],
        _ => vec!["kind"],
    };
    required
        .into_iter()
        .filter(|field| value_is_empty(values.get(*field)))
        .map(str::to_string)
        .collect()
}

fn database_required(input: &Value) -> Vec<&'static str> {
    match input.get("database_type").and_then(Value::as_str) {
        Some("SQLite" | "DuckDB") => vec!["name", "host"],
        _ => vec!["name", "host", "username"],
    }
}

fn mongodb_required(values: &Value) -> Vec<&'static str> {
    if values
        .get("connection_string")
        .and_then(Value::as_str)
        .is_some_and(|value| !value.trim().is_empty())
    {
        vec!["name", "connection_string"]
    } else {
        vec!["name", "host"]
    }
}

fn port_forwarding_required(values: &Value) -> Vec<&'static str> {
    match values.get("kind").and_then(Value::as_str) {
        Some("Dynamic") | Some("dynamic") => vec!["name", "ssh_connection_id", "bind_port"],
        _ => vec![
            "name",
            "ssh_connection_id",
            "bind_port",
            "target_host",
            "target_port",
        ],
    }
}

fn value_is_empty(value: Option<&Value>) -> bool {
    match value {
        Some(Value::String(value)) => value.trim().is_empty(),
        Some(Value::Null) | None => true,
        _ => false,
    }
}

fn invalid_fields(input: &Value) -> Vec<Value> {
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or_default();
    let values = input.get("values").unwrap_or(&Value::Null);
    let mut invalid = Vec::new();
    match kind {
        "database" | "ssh_sftp" => add_invalid_u16(values, "port", &mut invalid),
        "mongodb" => {
            add_invalid_u16(values, "port", &mut invalid);
            add_invalid_enum(
                values,
                "driver_variant",
                &["modern", "legacy"],
                &mut invalid,
            );
        }
        "redis" => {
            add_invalid_u16(values, "port", &mut invalid);
            add_invalid_u8(values, "db_index", &mut invalid);
        }
        "serial" => {
            add_invalid_u32(values, "baud_rate", &mut invalid);
            add_invalid_range(values, "data_bits", 5, 8, &mut invalid);
            add_invalid_range(values, "stop_bits", 1, 2, &mut invalid);
            add_invalid_enum(values, "parity", &["None", "Odd", "Even"], &mut invalid);
            add_invalid_enum(
                values,
                "flow_control",
                &["None", "Software", "Hardware"],
                &mut invalid,
            );
        }
        "port_forwarding" => {
            add_invalid_i64(values, "ssh_connection_id", &mut invalid);
            add_invalid_u16(values, "bind_port", &mut invalid);
            add_invalid_u16(values, "target_port", &mut invalid);
            add_invalid_enum(values, "kind", &["Local", "Dynamic"], &mut invalid);
        }
        "rdp" | "vnc" => add_invalid_u16(values, "port", &mut invalid),
        _ => {}
    }
    invalid
}

fn add_invalid_u32(values: &Value, field: &'static str, invalid: &mut Vec<Value>) {
    add_invalid_unsigned(values, field, u32::MAX as u64, invalid);
}

fn add_invalid_u16(values: &Value, field: &'static str, invalid: &mut Vec<Value>) {
    add_invalid_unsigned(values, field, u16::MAX as u64, invalid);
}

fn add_invalid_u8(values: &Value, field: &'static str, invalid: &mut Vec<Value>) {
    add_invalid_unsigned(values, field, u8::MAX as u64, invalid);
}

fn add_invalid_unsigned(values: &Value, field: &'static str, max: u64, invalid: &mut Vec<Value>) {
    let Some(value) = values.get(field) else {
        return;
    };
    if value.as_u64().is_some_and(|value| value <= max) {
        return;
    }
    invalid.push(json!({
        "field": field,
        "message": format!("must be an integer between 0 and {max}")
    }));
}

fn add_invalid_i64(values: &Value, field: &'static str, invalid: &mut Vec<Value>) {
    let Some(value) = values.get(field) else {
        return;
    };
    if value.as_i64().is_some() {
        return;
    }
    invalid.push(json!({ "field": field, "message": "must be an integer" }));
}

fn add_invalid_range(
    values: &Value,
    field: &'static str,
    min: u64,
    max: u64,
    invalid: &mut Vec<Value>,
) {
    let Some(value) = values.get(field) else {
        return;
    };
    if value
        .as_u64()
        .is_some_and(|value| (min..=max).contains(&value))
    {
        return;
    }
    invalid.push(json!({
        "field": field,
        "message": format!("must be an integer between {min} and {max}")
    }));
}

fn add_invalid_enum(
    values: &Value,
    field: &'static str,
    allowed: &[&'static str],
    invalid: &mut Vec<Value>,
) {
    let Some(value) = values.get(field) else {
        return;
    };
    if value
        .as_str()
        .is_some_and(|value| allowed.iter().any(|allowed| allowed == &value))
    {
        return;
    }
    invalid.push(
        json!({ "field": field, "message": format!("must be one of {}", allowed.join(", ")) }),
    );
}
