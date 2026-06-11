use db::compare::{SyncPlan, SyncPlanSummary, SyncStatement, SyncStatementKind};
use db::{DbNode, DbNodeType};
use one_core::storage::DatabaseType;

use crate::compare::sync_statement_picker::selected_sync_sql_text_for_ids;
use crate::compare::window_params::split_columns;
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
        "数据比较 - users"
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
        "结构比较 - app"
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
