//! 回归测试：查询结果的编辑网格保存变更时，表名不能重复拼接限定前缀。
//!
//! 复现场景：`select * from \`user_center\`.\`uc_user_info\`` 的查询结果网格里
//! 表名来自 SQL 分析，是 `库.表` 的拼接形式；如果生成 SQL 时再拼一次当前连接的
//! 数据库名，会得到 `` `user_center`.`user_center.uc_user_info` ``，
//! MySQL 报 `ERROR 42S02 (1146): Table 'user_center.user_center.uc_user_info' doesn't exist`。
use db::DbManager;
use db::plugin::DatabasePlugin;
use db::types::{TableCellChange, TableCellValue, TableRowChange, TableSaveRequest};
use one_core::storage::DatabaseType;

fn mysql_plugin() -> std::sync::Arc<dyn DatabasePlugin> {
    DbManager::default()
        .get_plugin(&DatabaseType::MySQL)
        .expect("MySQL plugin should exist")
}

fn qualified_table_request(table: &str) -> TableSaveRequest {
    TableSaveRequest {
        database: "user_center".to_string(),
        schema: None,
        table: table.to_string(),
        columns: vec![],
        index_infos: vec![],
        changes: vec![TableRowChange::Updated {
            original_data: vec![],
            changes: vec![TableCellChange {
                column_index: 0,
                column_name: "user_type".to_string(),
                old_value: TableCellValue::Null,
                new_value: TableCellValue::Text("000104".to_string()),
            }],
            rowid: Some("1".to_string()),
        }],
    }
}

#[test]
fn save_sql_keeps_qualified_table_name_once() {
    let plugin = mysql_plugin();
    assert_eq!(
        plugin.generate_table_changes_sql(&qualified_table_request("user_center.uc_user_info")),
        "UPDATE `user_center`.`uc_user_info` SET `user_type` = '000104' WHERE rowid = '1';"
    );
}

#[test]
fn save_sql_of_bare_table_name_is_unchanged() {
    let plugin = mysql_plugin();
    assert_eq!(
        plugin.generate_table_changes_sql(&qualified_table_request("uc_user_info")),
        "UPDATE `user_center`.`uc_user_info` SET `user_type` = '000104' WHERE rowid = '1';"
    );
}

#[test]
fn metadata_identity_uses_prefix_as_database_for_mysql() {
    let plugin = mysql_plugin();
    assert_eq!(
        plugin.resolve_metadata_table_identity("user_center", None, "user_center.uc_user_info"),
        ("user_center".to_string(), None, "uc_user_info".to_string())
    );
    assert_eq!(
        plugin.resolve_metadata_table_identity("main", None, "uc_user_info"),
        ("main".to_string(), None, "uc_user_info".to_string())
    );
}
