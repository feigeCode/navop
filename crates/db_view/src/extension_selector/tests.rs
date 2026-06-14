use db::DbNodeType;
use extension_component::{
    DbSelectorKind, DbSelectorQuery, DbSelectorSource, UiField, UiNode, ViewSpec,
};
use one_core::storage::DatabaseType;

use crate::extension_menu::DbTreeExtensionActionContext;
use crate::extension_widget::ExtensionWidgetSelectorPolicy;

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
    let spec = table_selector_spec();
    let sources = super::parts::db_selector_sources(&spec);
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

#[test]
fn db_selector_sources_hide_schema_when_policy_disables_schema() {
    let spec = table_selector_spec();
    let mut policies = std::collections::BTreeMap::new();
    policies.insert(
        "target".to_string(),
        ExtensionWidgetSelectorPolicy {
            show_schema: false,
            schema_as_database: false,
        },
    );

    let sources = super::parts::db_selector_sources_with_policies(&spec, &policies);
    let ids = sources
        .iter()
        .map(|(id, source)| (id.as_str(), &source.kind))
        .collect::<Vec<_>>();

    assert_eq!(
        vec![
            ("target.connection_id", &DbSelectorKind::Connection),
            ("target.database", &DbSelectorKind::Database),
            ("target.table", &DbSelectorKind::Table),
        ],
        ids
    );
}

#[test]
fn selector_database_schema_maps_oracle_schema_as_database() {
    let request = super::SelectorRequest {
        connection_id: Some("conn-1".to_string()),
        database: None,
        schema: Some("HR".to_string()),
        table: Some("EMP".to_string()),
    };
    let policy = crate::db_object_selector::DbObjectSelectorPolicy {
        show_schema: false,
        schema_as_database: true,
    };

    assert_eq!(
        Some("HR".to_string()),
        super::selector_database(&request, policy)
    );
    assert_eq!(
        Some("HR".to_string()),
        super::selector_schema(&request, policy)
    );
}

#[test]
fn selector_database_schema_clears_schema_when_unsupported() {
    let request = super::SelectorRequest {
        connection_id: Some("conn-1".to_string()),
        database: Some("app".to_string()),
        schema: Some("ignored".to_string()),
        table: None,
    };

    assert_eq!(
        Some("app".to_string()),
        super::selector_database(
            &request,
            crate::db_object_selector::DbObjectSelectorPolicy::default()
        )
    );
    assert_eq!(
        None,
        super::selector_schema(
            &request,
            crate::db_object_selector::DbObjectSelectorPolicy::default()
        )
    );
}

fn table_selector_spec() -> ViewSpec {
    ViewSpec::dialog(
        "full-search",
        "全库搜索",
        vec![UiNode::form(vec![UiField::db_select(
            "target",
            "目标",
            DbSelectorKind::Table,
        )])],
        vec![],
    )
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
