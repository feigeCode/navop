use std::collections::BTreeMap;

use db::{DbNodeType, GlobalDbState};
use extension_component::{
    DbSelectorKind, DbSelectorSource, FieldSource, PermissionSet, SelectOption, SqlAccess, UiNode,
    ViewSpec,
};

use crate::extension_menu::DbTreeExtensionActionContext;
use crate::extension_selector_parts::selector_parts;

pub type ExtensionSelectorOptions = BTreeMap<String, Vec<SelectOption>>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectorRequest {
    pub connection_id: Option<String>,
    pub database: Option<String>,
    pub schema: Option<String>,
    pub table: Option<String>,
}

pub async fn load_extension_selector_options(
    spec: &ViewSpec,
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    context: &DbTreeExtensionActionContext,
) -> ExtensionSelectorOptions {
    let mut options = BTreeMap::new();
    for (field_id, source) in db_selector_sources(spec) {
        let values = load_selector_options(&source, db_state, permissions, context).await;
        options.insert(field_id, values);
    }
    options
}

pub fn selector_request_for_source(
    source: &DbSelectorSource,
    context: &DbTreeExtensionActionContext,
) -> SelectorRequest {
    SelectorRequest {
        connection_id: source
            .query
            .connection_id
            .clone()
            .or_else(|| Some(context.connection_id.clone())),
        database: source.query.database.clone().or_else(|| {
            (context.node_type == DbNodeType::Database).then(|| context.node_name.clone())
        }),
        schema: source.query.schema.clone().or_else(|| {
            (context.node_type == DbNodeType::Schema).then(|| context.node_name.clone())
        }),
        table: source.query.table.clone().or_else(|| {
            (context.node_type == DbNodeType::Table).then(|| context.node_name.clone())
        }),
    }
}

async fn load_selector_options(
    source: &DbSelectorSource,
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    context: &DbTreeExtensionActionContext,
) -> Vec<SelectOption> {
    let request = selector_request_for_source(source, context);
    match source.kind {
        DbSelectorKind::Connection => connection_options(db_state, permissions),
        DbSelectorKind::Database => database_options(db_state, permissions, request).await,
        DbSelectorKind::Schema => schema_options(db_state, permissions, request).await,
        DbSelectorKind::Table => table_options(db_state, permissions, request).await,
        DbSelectorKind::Column => column_options(db_state, permissions, request).await,
    }
}

fn connection_options(db_state: &GlobalDbState, permissions: &PermissionSet) -> Vec<SelectOption> {
    if !permissions.allows_connection_list() {
        return Vec::new();
    }
    db_state
        .list_connection_summaries()
        .into_iter()
        .map(|connection| SelectOption {
            value: connection.id,
            label: connection.name,
        })
        .collect()
}

async fn database_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request) else {
        return Vec::new();
    };
    db_state
        .list_databases_direct(connection_id)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(named_option)
        .collect()
}

async fn schema_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let Some(database) = request.database else {
        return Vec::new();
    };
    db_state
        .list_schemas_direct(connection_id, database)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(named_option)
        .collect()
}

async fn table_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let Some(database) = request.database else {
        return Vec::new();
    };
    db_state
        .list_tables_direct(&connection_id, &database, request.schema)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|table| named_option(table.name))
        .collect()
}

async fn column_options(
    db_state: &GlobalDbState,
    permissions: &PermissionSet,
    request: SelectorRequest,
) -> Vec<SelectOption> {
    let Some(connection_id) = allowed_connection_id(permissions, request.clone()) else {
        return Vec::new();
    };
    let (Some(database), Some(table)) = (request.database, request.table) else {
        return Vec::new();
    };
    db_state
        .list_columns_direct(&connection_id, &database, request.schema, &table)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|column| SelectOption {
            value: column.name.clone(),
            label: format!("{} ({})", column.name, column.data_type),
        })
        .collect()
}

fn allowed_connection_id(permissions: &PermissionSet, request: SelectorRequest) -> Option<String> {
    let connection_id = request.connection_id?;
    permissions
        .allows_db(SqlAccess::Schema, &connection_id)
        .then_some(connection_id)
}

fn named_option(name: String) -> SelectOption {
    SelectOption {
        value: name.clone(),
        label: name,
    }
}

fn db_selector_sources(spec: &ViewSpec) -> Vec<(String, DbSelectorSource)> {
    spec.nodes
        .iter()
        .flat_map(|node| match node {
            UiNode::Text { .. } => Vec::new(),
            UiNode::Form { fields } => fields
                .iter()
                .flat_map(|field| match field.source.as_ref() {
                    Some(FieldSource::DbSelector(source)) => db_selector_parts(&field.id, source),
                    _ => Vec::new(),
                })
                .collect(),
        })
        .collect()
}

fn db_selector_parts(field_id: &str, source: &DbSelectorSource) -> Vec<(String, DbSelectorSource)> {
    selector_parts(source)
        .into_iter()
        .map(|part| (format!("{}.{}", field_id, part.suffix), part.source))
        .collect()
}

#[cfg(test)]
mod tests {
    use db::DbNodeType;
    use extension_component::{
        DbSelectorKind, DbSelectorQuery, DbSelectorSource, UiField, UiNode, ViewSpec,
    };
    use one_core::storage::DatabaseType;

    use crate::extension_menu::DbTreeExtensionActionContext;

    #[test]
    fn selector_request_uses_database_tree_context_defaults() {
        let source = DbSelectorSource {
            kind: DbSelectorKind::Schema,
            query: DbSelectorQuery::default(),
        };

        let request = super::selector_request_for_source(&source, &database_context());

        assert_eq!(Some("conn-1"), request.connection_id.as_deref());
        assert_eq!(Some("app"), request.database.as_deref());
    }

    #[test]
    fn selector_request_keeps_explicit_query_values() {
        let source = DbSelectorSource {
            kind: DbSelectorKind::Column,
            query: DbSelectorQuery {
                connection_id: Some("conn-2".to_string()),
                database: Some("warehouse".to_string()),
                schema: Some("analytics".to_string()),
                table: Some("events".to_string()),
            },
        };

        let request = super::selector_request_for_source(&source, &database_context());

        assert_eq!(Some("conn-2"), request.connection_id.as_deref());
        assert_eq!(Some("warehouse"), request.database.as_deref());
        assert_eq!(Some("analytics"), request.schema.as_deref());
        assert_eq!(Some("events"), request.table.as_deref());
    }

    #[test]
    fn db_selector_sources_expand_table_selector_to_composite_parts() {
        let spec = ViewSpec::dialog(
            "full-search",
            "全库搜索",
            vec![UiNode::form(vec![UiField::db_select(
                "target",
                "目标",
                DbSelectorKind::Table,
            )])],
            vec![],
        );

        let sources = super::db_selector_sources(&spec);
        let ids = sources
            .iter()
            .map(|(id, source)| (id.as_str(), &source.kind))
            .collect::<Vec<_>>();

        assert_eq!(
            vec![
                ("target.connection_id", &DbSelectorKind::Connection),
                ("target.database", &DbSelectorKind::Database),
                ("target.schema", &DbSelectorKind::Schema),
                ("target.table", &DbSelectorKind::Table),
            ],
            ids
        );
    }

    fn database_context() -> DbTreeExtensionActionContext {
        DbTreeExtensionActionContext {
            extension_id: "com.example.db".to_string(),
            command_id: "example.search".to_string(),
            node_id: "db-node".to_string(),
            node_name: "app".to_string(),
            node_type: DbNodeType::Database,
            database_type: DatabaseType::PostgreSQL,
            connection_id: "conn-1".to_string(),
        }
    }
}
