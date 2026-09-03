use db::DbNodeType;
use db::ipc::{IpcDriverManifest, IpcDriverRegistry};
use db::plugin::{DatabasePlugin, DatabaseUserOperationRequest};
use db::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionPlacement,
    DatabaseActionToolbarScope, DatabaseCapabilities, DatabaseFormKind, DatabaseUiManifest,
};
use gpui::{App, AppContext, Entity, Window};
use gpui_component::IconName;
use one_core::storage::DatabaseType;
use std::rc::Rc;

use crate::common::db_connection_form::{
    DbConnectionForm, DbFormConfig, FormField, FormFieldType, TabGroup,
};
use crate::common::manifest_bridge::{
    find_form, matches_node_type, to_column_editor_capabilities, to_connection_form_config,
    to_connection_form_config_with_text_resolver, to_table_designer_capabilities, translate,
};
use crate::common::{
    DatabaseEditorView, GenericDatabaseForm, GenericSchemaForm, GenericUserForm, SchemaEditorView,
    UserEditorView,
};
use crate::database_objects_tab::DatabaseObjectsEvent;
use crate::db_tree_view::{DbTreeViewEvent, SqlDumpMode};
use std::collections::HashMap;

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
#[derive(Clone, Debug, Default)]
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

/// 列编辑器 UI 配置能力
#[derive(Clone, Debug, Default)]
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

struct ManifestDatabaseViewPlugin {
    database_type: DatabaseType,
    manifest: DatabaseUiManifest,
    capabilities: DatabaseCapabilities,
    external_driver: Option<IpcDriverManifest>,
}

impl ManifestDatabaseViewPlugin {
    fn new(database_type: DatabaseType, plugin: &dyn DatabasePlugin) -> Self {
        Self {
            database_type,
            manifest: plugin.ui_manifest(),
            capabilities: plugin.capabilities(),
            external_driver: plugin.external_driver_manifest(),
        }
    }

    fn translate_text(&self, key_or_text: &str) -> String {
        if let Some(driver) = &self.external_driver {
            translate_external_driver_text(driver, key_or_text)
        } else {
            translate(key_or_text)
        }
    }

