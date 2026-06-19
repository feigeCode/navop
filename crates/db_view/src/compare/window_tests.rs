use db::compare::{SyncPlan, SyncPlanSummary, SyncStatement, SyncStatementKind};
use db::{DbNode, DbNodeType};
use one_core::storage::DatabaseType;
use rust_i18n::t;

use crate::compare::sync_statement_picker::selected_sync_sql_text_for_ids;
use crate::compare::window_params::{
    DataCompareSelection, SchemaCompareSelection, data_compare_params, schema_compare_params,
    split_columns,
};
use crate::compare::{DataCompareWindow, SchemaCompareWindow};

#[test]
fn data_compare_popup_title_uses_source_table() {
    let node = DbNode::new(
        "node-1",
        "users",
        DbNodeType::Table,
        "conn-1".to_string(),
        DatabaseType::PostgreSQL,
    );

    assert_eq!(
        DataCompareWindow::popup_title_for(&node),
        t!("Compare.data_compare_title", name = "users").to_string()
    );
}

#[test]
fn schema_compare_popup_title_uses_source_scope() {
    let node = DbNode::new(
        "db-1",
        "app",
        DbNodeType::Database,
        "conn-1".to_string(),
        DatabaseType::PostgreSQL,
    );

    assert_eq!(
        SchemaCompareWindow::popup_title_for(&node),
        t!("Compare.schema_compare_title", name = "app").to_string()
    );
}

#[test]
fn split_columns_ignores_empty_segments() {
    assert_eq!(
        split_columns("id, tenant_id, , ".to_string()),
        vec!["id".to_string(), "tenant_id".to_string()]
    );
}

#[test]
fn schema_compare_params_use_editable_source_selection() {
    let params = schema_compare_params(
        SchemaCompareSelection {
            connection_id: "source-2".to_string(),
            database: "source_db".to_string(),
            schema: "source_schema".to_string(),
        },
        SchemaCompareSelection {
            connection_id: "target-1".to_string(),
            database: "target_db".to_string(),
            schema: "target_schema".to_string(),
        },
    )
    .unwrap();

    assert_eq!(params.source_connection_id, "source-2");
    assert_eq!(params.source_database, "source_db");
    assert_eq!(params.source_schema.as_deref(), Some("source_schema"));
}

#[test]
fn data_compare_params_use_editable_source_selection() {
    let params = data_compare_params(
        DataCompareSelection {
            connection_id: "source-2".to_string(),
            database: "source_db".to_string(),
            schema: "source_schema".to_string(),
            table: "source_table".to_string(),
        },
        DataCompareSelection {
            connection_id: "target-1".to_string(),
            database: "target_db".to_string(),
            schema: "target_schema".to_string(),
            table: "target_table".to_string(),
        },
        "id, tenant_id".to_string(),
    )
    .unwrap();

    assert_eq!(params.source_connection_id, "source-2");
    assert_eq!(params.source_database, "source_db");
    assert_eq!(params.source_schema.as_deref(), Some("source_schema"));
    assert_eq!(params.source_table, "source_table");
    assert_eq!(params.key_columns, vec!["id", "tenant_id"]);
}

#[test]
fn selected_sync_sql_text_skips_unselected_destructive_statements() {
    let plan = SyncPlan {
        id: "plan-1".to_string(),
        target_table: "users".to_string(),
        statements: vec![
            statement(
                "insert-1",
                "INSERT INTO users (id) VALUES (1);",
                true,
                false,
            ),
            statement("delete-1", "DELETE FROM users WHERE id = 2;", false, true),
        ],
        summary: SyncPlanSummary {
            insert_count: 1,
            update_count: 0,
            delete_count: 1,
            ddl_count: 0,
            total_count: 2,
        },
        warnings: vec![],
        sql_text: "INSERT INTO users (id) VALUES (1);\nDELETE FROM users WHERE id = 2;".to_string(),
    };

    let selected_ids = crate::compare::sync_statement_picker::default_selected_statement_ids(&plan);

    assert_eq!(
        selected_sync_sql_text_for_ids(&plan, &selected_ids),
        "INSERT INTO users (id) VALUES (1);"
    );
}

fn statement(id: &str, sql: &str, selected: bool, destructive: bool) -> SyncStatement {
    SyncStatement {
        id: id.to_string(),
        sql: sql.to_string(),
        kind: SyncStatementKind::Unknown,
        object_name: None,
        row_key: None,
        destructive,
        transactional_safe: true,
        selected_by_default: selected,
        warnings: vec![],
    }
}
