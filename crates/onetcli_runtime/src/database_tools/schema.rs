use serde_json::{Value, json};
use tool_runtime::ToolAnnotations;

use super::DatabaseTool;

pub(super) fn descriptor_parts(
    tool: DatabaseTool,
) -> (
    &'static str,
    &'static str,
    &'static str,
    Value,
    ToolAnnotations,
) {
    match tool {
        DatabaseTool::Schema => (
            "db.schema",
            "Read database schema",
            "Read schema-level metadata for a saved database connection. The connection argument accepts a saved connection id or exact saved connection name.",
            connection_schema(),
            ToolAnnotations::read_only("Read database schema"),
        ),
        DatabaseTool::Query => (
            "db.query",
            "Run database query",
            "Run read-only SQL through a saved database connection. Non-query statements are rejected before execution; use db.exec for write-capable SQL.",
            query_schema(),
            ToolAnnotations::read_only("Run database query"),
        ),
        DatabaseTool::Exec => (
            "db.exec",
            "Execute database script",
            "Execute a SQL script or SQL file through a saved database connection. This may mutate database state and requires --allow-write when called through onetcli tool call.",
            exec_schema(),
            ToolAnnotations::mutating("Execute database script"),
        ),
    }
}

fn connection_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "database": database_property(),
            "schema": schema_property()
        },
        "required": ["connection"]
    })
}

fn query_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "database": database_property(),
            "schema": schema_property(),
            "sql": { "type": "string", "description": "SQL query text to run." }
        },
        "required": ["connection", "sql"]
    })
}

fn exec_schema() -> Value {
    json!({
        "type": "object",
        "properties": {
            "connection": connection_property(),
            "database": database_property(),
            "schema": schema_property(),
            "file": { "type": "string", "description": "SQL file path to execute." },
            "sql": { "type": "string", "description": "SQL script text to execute." }
        },
        "required": ["connection"]
    })
}

fn connection_property() -> Value {
    json!({
        "type": "string",
        "description": "Saved database connection id or exact saved connection name."
    })
}

fn database_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional database/catalog name to use for this call."
    })
}

fn schema_property() -> Value {
    json!({
        "type": "string",
        "description": "Optional schema name to narrow metadata or SQL context."
    })
}
