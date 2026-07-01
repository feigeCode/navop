use serde_json::{Value, json};

pub(super) fn path_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "path": {
                "type": "string",
                "description": "Remote path. Defaults to `.`."
            }
        }
    })
}

pub(super) fn read_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "path": {
                "type": "string",
                "description": "Remote file path. Defaults to `.`."
            },
            "max_bytes": {
                "type": "integer",
                "minimum": 1,
                "maximum": 1048576,
                "default": 1048576
            }
        }
    })
}

pub(super) fn write_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": optional_connection_property(),
            "path": {
                "type": "string",
                "description": "Remote file path to write."
            },
            "content_base64": {
                "type": "string",
                "description": "Base64 encoded file content."
            },
            "on_exists": {
                "type": "string",
                "enum": ["fail", "overwrite", "skip"],
                "default": "fail"
            }
        },
        "required": ["path", "content_base64"]
    })
}

fn optional_connection_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional SSH/SFTP connection id. Defaults to the current Agent SSH resource id."
    })
}
