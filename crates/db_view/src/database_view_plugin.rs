use db::DbNodeType;
use db::clickhouse::ClickHousePlugin;
use db::duckdb::DuckDbPlugin;
use db::mssql::MsSqlPlugin;
use db::mysql::MySqlPlugin;
use db::oracle::OraclePlugin;
use db::plugin::DatabasePlugin;
use db::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionPlacement,
    DatabaseActionToolbarScope, DatabaseFormKind, DatabaseUiManifest,
};
use db::postgresql::PostgresPlugin;
use db::sqlite::SqlitePlugin;
use gpui::{App, AppContext, Entity, Window};
use gpui_component::IconName;
use one_core::storage::DatabaseType;

use crate::common::db_connection_form::DbConnectionForm;
use crate::common::manifest_bridge::{
    find_form, matches_node_type, to_column_editor_capabilities, to_connection_form_config,
    to_table_designer_capabilities, translate,
};
use crate::common::{DatabaseEditorView, GenericDatabaseForm, GenericSchemaForm, SchemaEditorView};
use crate::database_objects_tab::DatabaseObjectsEvent;
use crate::db_tree_view::{DbTreeViewEvent, SqlDumpMode};

/// 工具栏按钮类型
#[derive(Debug, Clone)]
pub enum ToolbarButtonType {
    /// 针对当前选中的节点（如刷新、新建）
    CurrentNode,
    /// 针对表格中选中的行（如删除、编辑）
    SelectedRow,
}

/// 工具栏按钮配置
#[derive(Clone)]
pub struct ToolbarButton {
    pub id: &'static str,
    pub icon: IconName,
    pub tooltip: String,
    pub button_type: ToolbarButtonType,
    pub event_fn: fn(db::DbNode) -> DatabaseObjectsEvent,
}

impl ToolbarButton {
    pub fn current_node(
        id: &'static str,
        icon: IconName,
        tooltip: impl Into<String>,
        event_fn: fn(db::DbNode) -> DatabaseObjectsEvent,
    ) -> Self {
        Self {
            id,
            icon,
            tooltip: tooltip.into(),
            button_type: ToolbarButtonType::CurrentNode,
            event_fn,
        }
    }

    pub fn selected_row(
        id: &'static str,
        icon: IconName,
        tooltip: impl Into<String>,
        event_fn: fn(db::DbNode) -> DatabaseObjectsEvent,
    ) -> Self {
        Self {
            id,
            icon,
            tooltip: tooltip.into(),
            button_type: ToolbarButtonType::SelectedRow,
            event_fn,
        }
    }
}

/// 上下文菜单项定义
#[derive(Debug, Clone)]
pub enum ContextMenuItem {
    /// 普通菜单项
    Item {
        label: String,
        event: ContextMenuEvent,
        /// 是否需要连接处于激活状态才可用
        requires_active: bool,
    },
    /// 分隔符
    Separator,
    /// 子菜单
    Submenu {
        label: String,
        items: Vec<ContextMenuItem>,
        /// 是否需要连接处于激活状态才可用
        requires_active: bool,
    },
}

/// 上下文菜单事件
#[derive(Debug, Clone)]
pub enum ContextMenuEvent {
    /// 直接触发的树视图事件
    TreeEvent(DbTreeViewEvent),
    /// 自定义处理器（暂不实现，预留扩展）
    Custom(String),
}

impl ContextMenuItem {
    /// 创建普通菜单项（默认需要连接激活）
    pub fn item(label: impl Into<String>, event: impl Into<DbTreeViewEvent>) -> Self {
        Self::Item {
            label: label.into(),
            event: ContextMenuEvent::TreeEvent(event.into()),
            requires_active: true,
        }
    }

    /// 创建不需要连接激活的菜单项（如删除连接）
    pub fn always_enabled_item(
        label: impl Into<String>,
        event: impl Into<DbTreeViewEvent>,
    ) -> Self {
        Self::Item {
            label: label.into(),
            event: ContextMenuEvent::TreeEvent(event.into()),
            requires_active: false,
        }
    }

    /// 创建分隔符
    pub fn separator() -> Self {
        Self::Separator
    }

    /// 创建子菜单（默认需要连接激活）
    pub fn submenu(label: impl Into<String>, items: Vec<ContextMenuItem>) -> Self {
        Self::Submenu {
            label: label.into(),
            items,
            requires_active: true,
        }
    }
}

