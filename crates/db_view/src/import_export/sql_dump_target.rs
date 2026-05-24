use db::{DbNode, DbNodeType};

const DUMP_FILENAME_FALLBACK: &str = "dump";
const MAX_DUMP_FILENAME_COMPONENT_CHARS: usize = 120;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SqlDumpTarget {
    pub(crate) database: String,
    pub(crate) schema: Option<String>,
    pub(crate) table: Option<String>,
}

pub(crate) fn resolve_sql_dump_target(node: &DbNode) -> Option<SqlDumpTarget> {
    match node.node_type {
        DbNodeType::Database => Some(SqlDumpTarget {
            database: node.name.clone(),
            schema: None,
            table: None,
        }),
        DbNodeType::Schema => Some(SqlDumpTarget {
            database: non_empty(node.get_database_name())?,
            schema: non_empty(node.get_schema_name()),
            table: None,
        }),
        DbNodeType::Table => Some(SqlDumpTarget {
            database: non_empty(node.get_database_name())?,
            schema: non_empty(node.get_schema_name()),
            table: Some(node.name.clone()),
        }),
        _ => None,
    }
}

pub(crate) fn sql_dump_filename(
    database: &str,
    schema: Option<&str>,
    table: Option<&str>,
    timestamp: &str,
) -> String {
    let mut parts = vec![sanitize_dump_filename_component(database)];
    if let Some(schema) = schema {
        parts.push(sanitize_dump_filename_component(schema));
    }
    if let Some(table) = table {
        parts.push(sanitize_dump_filename_component(table));
    }
    parts.push(sanitize_dump_filename_component(timestamp));
    format!("{}.sql", parts.join("_"))
}

pub(crate) fn sanitize_dump_filename_component(value: &str) -> String {
    let mut safe = value
        .trim()
        .chars()
        .map(|ch| {
            if is_invalid_dump_filename_char(ch) {
                '_'
            } else {
                ch
            }
        })
        .take(MAX_DUMP_FILENAME_COMPONENT_CHARS)
        .collect::<String>();
    safe = safe.trim_matches('_').to_string();
    if safe.is_empty() {
        DUMP_FILENAME_FALLBACK.to_string()
    } else {
        safe
    }
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|text| !text.trim().is_empty())
}

fn is_invalid_dump_filename_char(ch: char) -> bool {
    ch.is_control() || matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use db::{DbNode, DbNodeType};
    use one_core::storage::DatabaseType;

    use super::{resolve_sql_dump_target, sanitize_dump_filename_component, sql_dump_filename};

    fn node_with_metadata(node_type: DbNodeType, name: &str, metadata: &[(&str, &str)]) -> DbNode {
        DbNode::new(
            "node-id",
            name,
            node_type,
            "conn-1".to_string(),
            DatabaseType::PostgreSQL,
        )
        .with_metadata(
            metadata
                .iter()
                .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
                .collect::<HashMap<_, _>>(),
        )
    }

    #[test]
    fn resolves_table_dump_target_with_database_and_schema() {
        let node = node_with_metadata(
            DbNodeType::Table,
            "orders",
            &[("database", "sales"), ("schema", "public")],
        );

        let target = resolve_sql_dump_target(&node).expect("table target should resolve");

        assert_eq!("sales", target.database);
        assert_eq!(Some("public"), target.schema.as_deref());
        assert_eq!(Some("orders"), target.table.as_deref());
    }

    #[test]
    fn resolves_schema_dump_target_without_treating_schema_as_database() {
        let node = node_with_metadata(DbNodeType::Schema, "public", &[("database", "analytics")]);

        let target = resolve_sql_dump_target(&node).expect("schema target should resolve");

        assert_eq!("analytics", target.database);
        assert_eq!(Some("public"), target.schema.as_deref());
        assert_eq!(None, target.table.as_deref());
    }

    #[test]
    fn sanitizes_dump_filename_components_for_path_safe_output() {
        assert_eq!(
            "sales_public",
            sanitize_dump_filename_component("sales/public")
        );
        assert_eq!("dump", sanitize_dump_filename_component(" \t\n"));
    }

    #[test]
    fn builds_dump_filename_with_schema_and_table_context() {
        let filename = sql_dump_filename(
            "sales/db",
            Some("public:schema"),
            Some("orders*2026"),
            "2026-05-24_10-00-00",
        );

        assert_eq!(
            "sales_db_public_schema_orders_2026_2026-05-24_10-00-00.sql",
            filename
        );
    }
}
