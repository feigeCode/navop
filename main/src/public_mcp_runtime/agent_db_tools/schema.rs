use agent_runtime::ToolError;
use serde_json::{Value, json};

pub(super) const DEFAULT_QUERY_MAX_ROWS: usize = 100;
const MAX_QUERY_ROWS: usize = 500;
pub(super) const DEFAULT_SAMPLE_ROWS: usize = 10;
const MAX_SAMPLE_ROWS: usize = 50;

pub(super) fn connection_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
        }
    })
}

pub(super) fn scoped_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "database": optional_database_property(),
            "schema": optional_schema_property(),
        }
    })
}

pub(super) fn query_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "database": optional_database_property(),
            "schema": optional_schema_property(),
            "sql": {"type": "string", "description": "Read-only SQL query text."},
            "max_rows": {"type": "integer", "minimum": 1, "maximum": MAX_QUERY_ROWS, "default": DEFAULT_QUERY_MAX_ROWS},
        },
        "required": ["sql"]
    })
}

pub(super) fn execute_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "database": optional_database_property(),
            "schema": optional_schema_property(),
            "sql": {"type": "string", "description": "SQL script text to execute after user approval."},
        },
        "required": ["sql"]
    })
}

pub(super) fn table_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "database": optional_database_property(),
            "schema": optional_schema_property(),
            "table": {"type": "string", "description": "Table name to inspect."},
        },
        "required": ["table"]
    })
}

pub(super) fn sample_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "database": optional_database_property(),
            "schema": optional_schema_property(),
            "table": {"type": "string", "description": "Table name to sample."},
            "limit": {"type": "integer", "minimum": 1, "maximum": MAX_SAMPLE_ROWS, "default": DEFAULT_SAMPLE_ROWS},
        },
        "required": ["table"]
    })
}

pub(super) fn optional_str(input: &Value, key: &str) -> Result<Option<String>, ToolError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(value)) => Ok(Some(value.clone())),
        _ => Err(ToolError::InvalidArguments(format!(
            "field `{key}` must be a string"
        ))),
    }
}

pub(super) fn bounded_query_rows(input: &Value, key: &str) -> Result<usize, ToolError> {
    bounded_usize(input, key, DEFAULT_QUERY_MAX_ROWS, MAX_QUERY_ROWS)
}

pub(super) fn bounded_sample_rows(input: &Value, key: &str) -> Result<usize, ToolError> {
    bounded_usize(input, key, DEFAULT_SAMPLE_ROWS, MAX_SAMPLE_ROWS)
}

fn bounded_usize(input: &Value, key: &str, default: usize, max: usize) -> Result<usize, ToolError> {
    match input.get(key) {
        None | Some(Value::Null) => Ok(default),
        Some(Value::Number(value)) => {
            let Some(value) = value.as_u64() else {
                return Err(ToolError::InvalidArguments(format!(
                    "field `{key}` must be a positive integer"
                )));
            };
            Ok((value as usize).clamp(1, max))
        }
        _ => Err(ToolError::InvalidArguments(format!(
            "field `{key}` must be an integer"
        ))),
    }
}

fn optional_connection_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional database connection id. Defaults to the current Agent database resource id."
    })
}

fn optional_database_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional database/catalog name. Defaults to the current Agent database scope."
    })
}

fn optional_schema_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional schema name. Defaults to the current Agent schema scope."
    })
}