    fn text_resolver(&self) -> Rc<dyn Fn(&str) -> String> {
        if let Some(driver) = self.external_driver.clone() {
            Rc::new(move |key| translate_external_driver_text(&driver, key))
        } else {
            Rc::new(translate)
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
            .filter(|action| match placement {
                DatabaseActionPlacement::Toolbar => action.toolbar_scope == toolbar_scope,
                DatabaseActionPlacement::ContextMenu | DatabaseActionPlacement::Both => true,
            })
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
        let config = to_connection_form_config(self.database_type.clone(), &form, plugin.as_ref());
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
        let database_type = self.database_type.clone();
        let text_resolver = self.text_resolver();
        cx.new(|cx| {
            let form = cx.new(|cx| {
                GenericDatabaseForm::new_with_text_resolver(
                    database_type.clone(),
                    manifest,
                    text_resolver,
                    window,
                    cx,
                )
            });
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
        let database_type = self.database_type.clone();
        let text_resolver = self.text_resolver();
        cx.new(|cx| {
            let form = cx.new(|cx| {
                GenericDatabaseForm::new_with_text_resolver(
                    database_type.clone(),
                    manifest,
                    text_resolver,
                    window,
                    cx,
                )
            });
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
        let database_type = self.database_type.clone();
        Some(cx.new(|cx| {
            let form = cx.new(|cx| GenericSchemaForm::new(manifest, window, cx));
            SchemaEditorView::new(form, database_type, window, cx)
        }))
    }

    fn create_user_editor_view(
        &self,
        operation: DatabaseFormKind,
        initial: Option<DatabaseUserOperationRequest>,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Entity<UserEditorView>> {
        let manifest = find_form(&self.manifest, operation)?;
        let database_type = self.database_type.clone();
        let text_resolver = self.text_resolver();
        Some(cx.new(|cx| {
            let form = cx.new(|cx| {
                GenericUserForm::new_with_text_resolver(
                    database_type.clone(),
                    manifest,
                    initial,
                    text_resolver,
                    window,
                    cx,
                )
            });
            let initial_request = form.read(cx).current_request(cx);
            UserEditorView::new(
                form,
                database_type,
                operation,
                Some(initial_request),
                window,
                cx,
            )
        }))
    }

    fn get_table_designer_capabilities(&self) -> TableDesignerCapabilities {
        to_table_designer_capabilities(&self.capabilities)
    }

    fn get_engines(&self) -> Vec<String> {
        self.capabilities.table_engines.clone()
    }

    fn get_column_editor_capabilities(&self) -> ColumnEditorCapabilities {
        to_column_editor_capabilities(&self.capabilities)
    }

    fn build_context_menu(&self, node_id: &str, node_type: DbNodeType) -> Vec<ContextMenuItem> {
        let mut actions =
            self.action_descriptors(node_type, DatabaseActionPlacement::ContextMenu, None);
        actions.sort_by_key(|action| context_menu_rank(node_type, action.id));
        let mut items = Vec::new();
        let mut index = 0;
        let mut last_group: Option<String> = None;

        while index < actions.len() {
            let current_group = context_menu_group(node_type, actions[index]);
            if let Some(current_group) = current_group.clone() {
                if let Some(previous_group) = &last_group {
                    if previous_group != &current_group && !items.is_empty() {
                        items.push(ContextMenuItem::separator());
                    }
                }
                last_group = Some(current_group);
            }

            if is_dump_sql_action(actions[index].id) {
                let mut sub_items = Vec::new();

                while index < actions.len() && is_dump_sql_action(actions[index].id) {
                    if let Some(item) = action_to_context_menu_item(
                        actions[index],
                        node_id,
                        self.translate_text(&actions[index].label_i18n_key),
                    ) {
                        sub_items.push(item);
                    }
                    index += 1;
                }

                if !sub_items.is_empty() {
                    items.push(ContextMenuItem::submenu(
                        translate("ImportExport.dump_sql_file"),
                        sub_items,
                    ));
                }
                continue;
            }

            if let Some(item) = action_to_context_menu_item(
                actions[index],
                node_id,
                self.translate_text(&actions[index].label_i18n_key),
            ) {
                items.push(item);
            }
            index += 1;
        }

        insert_query_table_context_menu_item(&mut items, node_type, node_id);
        items
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
                    self.translate_text(&action.label_i18n_key),
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
                    self.translate_text(&action.label_i18n_key),
                    event_fn,
                ))
            });
        current_node_buttons.chain(selected_row_buttons).collect()
    }
}

fn manifest_plugin(
    database_type: DatabaseType,
    cx: &impl AppContext,
) -> ManifestDatabaseViewPlugin {
    let plugin = cx.read_global::<db::GlobalDbState, _>(|state, _| {
        state
            .get_plugin(&database_type)
            .expect("database plugin should exist")
    });
    ManifestDatabaseViewPlugin::new(database_type, plugin.as_ref())
}

pub(crate) fn supports_database_action_for(
    database_type: DatabaseType,
    node_type: DbNodeType,
    action_id: DatabaseActionId,
    cx: &impl AppContext,
) -> bool {
    manifest_plugin(database_type, cx)
        .manifest
        .actions
        .actions
        .iter()
        .any(|action| action.id == action_id && matches_node_type(action, node_type))
}

fn action_to_context_menu_item(
    action: &DatabaseActionDescriptor,
    node_id: &str,
    label: String,
) -> Option<ContextMenuItem> {
    let event = map_tree_event(action.id, node_id)?;
    Some(if action.requires_active_connection {
        ContextMenuItem::item(label, event)
    } else {
        ContextMenuItem::always_enabled_item(label, event)
    })
}

fn is_dump_sql_action(action_id: DatabaseActionId) -> bool {
    matches!(
        action_id,
        DatabaseActionId::DumpSqlStructure
            | DatabaseActionId::DumpSqlData
            | DatabaseActionId::DumpSqlStructureAndData
    )
}

fn context_menu_rank(node_type: DbNodeType, action_id: DatabaseActionId) -> usize {
    match node_type {
        DbNodeType::Connection => match action_id {
            DatabaseActionId::RunSqlFile => 10,
            DatabaseActionId::CloseConnection => 20,
            DatabaseActionId::DeleteConnection => 30,
            DatabaseActionId::CreateDatabase => 40,
            _ => 900,
        },
        DbNodeType::Database => match action_id {
            DatabaseActionId::DesignTable => 10,
            DatabaseActionId::CreateNewQuery => 20,
            DatabaseActionId::RunSqlFile => 30,
            DatabaseActionId::DumpSqlStructure => 40,
            DatabaseActionId::DumpSqlData => 41,
            DatabaseActionId::DumpSqlStructureAndData => 42,
            DatabaseActionId::EditDatabase => 50,
            DatabaseActionId::CreateSchema => 60,
            DatabaseActionId::CloseDatabase => 70,
            DatabaseActionId::DeleteDatabase => 80,
            _ => 900,
        },
        DbNodeType::Schema => match action_id {
            DatabaseActionId::CreateNewQuery => 10,
            DatabaseActionId::RunSqlFile => 20,
            DatabaseActionId::DesignTable => 30,
            DatabaseActionId::DumpSqlStructure => 40,
            DatabaseActionId::DumpSqlData => 41,
            DatabaseActionId::DumpSqlStructureAndData => 42,
            DatabaseActionId::DeleteSchema => 50,
            _ => 900,
        },
        DbNodeType::Table => match action_id {
            DatabaseActionId::OpenTableData => 10,
            DatabaseActionId::DesignTable => 20,
            DatabaseActionId::RenameTable => 30,
            DatabaseActionId::CopyTable => 40,
            DatabaseActionId::TruncateTable => 50,
            DatabaseActionId::DeleteTable => 60,
            DatabaseActionId::DumpSqlStructure => 70,
            DatabaseActionId::DumpSqlData => 71,
            DatabaseActionId::DumpSqlStructureAndData => 72,
            DatabaseActionId::ImportData => 80,
            DatabaseActionId::ExportData => 90,
            _ => 900,
        },
        DbNodeType::View => match action_id {
            DatabaseActionId::OpenViewData => 10,
            DatabaseActionId::DeleteView => 20,
            _ => 900,
        },
        DbNodeType::Function => match action_id {
            DatabaseActionId::OpenFunction => 10,
            _ => 900,
        },
        DbNodeType::Procedure => match action_id {
            DatabaseActionId::OpenProcedure => 10,
            _ => 900,
        },
        DbNodeType::TablesFolder => match action_id {
            DatabaseActionId::DesignTable => 10,
            _ => 900,
        },
        DbNodeType::QueriesFolder => match action_id {
            DatabaseActionId::CreateNewQuery => 10,
            _ => 900,
        },
        DbNodeType::NamedQuery => match action_id {
            DatabaseActionId::OpenNamedQuery => 10,
            DatabaseActionId::RenameQuery => 20,
            DatabaseActionId::DeleteQuery => 30,
            DatabaseActionId::RevealQueryInFileManager => 40,
            _ => 900,
        },
        _ => 900,
    }
}

fn insert_query_table_context_menu_item(
    items: &mut Vec<ContextMenuItem>,
    node_type: DbNodeType,
    node_id: &str,
) {
    if !matches!(node_type, DbNodeType::Table | DbNodeType::View) {
        return;
    }

    let query_item = ContextMenuItem::item(
        translate("Query.query_table"),
        DbTreeViewEvent::CreateNewQuery {
            node_id: node_id.to_string(),
        },
    );
    let insert_index = items.len().min(2);
    items.insert(insert_index, query_item);
}

fn context_menu_group(node_type: DbNodeType, action: &DatabaseActionDescriptor) -> Option<String> {
    action.group.clone().or_else(|| {
        let group = match node_type {
            DbNodeType::Connection => match action.id {
                DatabaseActionId::RunSqlFile => Some("sql"),
                DatabaseActionId::CloseConnection | DatabaseActionId::DeleteConnection => {
                    Some("connection")
                }
                DatabaseActionId::CreateDatabase => Some("create"),
                _ => None,
            },
            DbNodeType::Database => match action.id {
                DatabaseActionId::DesignTable | DatabaseActionId::CreateNewQuery => Some("create"),
                DatabaseActionId::RunSqlFile
                | DatabaseActionId::DumpSqlStructure
                | DatabaseActionId::DumpSqlData
                | DatabaseActionId::DumpSqlStructureAndData => Some("sql"),
                DatabaseActionId::EditDatabase
                | DatabaseActionId::CreateSchema
                | DatabaseActionId::CloseDatabase
                | DatabaseActionId::DeleteDatabase => Some("database"),
                _ => None,
            },
            DbNodeType::Schema => match action.id {
                DatabaseActionId::CreateNewQuery | DatabaseActionId::DesignTable => Some("create"),
                DatabaseActionId::RunSqlFile => Some("sql"),
                DatabaseActionId::DumpSqlStructure
                | DatabaseActionId::DumpSqlData
                | DatabaseActionId::DumpSqlStructureAndData => Some("dump"),
                DatabaseActionId::DeleteSchema => Some("schema"),
                _ => None,
            },
            DbNodeType::Table => match action.id {
                DatabaseActionId::OpenTableData | DatabaseActionId::DesignTable => Some("open"),
                DatabaseActionId::RenameTable
                | DatabaseActionId::CopyTable
                | DatabaseActionId::TruncateTable
                | DatabaseActionId::DeleteTable => Some("table"),
                DatabaseActionId::DumpSqlStructure
                | DatabaseActionId::DumpSqlData
                | DatabaseActionId::DumpSqlStructureAndData => Some("dump"),
                DatabaseActionId::ImportData | DatabaseActionId::ExportData => Some("io"),
                _ => None,
            },
            DbNodeType::View => match action.id {
                DatabaseActionId::OpenViewData => Some("open"),
                DatabaseActionId::DeleteView => Some("view"),
                _ => None,
            },
            DbNodeType::Function => match action.id {
                DatabaseActionId::OpenFunction => Some("open"),
                _ => None,
            },
            DbNodeType::Procedure => match action.id {
                DatabaseActionId::OpenProcedure => Some("open"),
                _ => None,
            },
            DbNodeType::TablesFolder => Some("create"),
            DbNodeType::QueriesFolder => Some("create"),
            DbNodeType::NamedQuery => match action.id {
                DatabaseActionId::OpenNamedQuery => Some("open"),
                DatabaseActionId::RenameQuery
                | DatabaseActionId::DeleteQuery
                | DatabaseActionId::RevealQueryInFileManager => Some("query"),
                _ => None,
            },
            _ => None,
        };

        group.map(str::to_string)
    })
}

pub fn create_connection_form_for(
    database_type: DatabaseType,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DbConnectionForm> {
    let registry = IpcDriverRegistry::load_default();
    create_connection_form_for_with_registry(database_type, &registry, window, cx)
}

pub fn create_connection_form_for_with_registry(
    database_type: DatabaseType,
    registry: &IpcDriverRegistry,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DbConnectionForm> {
    if let Some(config) = duckdb_ipc_connection_form_config(&database_type, registry, cx) {
        return cx.new(|cx| DbConnectionForm::new(config, window, cx));
    }
    manifest_plugin(database_type, cx).create_connection_form(window, cx)
}

pub fn create_external_connection_form_for(
    driver_id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<DbConnectionForm>> {
    let registry = IpcDriverRegistry::load_default();
    create_external_connection_form_for_with_registry(driver_id, &registry, window, cx)
}

pub fn create_external_connection_form_for_with_registry(
    driver_id: &str,
    registry: &IpcDriverRegistry,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<DbConnectionForm>> {
    let driver = registry.find(driver_id)?;
    let config = external_form_config(&driver, cx)?;
    Some(cx.new(|cx| DbConnectionForm::new(config, window, cx)))
}

fn external_form_config(driver: &IpcDriverManifest, cx: &mut App) -> Option<DbFormConfig> {
    let database_type = DatabaseType::external(driver.id.clone());
    external_form_config_for_database_type(driver, database_type, cx)
}

fn duckdb_ipc_connection_form_config(
    database_type: &DatabaseType,
    registry: &IpcDriverRegistry,
    cx: &mut App,
) -> Option<DbFormConfig> {
    if database_type != &DatabaseType::DuckDB {
        return None;
    }
    let db_state = cx.global::<db::GlobalDbState>();
    let duckdb_plugin = db_state.get_plugin(&DatabaseType::DuckDB).ok()?;
    if duckdb_plugin.name() != DatabaseType::external("duckdb") {
        return None;
    }
    let driver = registry.find("duckdb")?;
    let plugin_type = DatabaseType::external(driver.id.clone());
    let plugin = db_state.get_plugin(&plugin_type).ok()?;
    duckdb_ipc_form_config_with_plugin(&driver, plugin.as_ref())
}

fn external_form_config_for_database_type(
    driver: &IpcDriverManifest,
    database_type: DatabaseType,
    cx: &mut App,
) -> Option<DbFormConfig> {
    let plugin_type = DatabaseType::external(driver.id.clone());
    let plugin = cx
        .global::<db::GlobalDbState>()
        .get_plugin(&plugin_type)
        .ok()?;
    external_form_config_with_plugin(driver, database_type, plugin.as_ref())
}

fn external_form_config_with_plugin(
    driver: &IpcDriverManifest,
    database_type: DatabaseType,
    plugin: &dyn DatabasePlugin,
) -> Option<DbFormConfig> {
    let mut config = raw_external_form_config_with_plugin(driver, database_type, plugin)?;
    apply_external_driver_defaults(&mut config, driver);
    Some(config)
}

fn raw_external_form_config_with_plugin(
    driver: &IpcDriverManifest,
    database_type: DatabaseType,
    plugin: &dyn DatabasePlugin,
) -> Option<DbFormConfig> {
    let mut config = if let Some(manifest) = driver.ui.form.clone() {
        let form = find_form(&manifest, DatabaseFormKind::Connection)?;
        to_connection_form_config_with_text_resolver(database_type.clone(), &form, plugin, |key| {
            translate_external_driver_text(driver, key)
        })
    } else {
        default_external_form_config_for_database_type(driver, database_type)
    };
    if config.title.trim().is_empty() {
        config.title = format!("{} ({})", translate("Common.new"), driver.name);
    }
    Some(config)
}

fn duckdb_ipc_form_config_with_plugin(
    driver: &IpcDriverManifest,
    plugin: &dyn DatabasePlugin,
) -> Option<DbFormConfig> {
    let mut config = raw_external_form_config_with_plugin(driver, DatabaseType::DuckDB, plugin)?;
    apply_duckdb_host_defaults(&mut config);
    apply_external_driver_defaults(&mut config, driver);
    Some(config)
}

fn apply_duckdb_host_defaults(config: &mut DbFormConfig) {
    let host_defaults = DbFormConfig::duckdb();
    for group in &mut config.tab_groups {
        let Some(default_group) = host_defaults
            .tab_groups
            .iter()
            .find(|default_group| default_group.name == group.name)
        else {
            continue;
        };
        for field in &mut group.fields {
            let Some(default_field) = default_group
                .fields
                .iter()
                .find(|default_field| default_field.name == field.name)
            else {
                continue;
            };
            if field.default_value.trim().is_empty() {
                field.default_value = default_field.default_value.clone();
            }
            if field.placeholder.trim().is_empty() {
                field.placeholder = default_field.placeholder.clone();
            }
        }
    }
}

fn apply_external_driver_defaults(config: &mut DbFormConfig, driver: &IpcDriverManifest) {
    if config.title.trim().is_empty() {
        config.title = format!("{} ({})", translate("Common.new"), driver.name);
    }
    apply_external_driver_empty_tab_defaults(config, driver);
    apply_external_driver_name_defaults(config, driver);
}

fn apply_external_driver_empty_tab_defaults(config: &mut DbFormConfig, driver: &IpcDriverManifest) {
    for group in &mut config.tab_groups {
        if !group.fields.is_empty() {
            continue;
        }

        if let Some(default_group) = external_driver_default_tab(driver, &group.name) {
            group.fields = default_group.fields;
        }
    }
}

fn external_driver_default_tab(driver: &IpcDriverManifest, tab_name: &str) -> Option<TabGroup> {
    match tab_name {
        "general" => find_tab(default_external_form_config(driver), "general"),
        "ssh" => find_tab(DbFormConfig::mysql(), "ssh"),
        "ssl" => find_compatible_ssl_tab(driver),
        "notes" | "remark" => find_tab(DbFormConfig::mysql(), "notes").map(|mut group| {
            group.name = tab_name.to_string();
            group
        }),
        _ => None,
    }
}

fn find_compatible_ssl_tab(driver: &IpcDriverManifest) -> Option<TabGroup> {
    find_tab(external_driver_compatible_host_form(driver), "ssl")
        .or_else(|| find_tab(DbFormConfig::mysql(), "ssl"))
}

fn external_driver_compatible_host_form(driver: &IpcDriverManifest) -> DbFormConfig {
    match driver.dialect.compatible_database_type.as_ref() {
        Some(DatabaseType::PostgreSQL) => DbFormConfig::postgres(),
        Some(DatabaseType::SQLite) => DbFormConfig::sqlite(),
        Some(DatabaseType::DuckDB) => DbFormConfig::duckdb(),
        Some(DatabaseType::MSSQL) => DbFormConfig::mssql(),
        Some(DatabaseType::Oracle) => DbFormConfig::oracle(),
        Some(DatabaseType::ClickHouse) => DbFormConfig::clickhouse(),
        Some(DatabaseType::TDengine) => DbFormConfig::tdengine(),
        _ => DbFormConfig::mysql(),
    }
}

fn find_tab(config: DbFormConfig, tab_name: &str) -> Option<TabGroup> {
    config
        .tab_groups
        .into_iter()
        .find(|group| group.name == tab_name)
}

fn apply_external_driver_name_defaults(config: &mut DbFormConfig, driver: &IpcDriverManifest) {
    for group in &mut config.tab_groups {
        for field in &mut group.fields {
            if field.name != "name" {
                continue;
            }
            if field.default_value.trim().is_empty() {
                field.default_value = driver.name.clone();
            }
            if field.placeholder.trim().is_empty() {
                field.placeholder = driver.name.clone();
            }
            return;
        }
    }
}

fn translate_external_driver_text(driver: &IpcDriverManifest, key_or_text: &str) -> String {
    if driver.locales_dir().is_some() {
        let translated = crate::t_driver(driver, key_or_text);
        if translated != key_or_text {
            return translated;
        }
    }

    let translated = translate_db_view_or_raw_for_locale(rust_i18n::locale().as_ref(), key_or_text);
    if translated != key_or_text {
        return translated;
    }

    db::translate_or_raw_for_locale(rust_i18n::locale().as_ref(), key_or_text)
}

fn translate_db_view_or_raw_for_locale(locale: &str, key_or_text: &str) -> String {
    let translated = crate::_rust_i18n_translate(locale, key_or_text).into_owned();
    let missing_with_locale = format!("{locale}.{key_or_text}");

    if translated == key_or_text || translated == missing_with_locale {
        key_or_text.to_string()
    } else {
        translated
    }
}

fn default_external_form_config(driver: &IpcDriverManifest) -> DbFormConfig {
    default_external_form_config_for_database_type(
        driver,
        DatabaseType::external(driver.id.clone()),
    )
}

fn default_external_form_config_for_database_type(
    driver: &IpcDriverManifest,
    database_type: DatabaseType,
) -> DbFormConfig {
    let t = |driver_key: &str, fallback_key: &str| -> String {
        let text = translate_external_driver_text(driver, driver_key);
        if text != driver_key {
            text
        } else {
            translate(fallback_key)
        }
    };
    let placeholder = |driver_key: &str, default: &str| -> String {
        let text = translate_external_driver_text(driver, driver_key);
        if text != driver_key {
            text
        } else {
            default.to_string()
        }
    };
    let title = {
        let text = translate_external_driver_text(driver, "connection.title");
        if text != "connection.title" {
            text
        } else {
            format!("{} ({})", translate("Common.new"), driver.name)
        }
    };

    DbFormConfig {
        db_type: database_type,
        title,
        hidden_params: HashMap::new(),
        tab_groups: vec![
            TabGroup::new("general", t("tabs.general", "ConnectionForm.general")).fields(vec![
                FormField::new(
                    "name",
                    t("fields.name.label", "ConnectionForm.connection_name"),
                    FormFieldType::Text,
                )
                .placeholder(driver.name.clone())
                .default(driver.name.clone()),
                FormField::new(
                    "host",
                    t("fields.host.label", "ConnectionForm.host"),
                    FormFieldType::Text,
                )
                .placeholder(placeholder("fields.host.placeholder", "localhost"))
                .default("localhost"),
                FormField::new(
                    "port",
                    t("fields.port.label", "ConnectionForm.port"),
                    FormFieldType::Number,
                )
                .placeholder("0")
                .default(driver.ui.default_port.unwrap_or_default().to_string()),
                FormField::new(
                    "username",
                    t("fields.username.label", "ConnectionForm.username"),
                    FormFieldType::Text,
                )
                .optional(),
                FormField::new(
                    "password",
                    t("fields.password.label", "ConnectionForm.password"),
                    FormFieldType::Password,
                )
                .optional(),
                FormField::new(
                    "database",
                    t("fields.database.label", "ConnectionForm.database"),
                    FormFieldType::Text,
                )
                .optional(),
            ]),
        ],
    }
}

pub fn create_database_editor_view_for_new(
    database_type: DatabaseType,
    connection_id: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DatabaseEditorView> {
    manifest_plugin(database_type, cx).create_database_editor_view(connection_id, window, cx)
}

pub fn create_database_editor_view_for_edit_type(
    database_type: DatabaseType,
    connection_id: String,
    database_name: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<DatabaseEditorView> {
    manifest_plugin(database_type, cx).create_database_editor_view_for_edit(
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
    manifest_plugin(database_type, cx).create_schema_editor_view(
        connection_id,
        database_name,
        window,
        cx,
    )
}

pub fn create_user_editor_view_for(
    database_type: DatabaseType,
    operation: DatabaseFormKind,
    initial: Option<DatabaseUserOperationRequest>,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<UserEditorView>> {
    manifest_plugin(database_type, cx).create_user_editor_view(operation, initial, window, cx)
}

pub fn build_context_menu_for(
    database_type: DatabaseType,
    node_id: &str,
    node_type: DbNodeType,
    cx: &impl AppContext,
) -> Vec<ContextMenuItem> {
    let mut items = manifest_plugin(database_type, cx).build_context_menu(node_id, node_type);
    append_query_directory_items(&mut items, node_id, node_type);
    append_er_diagram_item(&mut items, node_id, node_type);
    append_compare_items(&mut items, node_id, node_type);
    items
}

pub fn build_toolbar_buttons_for(
    database_type: DatabaseType,
    node_type: DbNodeType,
    data_node_type: DbNodeType,
    cx: &impl AppContext,
) -> Vec<ToolbarButton> {
    let mut buttons =
        manifest_plugin(database_type, cx).build_toolbar_buttons(node_type, data_node_type);
    append_query_directory_toolbar_buttons(&mut buttons, node_type);
    buttons
}

fn append_query_directory_items(
    items: &mut Vec<ContextMenuItem>,
    node_id: &str,
    node_type: DbNodeType,
) {
    if !matches!(
        node_type,
        DbNodeType::QueriesFolder | DbNodeType::QueryFolder
    ) {
        return;
    }

    if node_type == DbNodeType::QueryFolder {
        items.push(ContextMenuItem::item(
            translate("Query.new_query"),
            DbTreeViewEvent::CreateNewQuery {
                node_id: node_id.to_string(),
            },
        ));
    }
    if !items.is_empty() {
        items.push(ContextMenuItem::separator());
    }
    items.push(ContextMenuItem::item(
        translate("Query.new_folder"),
        DbTreeViewEvent::CreateQueryFolder {
            node_id: node_id.to_string(),
        },
    ));
    if node_type == DbNodeType::QueriesFolder {
        items.push(ContextMenuItem::item(
            translate("Query.add_sql_directory"),
            DbTreeViewEvent::AddQueryDirectory {
                node_id: node_id.to_string(),
            },
        ));
    }
    items.push(ContextMenuItem::item(
        translate("Query.import_sql"),
        DbTreeViewEvent::ImportQuerySql {
            node_id: node_id.to_string(),
        },
    ));
}

fn append_query_directory_toolbar_buttons(buttons: &mut Vec<ToolbarButton>, node_type: DbNodeType) {
    if !matches!(
        node_type,
        DbNodeType::QueriesFolder | DbNodeType::QueryFolder
    ) {
        return;
    }

    if node_type == DbNodeType::QueryFolder {
        buttons.push(ToolbarButton::current_node(
            "create-query",
            IconName::Plus,
            translate("Query.new_query"),
            |node| DatabaseObjectsEvent::CreateNewQuery { node },
        ));
    }
    buttons.push(ToolbarButton::current_node(
        "create-query-folder",
        IconName::NewFolder,
        translate("Query.new_folder"),
        |node| DatabaseObjectsEvent::TreeEvent {
            event: DbTreeViewEvent::CreateQueryFolder {
                node_id: node.id.clone(),
            },
        },
    ));
    if node_type == DbNodeType::QueriesFolder {
        buttons.push(ToolbarButton::current_node(
            "add-query-directory",
            IconName::FolderOpen,
            translate("Query.add_sql_directory"),
            |node| DatabaseObjectsEvent::TreeEvent {
                event: DbTreeViewEvent::AddQueryDirectory {
                    node_id: node.id.clone(),
                },
            },
        ));
    }
    buttons.push(ToolbarButton::current_node(
        "import-query-sql",
        IconName::ArrowDown,
        translate("Query.import_sql"),
        |node| DatabaseObjectsEvent::TreeEvent {
            event: DbTreeViewEvent::ImportQuerySql {
                node_id: node.id.clone(),
            },
        },
    ));
}

fn append_er_diagram_item(items: &mut Vec<ContextMenuItem>, node_id: &str, node_type: DbNodeType) {
    if !matches!(node_type, DbNodeType::Database | DbNodeType::Schema) {
        return;
    }
    if !items.is_empty() {
        items.push(ContextMenuItem::separator());
    }
    items.push(ContextMenuItem::item(
        translate("ErDiagram.open"),
        DbTreeViewEvent::OpenErDiagram {
            node_id: node_id.to_string(),
        },
    ));
}

fn append_compare_items(items: &mut Vec<ContextMenuItem>, node_id: &str, node_type: DbNodeType) {
    let can_compare_data = matches!(
        node_type,
        DbNodeType::Database | DbNodeType::Schema | DbNodeType::Table
    );
    let can_compare_schema = matches!(node_type, DbNodeType::Database | DbNodeType::Schema);
    if !(can_compare_data || can_compare_schema) {
        return;
    }

    if !items.is_empty() && !matches!(items.last(), Some(ContextMenuItem::Separator)) {
        items.push(ContextMenuItem::separator());
    }

    if can_compare_data {
        items.push(ContextMenuItem::item(
            "数据比较",
            DbTreeViewEvent::CompareData {
                node_id: node_id.to_string(),
            },
        ));
    }

    if can_compare_schema {
        items.push(ContextMenuItem::item(
            "结构比较",
            DbTreeViewEvent::CompareSchema {
                node_id: node_id.to_string(),
            },
        ));
    }
}

pub fn get_table_designer_capabilities_for(
    database_type: DatabaseType,
    cx: &impl AppContext,
) -> TableDesignerCapabilities {
    manifest_plugin(database_type, cx).get_table_designer_capabilities()
}

pub fn get_column_editor_capabilities_for(
    database_type: DatabaseType,
    cx: &impl AppContext,
) -> ColumnEditorCapabilities {
    manifest_plugin(database_type, cx).get_column_editor_capabilities()
}

pub fn get_engines_for(database_type: DatabaseType, cx: &impl AppContext) -> Vec<String> {
    manifest_plugin(database_type, cx).get_engines()
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
        DatabaseActionId::OpenFunction => DbTreeViewEvent::OpenFunction { node_id },
        DatabaseActionId::OpenProcedure => DbTreeViewEvent::OpenProcedure { node_id },
        DatabaseActionId::CreateNewQuery => DbTreeViewEvent::CreateNewQuery { node_id },
        DatabaseActionId::OpenNamedQuery => DbTreeViewEvent::OpenNamedQuery { node_id },
        DatabaseActionId::RenameQuery => DbTreeViewEvent::RenameQuery { node_id },
        DatabaseActionId::DeleteQuery => DbTreeViewEvent::DeleteQuery { node_id },
        DatabaseActionId::RevealQueryInFileManager => {
            DbTreeViewEvent::RevealQueryInFileManager { node_id }
        }
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
        DatabaseActionId::OpenFunction => Some(|node| DatabaseObjectsEvent::OpenFunction { node }),
        DatabaseActionId::OpenProcedure => {
            Some(|node| DatabaseObjectsEvent::OpenProcedure { node })
        }
        DatabaseActionId::CreateNewQuery => {
            Some(|node| DatabaseObjectsEvent::CreateNewQuery { node })
        }
        DatabaseActionId::OpenNamedQuery => {
            Some(|node| DatabaseObjectsEvent::OpenNamedQuery { node })
        }
        DatabaseActionId::RenameQuery => Some(|node| DatabaseObjectsEvent::RenameQuery { node }),
        DatabaseActionId::DeleteQuery => Some(|node| DatabaseObjectsEvent::DeleteQuery { node }),
        DatabaseActionId::RevealQueryInFileManager => {
            Some(|node| DatabaseObjectsEvent::RevealQueryInFileManager { node })
        }
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
        | DatabaseActionId::OpenFunction
        | DatabaseActionId::OpenProcedure
        | DatabaseActionId::RenameQuery => IconName::Edit,
        DatabaseActionId::RevealQueryInFileManager => IconName::FolderOpen,
        DatabaseActionId::OpenTableData
        | DatabaseActionId::OpenViewData
        | DatabaseActionId::OpenNamedQuery => IconName::Eye,
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
        DatabaseActionId::OpenFunction => "open-function",
        DatabaseActionId::OpenProcedure => "open-procedure",
        DatabaseActionId::CreateNewQuery => "create-query",
        DatabaseActionId::OpenNamedQuery => "open-query",
        DatabaseActionId::RenameQuery => "rename-query",
        DatabaseActionId::DeleteQuery => "delete-query",
        DatabaseActionId::RevealQueryInFileManager => "reveal-query-in-file-manager",
        DatabaseActionId::RunSqlFile => "run-sql-file",
        DatabaseActionId::ImportData => "import-data",
        DatabaseActionId::ExportData => "export-data",
        DatabaseActionId::DumpSqlStructure => "dump-sql-structure",
        DatabaseActionId::DumpSqlData => "dump-sql-data",
        DatabaseActionId::DumpSqlStructureAndData => "dump-sql-structure-and-data",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{ExternalDatabasePlugin, IpcDriverEntry, IpcDriverManifest, IpcDriverTransport};
    use db::mysql::MySqlPlugin;
    use db::plugin_manifest::{
        DatabaseFormField, DatabaseFormFieldType, DatabaseFormManifest, DatabaseFormTab,
    };
    use gpui_component::IconNamed;
    use std::path::PathBuf;

    fn mysql_manifest_plugin() -> ManifestDatabaseViewPlugin {
        let plugin = MySqlPlugin::new();
        ManifestDatabaseViewPlugin::new(DatabaseType::MySQL, &plugin)
    }

    fn has_label(items: &[ContextMenuItem], expected: &str) -> bool {
        items.iter().any(|item| match item {
            ContextMenuItem::Item { label, .. } => label == expected,
            ContextMenuItem::Separator => false,
            ContextMenuItem::Submenu { label, items, .. } => {
                label == expected || has_label(items, expected)
            }
        })
    }

    fn toolbar_ids(buttons: &[ToolbarButton]) -> Vec<&'static str> {
        buttons.iter().map(|button| button.id).collect()
    }

    fn action_descriptor(id: DatabaseActionId) -> DatabaseActionDescriptor {
        DatabaseActionDescriptor {
            id,
            label_i18n_key: String::new(),
            icon: None,
            targets: Vec::new(),
            placement: DatabaseActionPlacement::Both,
            requires_active_connection: false,
            group: None,
            submenu_of: None,
            toolbar_scope: None,
        }
    }

    fn field_names(tab_group: &TabGroup) -> Vec<&str> {
        tab_group
            .fields
            .iter()
            .map(|field| field.name.as_str())
            .collect()
    }

    fn tab_fields<'a>(config: &'a DbFormConfig, tab_name: &str) -> Vec<&'a str> {
        field_names(
            config
                .tab_groups
                .iter()
                .find(|group| group.name == tab_name)
                .expect("tab should exist"),
        )
    }

    fn config_field<'a>(config: &'a DbFormConfig, field_name: &str) -> &'a FormField {
        config
            .tab_groups
            .iter()
            .flat_map(|group| group.fields.iter())
            .find(|field| field.name == field_name)
            .expect("field should exist")
    }

    fn manifest_field(
        id: &str,
        label_i18n_key: &str,
        field_type: DatabaseFormFieldType,
    ) -> DatabaseFormField {
        DatabaseFormField {
            id: id.into(),
            label_i18n_key: label_i18n_key.into(),
            field_type,
            required: true,
            default_value: None,
            placeholder_i18n_key: None,
            help_i18n_key: None,
            options: Vec::new(),
            options_source: None,
            visible_when: Vec::new(),
            default_when: Vec::new(),
            disabled_when_editing: false,
            rows: None,
            min: None,
            max: None,
        }
    }

    fn duckdb_driver_form() -> DatabaseUiManifest {
        DatabaseUiManifest {
            forms: vec![DatabaseFormManifest {
                kind: DatabaseFormKind::Connection,
                title_i18n_key: "connection.title".into(),
                submit_i18n_key: "Common.save".into(),
                tabs: vec![DatabaseFormTab {
                    id: "general".into(),
                    label_i18n_key: "ConnectionForm.general".into(),
                    fields: vec![
                        manifest_field(
                            "name",
                            "ConnectionForm.connection_name",
                            DatabaseFormFieldType::Text,
                        ),
                        manifest_field(
                            "host",
                            "database.connection.field.host",
                            DatabaseFormFieldType::FilePath,
                        ),
                    ],
                }],
            }],
            ..DatabaseUiManifest::default()
        }
    }

    fn demo_driver() -> IpcDriverManifest {
        IpcDriverManifest {
            id: "demo".into(),
            name: "DemoDB".into(),
            api: "database".into(),
            category: None,
            description: String::new(),
            version: String::new(),
            engines: Default::default(),
            compatibility: serde_json::Value::Null,
            entry: IpcDriverEntry {
                command: "driver".into(),
                commands: Default::default(),
                args: Vec::new(),
                working_dir: None,
                env_from_config: Default::default(),
            },
            transport: IpcDriverTransport::local_socket("demo.sock"),
            dialect: Default::default(),
            capabilities: None,
            connection: Default::default(),
            methods: Vec::new(),
            ui: Default::default(),
            manifest_dir: PathBuf::from("."),
        }
    }

    fn demo_driver_with_locales(root: &std::path::Path) -> IpcDriverManifest {
        let locales_dir = root.join("locales");
        std::fs::create_dir_all(&locales_dir).unwrap();
        let locale = rust_i18n::locale().to_string();
        let content = r#"
connection:
  title: "Driver Connection"
database:
  connection:
    field:
      host: "Driver Host"
"#;
        std::fs::write(locales_dir.join(format!("{locale}.yml")), content).unwrap();
        std::fs::write(locales_dir.join("en.yml"), content).unwrap();
        let mut driver = demo_driver();
        driver.ui.locales_dir = Some("locales".to_string());
        driver.manifest_dir = root.to_path_buf();
        driver
    }

    fn demo_driver_with_database_locale(root: &std::path::Path) -> IpcDriverManifest {
        let locales_dir = root.join("locales");
        std::fs::create_dir_all(&locales_dir).unwrap();
        let locale = rust_i18n::locale().to_string();
        let content = r#"
driver:
  database:
    create: "New Driver Group"
    delete: "Delete Driver Group"
    name: "Driver Group Name"
    placeholder: "root.demo"
"#;
        std::fs::write(locales_dir.join(format!("{locale}.yml")), content).unwrap();
        std::fs::write(locales_dir.join("en.yml"), content).unwrap();

        let mut driver = demo_driver();
        driver.ui.locales_dir = Some("locales".to_string());
        driver.ui.form = Some(DatabaseUiManifest {
            forms: vec![DatabaseFormManifest {
                kind: DatabaseFormKind::CreateDatabase,
                title_i18n_key: "driver.database.create".into(),
                submit_i18n_key: "Common.create".into(),
                tabs: vec![DatabaseFormTab {
                    id: "general".into(),
                    label_i18n_key: "ConnectionForm.general".into(),
                    fields: vec![DatabaseFormField {
                        placeholder_i18n_key: Some("driver.database.placeholder".into()),
                        ..manifest_field(
                            "name",
                            "driver.database.name",
                            DatabaseFormFieldType::Text,
                        )
                    }],
                }],
            }],
            actions: db::plugin_manifest::DatabaseActionManifest {
                actions: vec![db::plugin_manifest::DatabaseActionDescriptor {
                    id: DatabaseActionId::CreateDatabase,
                    label_i18n_key: "driver.database.create".into(),
                    icon: None,
                    targets: vec![db::plugin_manifest::DatabaseActionTarget {
                        node_type: DbNodeType::Connection,
                    }],
                    placement: DatabaseActionPlacement::Both,
                    requires_active_connection: true,
                    group: None,
                    submenu_of: None,
                    toolbar_scope: Some(DatabaseActionToolbarScope::CurrentNode),
                }],
            },
            ..DatabaseUiManifest::default()
        });
        driver.manifest_dir = root.to_path_buf();
        driver
    }

    fn to_database_form_config_for_test(
        driver: &IpcDriverManifest,
        plugin: &dyn DatabasePlugin,
        form: DatabaseFormManifest,
    ) -> DbFormConfig {
        to_connection_form_config_with_text_resolver(
            DatabaseType::external(driver.id.clone()),
            &form,
            plugin,
            |key| translate_external_driver_text(driver, key),
        )
    }

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("onetcli-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn open_named_query_uses_open_icon_instead_of_edit_icon() {
        let action = action_descriptor(DatabaseActionId::OpenNamedQuery);

        assert_eq!(toolbar_icon(&action).path(), IconName::Eye.path());
        assert_ne!(toolbar_icon(&action).path(), IconName::Edit.path());
    }

    #[test]
    fn open_procedure_maps_to_tree_and_object_events() {
        assert!(matches!(
            map_tree_event(DatabaseActionId::OpenProcedure, "procedure-1"),
            Some(DbTreeViewEvent::OpenProcedure { node_id }) if node_id == "procedure-1"
        ));

        let mapper = map_objects_event(DatabaseActionId::OpenProcedure)
            .expect("procedure action should map to an object event");
        let node = db::DbNode::new(
            "procedure-1",
            "sync_orders",
            DbNodeType::Procedure,
            "conn-1".to_string(),
            DatabaseType::MySQL,
        );
        assert!(matches!(
            mapper(node),
            DatabaseObjectsEvent::OpenProcedure { node } if node.id == "procedure-1"
        ));
    }

    #[test]
    fn open_function_maps_to_tree_and_object_events() {
        assert!(matches!(
            map_tree_event(DatabaseActionId::OpenFunction, "function-1"),
            Some(DbTreeViewEvent::OpenFunction { node_id }) if node_id == "function-1"
        ));

        let mapper = map_objects_event(DatabaseActionId::OpenFunction)
            .expect("function action should map to an object event");
        let node = db::DbNode::new(
            "function-1",
            "calculate_total",
            DbNodeType::Function,
            "conn-1".to_string(),
            DatabaseType::MySQL,
        );
        assert!(matches!(
            mapper(node),
            DatabaseObjectsEvent::OpenFunction { node } if node.id == "function-1"
        ));
    }

    #[test]
    fn open_function_uses_edit_icon_and_stable_action_id() {
        let action = action_descriptor(DatabaseActionId::OpenFunction);

        assert_eq!(toolbar_icon(&action).path(), IconName::Edit.path());
        assert_eq!("open-function", action_id(&action));
    }

    #[test]
    fn open_procedure_uses_edit_icon_and_stable_action_id() {
        let action = action_descriptor(DatabaseActionId::OpenProcedure);

        assert_eq!(toolbar_icon(&action).path(), IconName::Edit.path());
        assert_eq!("open-procedure", action_id(&action));
    }

    #[test]
    fn external_driver_text_uses_driver_locale_then_app_locale() {
        let temp = temp_test_dir("driver-i18n");
        let driver = demo_driver_with_locales(&temp);

        assert_eq!(
            "Driver Connection",
            translate_external_driver_text(&driver, "connection.title")
        );
        assert_eq!(
            "Driver Host",
            translate_external_driver_text(&driver, "database.connection.field.host")
        );
        assert_eq!(
            translate("ConnectionForm.general"),
            translate_external_driver_text(&driver, "ConnectionForm.general")
        );
        assert_eq!(
            translate("Table.new_table"),
            translate_external_driver_text(&driver, "Table.new_table")
        );
        assert_eq!(
            "literal text",
            translate_external_driver_text(&driver, "literal text")
        );
    }

    #[test]
    fn query_root_context_menu_exposes_directory_actions() {
        let mut items = Vec::new();
        append_query_directory_items(&mut items, "queries", DbNodeType::QueriesFolder);

        assert!(has_label(&items, &translate("Query.new_folder")));
        assert!(has_label(&items, &translate("Query.add_sql_directory")));
        assert!(has_label(&items, &translate("Query.import_sql")));
    }

    #[test]
    fn query_subdirectory_context_menu_exposes_nested_actions_only() {
        let mut items = Vec::new();
        append_query_directory_items(&mut items, "reports", DbNodeType::QueryFolder);

        assert!(has_label(&items, &translate("Query.new_query")));
        assert!(has_label(&items, &translate("Query.new_folder")));
        assert!(has_label(&items, &translate("Query.import_sql")));
        assert!(!has_label(&items, &translate("Query.add_sql_directory")));
    }

    #[test]
    fn query_directory_toolbar_buttons_match_root_and_subdirectory_capabilities() {
        let mut root_buttons = Vec::new();
        append_query_directory_toolbar_buttons(&mut root_buttons, DbNodeType::QueriesFolder);
        assert_eq!(
            vec![
                "create-query-folder",
                "add-query-directory",
                "import-query-sql"
            ],
            toolbar_ids(&root_buttons)
        );

        let mut folder_buttons = Vec::new();
        append_query_directory_toolbar_buttons(&mut folder_buttons, DbNodeType::QueryFolder);
        assert_eq!(
            vec!["create-query", "create-query-folder", "import-query-sql"],
            toolbar_ids(&folder_buttons)
        );
    }

    #[test]
    fn default_external_form_config_uses_driver_title_locale() {
        let temp = temp_test_dir("driver-title-i18n");
        let driver = demo_driver_with_locales(&temp);

        let config = default_external_form_config(&driver);

        assert_eq!("Driver Connection", config.title);
    }

    #[test]
    fn external_driver_database_form_uses_driver_locale() {
        let temp = temp_test_dir("driver-database-form-i18n");
        let driver = demo_driver_with_database_locale(&temp);
        let plugin = ExternalDatabasePlugin::for_driver(driver.clone());
        let manifest = driver
            .ui
            .form
            .as_ref()
            .and_then(|form| {
                form.forms
                    .iter()
                    .find(|form| form.kind == DatabaseFormKind::CreateDatabase)
                    .cloned()
            })
            .expect("driver should expose create database form");

        let config = to_database_form_config_for_test(&driver, &plugin, manifest);

        assert_eq!("New Driver Group", config.title);
        assert_eq!("Driver Group Name", config_field(&config, "name").label);
    }

    #[test]
    fn external_driver_context_menu_uses_driver_locale() {
        let temp = temp_test_dir("driver-action-i18n");
        let driver = demo_driver_with_database_locale(&temp);
        let plugin = ExternalDatabasePlugin::for_driver(driver.clone());
        let view_plugin = ManifestDatabaseViewPlugin::new(DatabaseType::external("demo"), &plugin);

        let items = view_plugin.build_context_menu("node-1", DbNodeType::Connection);

        assert!(has_label(&items, "New Driver Group"));
    }

    #[test]
    fn external_driver_context_menu_falls_back_to_host_locale() {
        let temp = temp_test_dir("driver-action-host-i18n");
        let mut driver = demo_driver_with_database_locale(&temp);
        if let Some(form) = driver.ui.form.as_mut() {
            if let Some(action) = form.actions.actions.first_mut() {
                action.label_i18n_key = "Table.new_table".into();
            }
        }
        let plugin = ExternalDatabasePlugin::for_driver(driver.clone());
        let view_plugin = ManifestDatabaseViewPlugin::new(DatabaseType::external("demo"), &plugin);

        let items = view_plugin.build_context_menu("node-1", DbNodeType::Connection);

        assert!(has_label(&items, &translate("Table.new_table")));
    }

    #[test]
    fn duckdb_ipc_form_keeps_builtin_type_and_applies_host_defaults() {
        let temp = temp_test_dir("duckdb-ipc-host-defaults");
        let mut driver = demo_driver_with_locales(&temp);
        driver.id = "duckdb".into();
        driver.name = "DuckDB".into();
        driver.ui.form = Some(duckdb_driver_form());
        let plugin = ExternalDatabasePlugin::for_driver(driver.clone());

        let config = duckdb_ipc_form_config_with_plugin(&driver, &plugin)
            .expect("DuckDB IPC form should be converted");
        let name = config_field(&config, "name");
        let host = config_field(&config, "host");

        assert_eq!(DatabaseType::DuckDB, config.db_type);
        assert_eq!("Driver Connection", config.title);
        assert_eq!("Local DuckDB", name.default_value);
        assert_eq!("My DuckDB Database", name.placeholder);
        assert_eq!("Driver Host", host.label);
        assert!(host.default_value.ends_with("onetcli_default.duckdb"));
        assert_eq!("/path/to/database.duckdb", host.placeholder);
    }

    #[test]
    fn external_driver_form_defaults_preserve_manifest_title() {
        let mut config = DbFormConfig {
            db_type: DatabaseType::external("demo"),
            title: "Driver Connection".into(),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", "General").field(
                    FormField::new("name", "Name", FormFieldType::Text)
                        .placeholder("")
                        .default(""),
                ),
            ],
        };

        apply_external_driver_defaults(&mut config, &demo_driver());

        assert_eq!("Driver Connection", config.title);
        assert_eq!(
            None,
            config.hidden_params.get("external_driver_id"),
            "external driver identity must be stored in DatabaseType, not hidden params"
        );
        assert_eq!("DemoDB", config.tab_groups[0].fields[0].default_value);
        assert_eq!("DemoDB", config.tab_groups[0].fields[0].placeholder);
    }

    #[test]
    fn external_driver_empty_manifest_tabs_use_host_defaults() {
        let mut driver = demo_driver();
        driver.dialect.compatible_database_type = Some(DatabaseType::PostgreSQL);
        let mut config = DbFormConfig {
            db_type: DatabaseType::external("demo"),
            title: "Driver Connection".into(),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", "General"),
                TabGroup::new("ssl", "SSL"),
                TabGroup::new("ssh", "SSH"),
                TabGroup::new("remark", "Remark"),
            ],
        };

        apply_external_driver_defaults(&mut config, &driver);

        assert_eq!(
            tab_fields(&config, "general"),
            vec!["name", "host", "port", "username", "password", "database"]
        );
        assert_eq!(
            tab_fields(&config, "ssl"),
            vec![
                "ssl_mode",
                "ssl_root_cert_path",
                "ssl_accept_invalid_certs",
                "ssl_accept_invalid_hostnames"
            ]
        );
        assert_eq!(
            tab_fields(&config, "ssh"),
            vec![
                "ssh_tunnel_enabled",
                "ssh_connection_id",
                "ssh_host",
                "ssh_port",
                "ssh_username",
                "ssh_auth_type",
                "ssh_password",
                "ssh_private_key_path",
                "ssh_private_key_content",
                "ssh_private_key_passphrase",
                "ssh_target_host",
                "ssh_target_port"
            ]
        );
        assert_eq!(tab_fields(&config, "remark"), vec!["remark"]);
    }

    #[test]
    fn external_driver_non_empty_manifest_tab_keeps_driver_fields() {
        let mut config = DbFormConfig {
            db_type: DatabaseType::external("demo"),
            title: "Driver Connection".into(),
            hidden_params: HashMap::new(),
            tab_groups: vec![
                TabGroup::new("general", "General")
                    .field(FormField::new("dsn", "DSN", FormFieldType::Text).default("demo-dsn")),
            ],
        };

        apply_external_driver_defaults(&mut config, &demo_driver());

        assert_eq!(tab_fields(&config, "general"), vec!["dsn"]);
    }

    #[test]
    fn external_driver_empty_ssh_tab_uses_host_defaults_for_file_compatible_driver() {
        let mut driver = demo_driver();
        driver.dialect.compatible_database_type = Some(DatabaseType::DuckDB);
        let mut config = DbFormConfig {
            db_type: DatabaseType::external("duckdb"),
            title: "Driver Connection".into(),
            hidden_params: HashMap::new(),
            tab_groups: vec![TabGroup::new("ssh", "SSH")],
        };

        apply_external_driver_defaults(&mut config, &driver);

        assert_eq!(
            tab_fields(&config, "ssh"),
            vec![
                "ssh_tunnel_enabled",
                "ssh_connection_id",
                "ssh_host",
                "ssh_port",
                "ssh_username",
                "ssh_auth_type",
                "ssh_password",
                "ssh_private_key_path",
                "ssh_private_key_content",
                "ssh_private_key_passphrase",
                "ssh_target_host",
                "ssh_target_port"
            ]
        );
    }

    #[test]
    fn mysql_table_context_menu_keeps_design_table_action() {
        let items = mysql_manifest_plugin().build_context_menu("node-1", DbNodeType::Table);

        assert!(
            has_label(&items, &translate("Table.design_table")),
            "设计表菜单项不应因 toolbar_scope 过滤而丢失"
        );
    }

    #[test]
    fn table_context_menu_includes_query_table_action() {
        let items = mysql_manifest_plugin().build_context_menu("node-1", DbNodeType::Table);

        assert!(has_label(&items, &translate("Query.query_table")));
    }

    #[test]
    fn mysql_table_context_menu_keeps_dump_sql_submenu() {
        let items = mysql_manifest_plugin().build_context_menu("node-1", DbNodeType::Table);

        let dump_submenu = items.iter().find_map(|item| match item {
            ContextMenuItem::Submenu { label, items, .. }
                if label == &translate("ImportExport.dump_sql_file") =>
            {
                Some(items)
            }
            _ => None,
        });

        let dump_submenu = dump_submenu.expect("导出 SQL 二级菜单不应丢失");
        assert!(
            has_label(dump_submenu, &translate("ImportExport.export_structure")),
            "导出结构菜单项应存在于二级菜单中"
        );
        assert!(
            has_label(dump_submenu, &translate("ImportExport.export_data")),
            "导出数据菜单项应存在于二级菜单中"
        );
        assert!(
            has_label(
                dump_submenu,
                &translate("ImportExport.export_structure_and_data")
            ),
            "导出结构和数据菜单项应存在于二级菜单中"
        );
    }

    #[test]
    fn compare_context_menu_items_are_available() {
        let mut table_items = Vec::new();
        append_compare_items(&mut table_items, "table-1", DbNodeType::Table);
        assert!(has_label(&table_items, "数据比较"));

        let mut database_items = Vec::new();
        append_compare_items(&mut database_items, "database-1", DbNodeType::Database);
        assert!(has_label(&database_items, "数据比较"));
        assert!(has_label(&database_items, "结构比较"));

        let mut schema_items = Vec::new();
        append_compare_items(&mut schema_items, "schema-1", DbNodeType::Schema);
        assert!(has_label(&schema_items, "数据比较"));
        assert!(has_label(&schema_items, "结构比较"));
    }

    #[test]
    fn mysql_database_context_menu_restores_legacy_order_and_separators() {
        let items = mysql_manifest_plugin().build_context_menu("node-1", DbNodeType::Database);

        let labels: Vec<String> = items
            .iter()
            .map(|item| match item {
                ContextMenuItem::Item { label, .. } => label.clone(),
                ContextMenuItem::Separator => "---".to_string(),
                ContextMenuItem::Submenu { label, .. } => format!("submenu:{label}"),
            })
            .collect();

        let expected = vec![
            translate("Table.new_table"),
            translate("Query.new_query"),
            "---".to_string(),
            translate("ImportExport.run_sql_file"),
            format!("submenu:{}", translate("ImportExport.dump_sql_file")),
            "---".to_string(),
            translate("Database.edit_database"),
            translate("Database.close_database"),
            translate("Database.delete_database"),
        ];

        assert_eq!(labels, expected);
    }
}
