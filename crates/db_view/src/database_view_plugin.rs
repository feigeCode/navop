use db::DbNodeType;
use db::ipc::{IpcDriverManifest, IpcDriverRegistry};
use db::plugin::DatabasePlugin;
use db::plugin_manifest::{
    DatabaseActionDescriptor, DatabaseActionId, DatabaseActionPlacement,
    DatabaseActionToolbarScope, DatabaseCapabilities, DatabaseFormKind, DatabaseUiManifest,
};
use gpui::{App, AppContext, Entity, Window};
use gpui_component::IconName;
use one_core::storage::DatabaseType;

use crate::common::db_connection_form::{
    DbConnectionForm, DbFormConfig, FormField, FormFieldType, TabGroup,
};
use crate::common::manifest_bridge::{
    find_form, matches_node_type, to_column_editor_capabilities, to_connection_form_config,
    to_connection_form_config_with_text_resolver, to_table_designer_capabilities, translate,
};
use crate::common::{DatabaseEditorView, GenericDatabaseForm, GenericSchemaForm, SchemaEditorView};
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
}

impl ManifestDatabaseViewPlugin {
    fn new(database_type: DatabaseType, plugin: &dyn DatabasePlugin) -> Self {
        Self {
            database_type,
            manifest: plugin.ui_manifest(),
            capabilities: plugin.capabilities(),
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
        cx.new(|cx| {
            let form =
                cx.new(|cx| GenericDatabaseForm::new(database_type.clone(), manifest, window, cx));
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
        cx.new(|cx| {
            let form =
                cx.new(|cx| GenericDatabaseForm::new(database_type.clone(), manifest, window, cx));
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
                    if let Some(item) = action_to_context_menu_item(actions[index], node_id) {
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

            if let Some(item) = action_to_context_menu_item(actions[index], node_id) {
                items.push(item);
            }
            index += 1;
        }

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

fn action_to_context_menu_item(
    action: &DatabaseActionDescriptor,
    node_id: &str,
) -> Option<ContextMenuItem> {
    let label = translate(&action.label_i18n_key);
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
            _ => 900,
        },
        _ => 900,
    }
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
            DbNodeType::TablesFolder => Some("create"),
            DbNodeType::QueriesFolder => Some("create"),
            DbNodeType::NamedQuery => match action.id {
                DatabaseActionId::OpenNamedQuery => Some("open"),
                DatabaseActionId::RenameQuery | DatabaseActionId::DeleteQuery => Some("query"),
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
    manifest_plugin(database_type, cx).create_connection_form(window, cx)
}

pub fn create_external_connection_form_for(
    driver_id: &str,
    window: &mut Window,
    cx: &mut App,
) -> Option<Entity<DbConnectionForm>> {
    let driver = IpcDriverRegistry::load_default().find(driver_id)?;
    let config = external_form_config(&driver, cx)?;
    Some(cx.new(|cx| DbConnectionForm::new(config, window, cx)))
}

fn external_form_config(driver: &IpcDriverManifest, cx: &mut App) -> Option<DbFormConfig> {
    let database_type = DatabaseType::external(driver.id.clone());
    let plugin = cx
        .global::<db::GlobalDbState>()
        .get_plugin(&database_type)
        .ok()?;
    let mut config = if let Some(manifest) = driver.ui.form.clone() {
        let form = find_form(&manifest, DatabaseFormKind::Connection)?;
        to_connection_form_config_with_text_resolver(
            database_type.clone(),
            &form,
            plugin.as_ref(),
            |key| translate_external_driver_text(driver, key),
        )
    } else {
        default_external_form_config(driver)
    };
    apply_external_driver_defaults(&mut config, driver);
    Some(config)
}

fn apply_external_driver_defaults(config: &mut DbFormConfig, driver: &IpcDriverManifest) {
    if config.title.trim().is_empty() {
        config.title = format!("{} ({})", translate("Common.new"), driver.name);
    }
    apply_external_driver_name_defaults(config, driver);
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

    db::translate_or_raw_for_locale(rust_i18n::locale().as_ref(), key_or_text)
}

fn default_external_form_config(driver: &IpcDriverManifest) -> DbFormConfig {
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
        db_type: DatabaseType::external(driver.id.clone()),
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

pub fn build_context_menu_for(
    database_type: DatabaseType,
    node_id: &str,
    node_type: DbNodeType,
    cx: &impl AppContext,
) -> Vec<ContextMenuItem> {
    let mut items = manifest_plugin(database_type, cx).build_context_menu(node_id, node_type);
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
    manifest_plugin(database_type, cx).build_toolbar_buttons(node_type, data_node_type)
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

#[cfg(feature = "compare")]
fn append_compare_items(items: &mut Vec<ContextMenuItem>, node_id: &str, node_type: DbNodeType) {
    // 数据比较：仅对表显示
    if matches!(node_type, DbNodeType::Table) {
        if !items.is_empty() {
            items.push(ContextMenuItem::separator());
        }
        items.push(ContextMenuItem::item(
            "数据比较",
            DbTreeViewEvent::CompareData {
                node_id: node_id.to_string(),
            },
        ));
    }

    // 结构比较：对数据库和 Schema 显示
    if matches!(node_type, DbNodeType::Database | DbNodeType::Schema) {
        if items.is_empty() || !matches!(items.last(), Some(ContextMenuItem::Separator)) {
            items.push(ContextMenuItem::separator());
        }
        items.push(ContextMenuItem::item(
            "结构比较",
            DbTreeViewEvent::CompareSchema {
                node_id: node_id.to_string(),
            },
        ));
    }
}

#[cfg(not(feature = "compare"))]
fn append_compare_items(_items: &mut Vec<ContextMenuItem>, _node_id: &str, _node_type: DbNodeType) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{IpcDriverEntry, IpcDriverManifest, IpcDriverTransport};
    use db::mysql::MySqlPlugin;
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

    fn demo_driver() -> IpcDriverManifest {
        IpcDriverManifest {
            id: "demo".into(),
            name: "DemoDB".into(),
            category: None,
            description: String::new(),
            version: String::new(),
            entry: IpcDriverEntry {
                command: "driver".into(),
                args: Vec::new(),
                working_dir: None,
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

    fn temp_test_dir(name: &str) -> std::path::PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("onetcli-{name}-{}-{nanos}", std::process::id()))
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
            "literal text",
            translate_external_driver_text(&driver, "literal text")
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
    fn mysql_table_context_menu_keeps_design_table_action() {
        let items = mysql_manifest_plugin().build_context_menu("node-1", DbNodeType::Table);

        assert!(
            has_label(&items, &translate("Table.design_table")),
            "设计表菜单项不应因 toolbar_scope 过滤而丢失"
        );
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
    #[cfg(not(feature = "compare"))]
    fn compare_context_menu_items_are_hidden_without_compare_feature() {
        let mut table_items = Vec::new();
        append_compare_items(&mut table_items, "table-1", DbNodeType::Table);
        assert!(
            !has_label(&table_items, "数据比较"),
            "默认构建不应暴露未完成的数据比较入口"
        );

        let mut database_items = Vec::new();
        append_compare_items(&mut database_items, "database-1", DbNodeType::Database);
        assert!(
            !has_label(&database_items, "结构比较"),
            "默认构建不应暴露未完成的结构比较入口"
        );
    }

    #[test]
    #[cfg(feature = "compare")]
    fn compare_context_menu_items_are_available_with_compare_feature() {
        let mut table_items = Vec::new();
        append_compare_items(&mut table_items, "table-1", DbNodeType::Table);
        assert!(has_label(&table_items, "数据比较"));

        let mut database_items = Vec::new();
        append_compare_items(&mut database_items, "database-1", DbNodeType::Database);
        assert!(has_label(&database_items, "结构比较"));
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