/// 表设计器 UI 配置能力
#[derive(Clone, Debug)]
pub struct TableDesignerCapabilities {
    /// 是否支持存储引擎选择（MySQL: InnoDB/MyISAM）
    pub supports_engine: bool,
    /// 是否支持字符集选择
    pub supports_charset: bool,
    /// 是否支持排序规则选择
    pub supports_collation: bool,
    /// 是否支持自增起始值设置
    pub supports_auto_increment: bool,
    /// 是否支持表空间（PostgreSQL）
    pub supports_tablespace: bool,
}

impl Default for TableDesignerCapabilities {
    fn default() -> Self {
        Self {
            supports_engine: false,
            supports_charset: false,
            supports_collation: false,
            supports_auto_increment: false,
            supports_tablespace: false,
        }
    }
}

/// 列编辑器 UI 配置能力
#[derive(Clone, Debug)]
pub struct ColumnEditorCapabilities {
    /// 是否支持 unsigned（MySQL 特有）
    pub supports_unsigned: bool,
    /// 是否支持枚举/集合类型值编辑（MySQL ENUM/SET）
    pub supports_enum_values: bool,
    /// 是否在详情面板显示字符集
    pub show_charset_in_detail: bool,
    /// 是否在详情面板显示排序规则
    pub show_collation_in_detail: bool,
}

impl Default for ColumnEditorCapabilities {
    fn default() -> Self {
        Self {
            supports_unsigned: false,
            supports_enum_values: false,
            show_charset_in_detail: false,
            show_collation_in_detail: false,
        }
    }
}

struct ManifestDatabaseViewPlugin {
    database_type: DatabaseType,
    manifest: DatabaseUiManifest,
}

impl ManifestDatabaseViewPlugin {
    fn new(database_type: DatabaseType) -> Self {
        Self {
            database_type,
            manifest: build_ui_manifest(database_type),
        }
    }

    fn action_descriptors(
        &self,
        node_type: DbNodeType,
        placement: DatabaseActionPlacement,
        toolbar_scope: Option<DatabaseActionToolbarScope>,
    ) -> Vec<&DatabaseActionDescriptor> {
        self.manifest
            .actions
            .actions
            .iter()
            .filter(|action| matches_node_type(action, node_type))
            .filter(|action| match placement {
                DatabaseActionPlacement::ContextMenu => matches!(
                    action.placement,
                    DatabaseActionPlacement::ContextMenu | DatabaseActionPlacement::Both
                ),
                DatabaseActionPlacement::Toolbar => matches!(
                    action.placement,
                    DatabaseActionPlacement::Toolbar | DatabaseActionPlacement::Both
                ),
                DatabaseActionPlacement::Both => true,
            })
            .filter(|action| action.toolbar_scope == toolbar_scope)
            .collect()
    }
}

