use db::DbNodeType;
use gpui::{App, Global, Window};
use one_core::storage::DatabaseType;
use std::sync::Arc;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbTreeExtensionMenuItem {
    pub extension_id: String,
    pub command_id: String,
    pub label: String,
    pub group: Option<String>,
    pub when_clause: Option<String>,
    pub requires_active: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbTreeExtensionActionContext {
    pub extension_id: String,
    pub command_id: String,
    pub node_id: String,
    pub node_name: String,
    pub node_type: DbNodeType,
    pub database_type: DatabaseType,
    pub connection_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DbTreeExtensionMenuContext {
    pub node_type: DbNodeType,
    pub node_name: String,
    pub connection_id: String,
    pub database_type: DatabaseType,
}

pub trait DbTreeExtensionActionHandler: Send + Sync {
    fn run(&self, context: DbTreeExtensionActionContext, window: &mut Window, cx: &mut App);
}

#[derive(Clone)]
pub struct GlobalDbTreeExtensionActionHandler {
    handler: Arc<dyn DbTreeExtensionActionHandler>,
}

impl Global for GlobalDbTreeExtensionActionHandler {}

impl GlobalDbTreeExtensionActionHandler {
    pub fn new(handler: Arc<dyn DbTreeExtensionActionHandler>) -> Self {
        Self { handler }
    }

    pub fn run(&self, context: DbTreeExtensionActionContext, window: &mut Window, cx: &mut App) {
        self.handler.run(context, window, cx);
    }
}

#[derive(Clone, Debug, Default)]
pub struct DbTreeExtensionMenuRegistry {
    items: Vec<RegisteredDbTreeMenuItem>,
}

impl Global for DbTreeExtensionMenuRegistry {}

impl DbTreeExtensionMenuRegistry {
    pub fn add(&mut self, position: impl Into<String>, item: DbTreeExtensionMenuItem) {
        self.items.push(RegisteredDbTreeMenuItem {
            position: position.into(),
            item,
        });
    }

    pub fn items_for_node(&self, node_type: DbNodeType) -> Vec<DbTreeExtensionMenuItem> {
        let Some(position) = db_tree_menu_position(node_type) else {
            return Vec::new();
        };
        let mut items = self
            .items
            .iter()
            .filter(|entry| entry.position == position)
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>();
        items.sort_by_key(menu_sort_key);
        items
    }

    pub fn items_for_context(
        &self,
        context: &DbTreeExtensionMenuContext,
    ) -> Vec<DbTreeExtensionMenuItem> {
        let Some(position) = db_tree_menu_position(context.node_type) else {
            return Vec::new();
        };
        let when_context = when_context_for_menu(context);
        let mut items = self
            .items
            .iter()
            .filter(|entry| entry.position == position)
            .filter(|entry| menu_item_visible(&entry.item, &when_context))
            .map(|entry| entry.item.clone())
            .collect::<Vec<_>>();
        items.sort_by_key(menu_sort_key);
        items
    }
}

#[derive(Clone, Debug)]
struct RegisteredDbTreeMenuItem {
    position: String,
    item: DbTreeExtensionMenuItem,
}

fn db_tree_menu_position(node_type: DbNodeType) -> Option<&'static str> {
    match node_type {
        DbNodeType::Connection => Some("db.tree.connection"),
        DbNodeType::Database => Some("db.tree.database"),
        DbNodeType::Schema => Some("db.tree.schema"),
        DbNodeType::Table => Some("db.tree.table"),
        DbNodeType::View => Some("db.tree.view"),
        DbNodeType::Column => Some("db.tree.column"),
        DbNodeType::Index => Some("db.tree.index"),
        DbNodeType::Function => Some("db.tree.function"),
        DbNodeType::Procedure => Some("db.tree.procedure"),
        DbNodeType::Trigger => Some("db.tree.trigger"),
        DbNodeType::Sequence => Some("db.tree.sequence"),
        _ => None,
    }
}

fn when_context_for_menu(
    context: &DbTreeExtensionMenuContext,
) -> one_core::when_clause::WhenContext {
    one_core::when_clause::WhenContext::from_json(serde_json::json!({
        "connection": {
            "id": context.connection_id.as_str(),
            "kind": database_type_when_value(&context.database_type),
            "driver_id": context.database_type.external_driver_id()
        },
        "node": {
            "type": db_node_type_when_value(context.node_type),
            "name": context.node_name.as_str()
        }
    }))
}

fn menu_item_visible(
    item: &DbTreeExtensionMenuItem,
    context: &one_core::when_clause::WhenContext,
) -> bool {
    match item.when_clause.as_deref() {
        None | Some("") => true,
        Some(source) => one_core::when_clause::evaluate(source, context).unwrap_or(false),
    }
}

fn db_node_type_when_value(node_type: DbNodeType) -> &'static str {
    match node_type {
        DbNodeType::Connection => "connection",
        DbNodeType::Database => "database",
        DbNodeType::Schema => "schema",
        DbNodeType::Table => "table",
        DbNodeType::View => "view",
        DbNodeType::Column => "column",
        DbNodeType::Index => "index",
        DbNodeType::Function => "function",
        DbNodeType::Procedure => "procedure",
        DbNodeType::Trigger => "trigger",
        DbNodeType::Sequence => "sequence",
        DbNodeType::QueryFolder => "query_folder",
        DbNodeType::NamedQuery => "query",
        _ => "",
    }
}

