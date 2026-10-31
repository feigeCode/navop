use serde_json::{Value, json};

pub(super) const MAX_DB_INDEX: u64 = 255;

pub(super) fn command_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "command": {
                "type": "string",
                "description": "Single Redis command, for example `PING` or `GET user:1`."
            },
            "db": db_property()
        },
        "required": ["connection", "command"]
    })
}

pub(super) fn keys_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "pattern": {
                "type": "string",
                "description": "Redis key pattern, for example `user:*`."
            },
            "db": db_property()
        },
        "required": ["connection", "pattern"]
    })
}

pub(super) fn get_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "key": {
                "type": "string",
                "description": "Redis key to read."
            },
            "db": db_property()
        },
        "required": ["connection", "key"]
    })
}

pub(super) fn set_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
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
        "required": ["connection", "key", "value"]
    })
}

fn connection_property() -> Value {
    json!({
        "type": "string",
        "description": "Saved Redis connection id or exact saved connection name."
    })
}

fn db_property() -> Value {
    json!({
        "type": "integer",
        "minimum": 0,
        "maximum": MAX_DB_INDEX,
        "description": "Optional Redis logical database index."
    })
}
