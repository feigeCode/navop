use db::DbNodeType;
use one_core::storage::DatabaseType;

use crate::extension_menu::{
    DbTreeExtensionMenuContext, DbTreeExtensionMenuItem, DbTreeExtensionMenuRegistry,
};

#[test]
fn extension_menu_filters_visible_items_by_node_and_when_clause() {
    let mut registry = DbTreeExtensionMenuRegistry::default();
    registry.add("db.tree.table", menu_item("visible", "extension@10"));
    let mut hidden = menu_item("hidden", "extension@20");
    hidden.when_clause = Some("connection.kind == 'mysql'".to_string());
    registry.add("db.tree.table", hidden);

    let context = DbTreeExtensionMenuContext {
        node_type: DbNodeType::Table,
        node_name: "users".to_string(),
        connection_id: "conn-1".to_string(),
        database_type: DatabaseType::DuckDB,
    };

    let items = registry.items_for_context(&context);

    assert_eq!(1, items.len());
    assert_eq!("visible", items[0].command_id);
}

fn menu_item(command_id: &str, group: &str) -> DbTreeExtensionMenuItem {
    DbTreeExtensionMenuItem {
        icon: None,
        extension_id: "com.example.tools".to_string(),
        command_id: command_id.to_string(),
        label: command_id.to_string(),
        group: Some(group.to_string()),
        when_clause: Some("node.type == 'table' && connection.kind == 'duckdb'".to_string()),
        requires_active: true,
    }
}

/// DB 树菜单项的图标：命令声明了 icon 就画，没声明就纯文字。
/// 这条是结构断言——图标要经 `local-icon/` 资产命名空间，不能再退化成
/// 绝对路径的 URL（那会被 `url::Url::parse` 当成 URI 走 HTTP）。
#[test]
fn db_tree_menu_items_render_extension_icons() {
    let source = include_str!("db_tree_view.rs");
    assert!(source.contains("fn menu_item_icon("));
    assert!(source.contains("item.icon.as_deref().and_then(menu_item_icon)"));
    assert!(source.contains("entry = entry.icon(icon);"));
    assert!(source.contains("local_icon_asset_path"));
}