fn database_type_when_value(database_type: &DatabaseType) -> &'static str {
    match database_type {
        DatabaseType::MySQL => "mysql",
        DatabaseType::PostgreSQL => "postgresql",
        DatabaseType::SQLite => "sqlite",
        DatabaseType::DuckDB => "duckdb",
        DatabaseType::MSSQL => "mssql",
        DatabaseType::Oracle => "oracle",
        DatabaseType::ClickHouse => "clickhouse",
        DatabaseType::TDengine => "tdengine",
        DatabaseType::External { .. } => "external",
    }
}

fn menu_sort_key(item: &DbTreeExtensionMenuItem) -> (String, i32, String) {
    let Some(group) = &item.group else {
        return (String::new(), 0, item.label.clone());
    };
    let (name, order) = group
        .split_once('@')
        .map(|(name, order)| (name, order.parse::<i32>().unwrap_or(0)))
        .unwrap_or((group.as_str(), 0));
    (name.to_string(), order, item.label.clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::DbNodeType;

    fn item(command_id: &str, label: &str, group: &str) -> DbTreeExtensionMenuItem {
        DbTreeExtensionMenuItem {
            extension_id: "com.example.tools".to_string(),
            command_id: command_id.to_string(),
            label: label.to_string(),
            group: Some(group.to_string()),
            when_clause: None,
            requires_active: true,
        }
    }

    #[test]
    fn registry_matches_db_tree_table_items_by_node_type() {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        registry.add(
            "db.tree.table",
            item("my.sync.table", "同步表", "extension@20"),
        );
        registry.add(
            "db.tree.database",
            item("my.search.database", "全库搜索", "extension@10"),
        );

        let items = registry.items_for_node(DbNodeType::Table);

        assert_eq!(1, items.len());
        assert_eq!("my.sync.table", items[0].command_id);
    }

    #[test]
    fn registry_orders_items_by_group_order() {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        registry.add("db.tree.table", item("late", "Late", "extension@20"));
        registry.add("db.tree.table", item("early", "Early", "extension@10"));

        let items = registry.items_for_node(DbNodeType::Table);

        assert_eq!("early", items[0].command_id);
        assert_eq!("late", items[1].command_id);
    }

    #[test]
    fn registry_filters_items_by_when_clause() {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        let mut visible = item("visible", "Visible", "extension@10");
        visible.when_clause =
            Some("node.type == 'table' && connection.kind == 'duckdb'".to_string());
        let mut hidden = item("hidden", "Hidden", "extension@20");
        hidden.when_clause = Some("node.type == 'schema'".to_string());
        registry.add("db.tree.table", visible);
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

    #[test]
    fn invalid_when_clause_hides_item() {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        let mut invalid = item("invalid", "Invalid", "extension@10");
        invalid.when_clause = Some("node.type ==".to_string());
        registry.add("db.tree.table", invalid);

        let context = DbTreeExtensionMenuContext {
            node_type: DbNodeType::Table,
            node_name: "users".to_string(),
            connection_id: "conn-1".to_string(),
            database_type: DatabaseType::DuckDB,
        };

        assert!(registry.items_for_context(&context).is_empty());
    }

    #[test]
    fn external_menu_when_clause_can_match_driver_id() {
        let mut registry = DbTreeExtensionMenuRegistry::default();
        let mut visible = item("iotdb.export", "IoTDB 导出", "extension@10");
        visible.when_clause = Some("connection.driver_id == 'iotdb'".to_string());
        registry.add("db.tree.table", visible);

        let context = DbTreeExtensionMenuContext {
            node_type: DbNodeType::Table,
            node_name: "metrics".to_string(),
            connection_id: "conn-1".to_string(),
            database_type: DatabaseType::external("iotdb"),
        };

        let items = registry.items_for_context(&context);

        assert_eq!(1, items.len());
        assert_eq!("iotdb.export", items[0].command_id);
    }
}