impl ManifestDatabaseViewPlugin {
    fn create_connection_form(
        &self,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<DbConnectionForm> {
        let plugin = cx
            .global::<db::GlobalDbState>()
            .get_plugin(&self.database_type)
            .expect("database plugin should exist");
        let form = find_form(&self.manifest, DatabaseFormKind::Connection)
            .expect("connection form manifest should exist");
        let config = to_connection_form_config(self.database_type, &form, plugin.as_ref());
        cx.new(|cx| DbConnectionForm::new(config, window, cx))
    }

    fn create_database_editor_view(
        &self,
        _connection_id: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<DatabaseEditorView> {
        let manifest = find_form(&self.manifest, DatabaseFormKind::CreateDatabase)
            .expect("create database form manifest should exist");
        let database_type = self.database_type;
        cx.new(|cx| {
            let form = cx.new(|cx| GenericDatabaseForm::new(database_type, manifest, window, cx));
            DatabaseEditorView::new(form, database_type, false, window, cx)
        })
    }

    fn create_database_editor_view_for_edit(
        &self,
        _connection_id: String,
        _database_name: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Entity<DatabaseEditorView> {
        let manifest = find_form(&self.manifest, DatabaseFormKind::EditDatabase)
            .expect("edit database form manifest should exist");
        let database_type = self.database_type;
        cx.new(|cx| {
            let form = cx.new(|cx| GenericDatabaseForm::new(database_type, manifest, window, cx));
            DatabaseEditorView::new(form, database_type, true, window, cx)
        })
    }

    fn create_schema_editor_view(
        &self,
        _connection_id: String,
        _database_name: String,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<SchemaEditorView>> {
        let manifest = find_form(&self.manifest, DatabaseFormKind::CreateSchema)?;
        let database_type = self.database_type;
        Some(cx.new(|cx| {
            let form = cx.new(|cx| GenericSchemaForm::new(manifest, window, cx));
            SchemaEditorView::new(form, database_type, window, cx)
        }))
    }

    fn get_table_designer_capabilities(&self) -> TableDesignerCapabilities {
        to_table_designer_capabilities(&self.manifest.capabilities)
    }

    fn get_engines(&self) -> Vec<String> {
        self.manifest.capabilities.table_engines.clone()
    }

    fn get_column_editor_capabilities(&self) -> ColumnEditorCapabilities {
        to_column_editor_capabilities(&self.manifest.capabilities)
    }

    fn build_context_menu(&self, node_id: &str, node_type: DbNodeType) -> Vec<ContextMenuItem> {
        self.action_descriptors(node_type, DatabaseActionPlacement::ContextMenu, None)
            .into_iter()
            .filter_map(|action| {
                let label = translate(&action.label_i18n_key);
                let event = map_tree_event(action.id, node_id)?;
                Some(if action.requires_active_connection {
                    ContextMenuItem::item(label, event)
                } else {
                    ContextMenuItem::always_enabled_item(label, event)
                })
            })
            .collect()
    }

    fn build_toolbar_buttons(
        &self,
        node_type: DbNodeType,
        data_node_type: DbNodeType,
    ) -> Vec<ToolbarButton> {
        let current_node_buttons = self
            .action_descriptors(
                node_type,
                DatabaseActionPlacement::Toolbar,
                Some(DatabaseActionToolbarScope::CurrentNode),
            )
            .into_iter()
            .filter_map(|action| {
                let event_fn = map_objects_event(action.id)?;
                Some(ToolbarButton::current_node(
                    action_id(action),
                    toolbar_icon(action),
                    translate(&action.label_i18n_key),
                    event_fn,
                ))
            });

        let selected_row_buttons = self
            .action_descriptors(
                data_node_type,
                DatabaseActionPlacement::Toolbar,
                Some(DatabaseActionToolbarScope::SelectedRow),
            )
            .into_iter()
            .filter_map(|action| {
                let event_fn = map_objects_event(action.id)?;
                Some(ToolbarButton::selected_row(
                    action_id(action),
                    toolbar_icon(action),
                    translate(&action.label_i18n_key),
                    event_fn,
                ))
            });
        current_node_buttons.chain(selected_row_buttons).collect()
    }
}

fn manifest_plugin(database_type: DatabaseType) -> ManifestDatabaseViewPlugin {
    ManifestDatabaseViewPlugin::new(database_type)
}

pub fn create_connection_form_for(
    database_type: DatabaseType,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DbConnectionForm> {
    manifest_plugin(database_type).create_connection_form(window, cx)
}

pub fn create_database_editor_view_for_new(
    database_type: DatabaseType,
    connection_id: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DatabaseEditorView> {
    manifest_plugin(database_type).create_database_editor_view(connection_id, window, cx)
}

pub fn create_database_editor_view_for_edit_type(
    database_type: DatabaseType,
    connection_id: String,
    database_name: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DatabaseEditorView> {
    manifest_plugin(database_type).create_database_editor_view_for_edit(
        connection_id,
        database_name,
        window,
        cx,
    )
}

pub fn create_schema_editor_view_for(
    database_type: DatabaseType,
    connection_id: String,
    database_name: String,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<SchemaEditorView>> {
    manifest_plugin(database_type).create_schema_editor_view(
        connection_id,
        database_name,
        window,
        cx,
    )
}

pub fn build_context_menu_for(
    database_type: DatabaseType,
    node_id: &str,
    node_type: DbNodeType,
) -> Vec<ContextMenuItem> {
    manifest_plugin(database_type).build_context_menu(node_id, node_type)
}

pub fn build_toolbar_buttons_for(
    database_type: DatabaseType,
    node_type: DbNodeType,
    data_node_type: DbNodeType,
) -> Vec<ToolbarButton> {
    manifest_plugin(database_type).build_toolbar_buttons(node_type, data_node_type)
}

pub fn get_table_designer_capabilities_for(
    database_type: DatabaseType,
) -> TableDesignerCapabilities {
    manifest_plugin(database_type).get_table_designer_capabilities()
}

pub fn get_column_editor_capabilities_for(database_type: DatabaseType) -> ColumnEditorCapabilities {
    manifest_plugin(database_type).get_column_editor_capabilities()
}

pub fn get_engines_for(database_type: DatabaseType) -> Vec<String> {
    manifest_plugin(database_type).get_engines()
}

fn build_ui_manifest(database_type: DatabaseType) -> DatabaseUiManifest {
    match database_type {
        DatabaseType::MySQL => MySqlPlugin::new().ui_manifest(),
        DatabaseType::PostgreSQL => PostgresPlugin::new().ui_manifest(),
        DatabaseType::MSSQL => MsSqlPlugin::new().ui_manifest(),
        DatabaseType::Oracle => OraclePlugin::new().ui_manifest(),
        DatabaseType::ClickHouse => ClickHousePlugin::new().ui_manifest(),
        DatabaseType::SQLite => SqlitePlugin::new().ui_manifest(),
        DatabaseType::DuckDB => DuckDbPlugin::new().ui_manifest(),
    }
}

fn map_tree_event(action_id: DatabaseActionId, node_id: &str) -> Option<DbTreeViewEvent> {
    let node_id = node_id.to_string();
    Some(match action_id {
        DatabaseActionId::CloseConnection => DbTreeViewEvent::CloseConnection { node_id },
        DatabaseActionId::DeleteConnection => DbTreeViewEvent::DeleteConnection { node_id },
        DatabaseActionId::CreateDatabase => DbTreeViewEvent::CreateDatabase { node_id },
        DatabaseActionId::EditDatabase => DbTreeViewEvent::EditDatabase { node_id },
        DatabaseActionId::CloseDatabase => DbTreeViewEvent::CloseDatabase { node_id },
        DatabaseActionId::DeleteDatabase => DbTreeViewEvent::DeleteDatabase { node_id },
        DatabaseActionId::CreateSchema => DbTreeViewEvent::CreateSchema { node_id },
        DatabaseActionId::DeleteSchema => DbTreeViewEvent::DeleteSchema { node_id },
        DatabaseActionId::OpenTableData => DbTreeViewEvent::OpenTableData { node_id },
        DatabaseActionId::DesignTable => DbTreeViewEvent::DesignTable { node_id },
        DatabaseActionId::RenameTable => DbTreeViewEvent::RenameTable { node_id },
        DatabaseActionId::CopyTable => DbTreeViewEvent::CopyTable { node_id },
        DatabaseActionId::TruncateTable => DbTreeViewEvent::TruncateTable { node_id },
        DatabaseActionId::DeleteTable => DbTreeViewEvent::DeleteTable { node_id },
        DatabaseActionId::OpenViewData => DbTreeViewEvent::OpenViewData { node_id },
        DatabaseActionId::DeleteView => DbTreeViewEvent::DeleteView { node_id },
        DatabaseActionId::CreateNewQuery => DbTreeViewEvent::CreateNewQuery { node_id },
        DatabaseActionId::OpenNamedQuery => DbTreeViewEvent::OpenNamedQuery { node_id },
        DatabaseActionId::RenameQuery => DbTreeViewEvent::RenameQuery { node_id },
        DatabaseActionId::DeleteQuery => DbTreeViewEvent::DeleteQuery { node_id },
        DatabaseActionId::RunSqlFile => DbTreeViewEvent::RunSqlFile { node_id },
        DatabaseActionId::ImportData => DbTreeViewEvent::ImportData { node_id },
        DatabaseActionId::ExportData => DbTreeViewEvent::ExportData { node_id },
        DatabaseActionId::DumpSqlStructure => DbTreeViewEvent::DumpSqlFile {
            node_id,
            mode: SqlDumpMode::StructureOnly,
        },
        DatabaseActionId::DumpSqlData => DbTreeViewEvent::DumpSqlFile {
            node_id,
            mode: SqlDumpMode::DataOnly,
        },
        DatabaseActionId::DumpSqlStructureAndData => DbTreeViewEvent::DumpSqlFile {
            node_id,
            mode: SqlDumpMode::StructureAndData,
        },
    })
}

fn map_objects_event(
    action_id: DatabaseActionId,
) -> Option<fn(db::DbNode) -> DatabaseObjectsEvent> {
    match action_id {
        DatabaseActionId::CloseConnection => {
            Some(|node| DatabaseObjectsEvent::CloseConnection { node })
        }
        DatabaseActionId::DeleteConnection => {
            Some(|node| DatabaseObjectsEvent::DeleteConnection { node })
        }
        DatabaseActionId::CreateDatabase => {
            Some(|node| DatabaseObjectsEvent::CreateDatabase { node })
        }
        DatabaseActionId::EditDatabase => Some(|node| DatabaseObjectsEvent::EditDatabase { node }),
        DatabaseActionId::DeleteDatabase => {
            Some(|node| DatabaseObjectsEvent::DeleteDatabase { node })
        }
        DatabaseActionId::CreateSchema => Some(|node| DatabaseObjectsEvent::CreateSchema { node }),
        DatabaseActionId::DeleteSchema => Some(|node| DatabaseObjectsEvent::DeleteSchema { node }),
        DatabaseActionId::OpenTableData => {
            Some(|node| DatabaseObjectsEvent::OpenTableData { node })
        }
        DatabaseActionId::DesignTable => Some(|node| DatabaseObjectsEvent::DesignTable { node }),
        DatabaseActionId::DeleteTable => Some(|node| DatabaseObjectsEvent::DeleteTable { node }),
        DatabaseActionId::OpenViewData => Some(|node| DatabaseObjectsEvent::OpenViewData { node }),
        DatabaseActionId::DeleteView => Some(|node| DatabaseObjectsEvent::DeleteView { node }),
        DatabaseActionId::CreateNewQuery => {
            Some(|node| DatabaseObjectsEvent::CreateNewQuery { node })
        }
        DatabaseActionId::OpenNamedQuery => {
            Some(|node| DatabaseObjectsEvent::OpenNamedQuery { node })
        }
        DatabaseActionId::RenameQuery => Some(|node| DatabaseObjectsEvent::RenameQuery { node }),
        DatabaseActionId::DeleteQuery => Some(|node| DatabaseObjectsEvent::DeleteQuery { node }),
        DatabaseActionId::CloseDatabase
        | DatabaseActionId::RenameTable
        | DatabaseActionId::CopyTable
        | DatabaseActionId::TruncateTable
        | DatabaseActionId::RunSqlFile
        | DatabaseActionId::ImportData
        | DatabaseActionId::ExportData
        | DatabaseActionId::DumpSqlStructure
        | DatabaseActionId::DumpSqlData
        | DatabaseActionId::DumpSqlStructureAndData => None,
    }
}

fn toolbar_icon(action: &DatabaseActionDescriptor) -> IconName {
    match action.id {
        DatabaseActionId::CloseConnection => IconName::CircleX,
        DatabaseActionId::DeleteConnection
        | DatabaseActionId::DeleteDatabase
        | DatabaseActionId::DeleteSchema
        | DatabaseActionId::DeleteTable
        | DatabaseActionId::DeleteView
        | DatabaseActionId::DeleteQuery => IconName::Minus,
        DatabaseActionId::EditDatabase
        | DatabaseActionId::RenameQuery
        | DatabaseActionId::OpenNamedQuery => IconName::Edit,
        DatabaseActionId::OpenTableData | DatabaseActionId::OpenViewData => IconName::Eye,
        DatabaseActionId::CreateDatabase
        | DatabaseActionId::CreateSchema
        | DatabaseActionId::CreateNewQuery => IconName::Plus,
        DatabaseActionId::DesignTable => {
            if action.label_i18n_key == "Table.new_table" {
                IconName::Plus
            } else {
                IconName::Edit
            }
        }
        _ => IconName::Plus,
    }
}

fn action_id(action: &DatabaseActionDescriptor) -> &'static str {
    match action.id {
        DatabaseActionId::CloseConnection => "close-connection",
        DatabaseActionId::DeleteConnection => "delete-connection",
        DatabaseActionId::CreateDatabase => "create-database",
        DatabaseActionId::EditDatabase => "edit-database",
        DatabaseActionId::CloseDatabase => "close-database",
        DatabaseActionId::DeleteDatabase => "delete-database",
        DatabaseActionId::CreateSchema => "create-schema",
        DatabaseActionId::DeleteSchema => "delete-schema",
        DatabaseActionId::OpenTableData => "open-table-data",
        DatabaseActionId::DesignTable => {
            if action.label_i18n_key == "Table.new_table" {
                "create-table"
            } else {
                "design-table"
            }
        }
        DatabaseActionId::RenameTable => "rename-table",
        DatabaseActionId::CopyTable => "copy-table",
        DatabaseActionId::TruncateTable => "truncate-table",
        DatabaseActionId::DeleteTable => "delete-table",
        DatabaseActionId::OpenViewData => "open-view-data",
        DatabaseActionId::DeleteView => "delete-view",
        DatabaseActionId::CreateNewQuery => "create-query",
        DatabaseActionId::OpenNamedQuery => "open-query",
        DatabaseActionId::RenameQuery => "rename-query",
        DatabaseActionId::DeleteQuery => "delete-query",
        DatabaseActionId::RunSqlFile => "run-sql-file",
        DatabaseActionId::ImportData => "import-data",
        DatabaseActionId::ExportData => "export-data",
        DatabaseActionId::DumpSqlStructure => "dump-sql-structure",
        DatabaseActionId::DumpSqlData => "dump-sql-data",
        DatabaseActionId::DumpSqlStructureAndData => "dump-sql-structure-and-data",
    }
}
