use serde_json::{Value, json};

pub(super) fn list_connections_schema() -> Value {
    json!({
        "type": "object",
        "properties": {}
    })
}

pub(super) fn command_schema(connection_field: &'static str) -> Value {
    json!({
        "type": "object",
        "properties": {
            connection_field: connection_property(),
            "command": {
                "type": "string",
                "description": "Single Redis command, for example `PING` or `GET user:1`."
            },
            "db": db_property()
        },
        "required": [connection_field, "command"]
    })
}

pub(super) fn keys_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection_id": connection_property(),
            "pattern": {
                "type": "string",
                "description": "Redis key pattern, for example `user:*`."
            },
            "db": db_property()
        },
        "required": ["connection_id", "pattern"]
    })
}

pub(super) fn get_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection_id": connection_property(),
            "key": {
                "type": "string",
                "description": "Redis key to read."
            },
            "db": db_property()
        },
        "required": ["connection_id", "key"]
    })
}

pub(super) fn set_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection_id": connection_property(),
            "key": {
                "type": "string",
                "description": "Redis key to write."
            },
            "value": {
                "type": "string",
                "description": "String value to store."
            },
            "db": db_property()
        },
        "required": ["connection_id", "key", "value"]
    })
}

fn connection_property() -> Value {
    json!({
        "type": "string",
        "description": "Redis connection identifier."
    })
}

fn db_property() -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "maximum": 255,
        "description": "Optional Redis logical database index."
    })
}
