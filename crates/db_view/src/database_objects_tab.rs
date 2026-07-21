use crate::database_table_columns::{
    render_table_column_resize_handle, resize_table_column, ui_columns_from_object_columns,
};
use crate::database_view_plugin::{
    ContextMenuEvent, ContextMenuItem, ToolbarButtonType, build_context_menu_for,
    build_toolbar_buttons_for,
};
use crate::db_tree_view::{DbTreeViewEvent, get_icon_for_node_type};
use crate::extension_menu::{
    DbTreeExtensionActionContext, DbTreeExtensionMenuContext, DbTreeExtensionMenuItem,
    DbTreeExtensionMenuRegistry, GlobalDbTreeExtensionActionHandler,
};
use crate::search_shortcut::{
    DB_SEARCH_CONTEXT, FocusSearchInput, OpenSelectedTableQuery, focus_search_input,
};
use db::{DbNode, DbNodeType, GlobalDbState, ObjectView, ObjectViewColumn};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    HighlightStyle, InteractiveElement, IntoElement, ListSizingBehavior, MouseButton,
    MouseDownEvent, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    StyledText, Subscription, WeakEntity, Window, div, px, uniform_list,
};
use gpui_component::button::Button;
use gpui_component::input::{Input, InputEvent, InputState};
use gpui_component::menu::{ContextMenuExt, PopupMenu, PopupMenuItem};
use gpui_component::notification::Notification;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, table::Column, tooltip::Tooltip, v_flex,
};
use gpui_component::{InteractiveElementExt, WindowExt};
use one_core::storage::manager::get_queries_dir;
use one_core::storage::{
    ActiveConnections, ConnectionRepository, DatabaseType, DbConnectionConfig, GlobalStorageState,
    StorageManager, Workspace,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use one_core::utils::debouncer::Debouncer;
use rust_i18n::t;
use std::collections::{HashMap, HashSet};
use std::ops::Range;
use std::sync::Arc;
use std::time::Duration;
use tracing::log::warn;

fn format_timestamp(ts: i64) -> String {
    use chrono::{DateTime, Local};
    if let Some(dt) = DateTime::from_timestamp_millis(ts) {
        let local: DateTime<Local> = dt.into();
        local.format("%Y-%m-%d %H:%M:%S").to_string()
    } else {
        "".to_string()
    }
}

pub(crate) fn object_name_highlight_ranges(text: &str, query: &str) -> Vec<Range<usize>> {
    if query.is_empty() {
        return Vec::new();
    }

    let text_lower = text.to_lowercase();
    let query_lower = query.to_lowercase();
    let mut ranges = Vec::new();
    let mut search_start = 0;

    while let Some(offset) = text_lower[search_start..].find(&query_lower) {
        let start = search_start + offset;
        let end = start + query_lower.len();
        if text.is_char_boundary(start) && text.is_char_boundary(end) {
            ranges.push(start..end);
        }
        search_start = end;
    }

    ranges
}

fn render_object_name_text(cell_value: String, search_query: &str, cx: &App) -> AnyElement {
    if search_query.is_empty() {
        return div().child(cell_value).into_any_element();
    }

    let text = SharedString::from(cell_value);
    let highlights = object_name_highlight_ranges(&text, search_query)
        .into_iter()
        .map(|range| {
            (
                range,
                HighlightStyle {
                    color: Some(cx.theme().blue),
                    ..Default::default()
                },
            )
        })
        .collect::<Vec<_>>();

    div()
        .child(StyledText::new(&text).with_highlights(highlights))
        .into_any_element()
}

pub(crate) fn apply_object_context_menu_target(
    selected_indices: &mut HashSet<usize>,
    context_menu_row: &mut Option<usize>,
    row_ix: usize,
) {
    selected_indices.clear();
    selected_indices.insert(row_ix);
    *context_menu_row = Some(row_ix);
}

/// 数据库对象面板事件 - 统一的表格交互事件
#[derive(Clone, Debug)]
pub enum DatabaseObjectsEvent {
    /// 将对象页签菜单动作转发到数据库树的统一事件处理链
    TreeEvent { event: DbTreeViewEvent },

    /// 刷新当前视图
    Refresh { node: DbNode },

    /// 将数据库添加到树视图并展开
    AddDatabaseToTree { node: DbNode },

    /// 新建数据库
    CreateDatabase { node: DbNode },

    /// 编辑数据库
    EditDatabase { node: DbNode },

    /// 删除数据库
    DeleteDatabase { node: DbNode },

    /// 删除连接
    DeleteConnection { node: DbNode },

    /// 关闭连接
    CloseConnection { node: DbNode },

    /// 打开表数据
    OpenTableData { node: DbNode },

    /// 设计表（新建或编辑）
    DesignTable { node: DbNode },

    /// 删除表
    DeleteTable { node: DbNode },

    /// 打开视图数据
    OpenViewData { node: DbNode },

    /// 删除视图
    DeleteView { node: DbNode },

    /// 新建查询
    CreateNewQuery { node: DbNode },

    /// 打开命名查询
    OpenNamedQuery { node: DbNode },

    /// 重命名查询
    RenameQuery { node: DbNode },

    /// 删除查询
    DeleteQuery { node: DbNode },

    /// 在文件管理器中显示查询文件
    RevealQueryInFileManager { node: DbNode },

    /// 删除模式/Schema
    DeleteSchema { node: DbNode },

    /// 新建模式/Schema
    CreateSchema { node: DbNode },

    /// 批量操作
    Batch {
        action: DatabaseObjectsBatchAction,
        nodes: Vec<DbNode>,
    },
}

#[derive(Clone, Debug)]
pub enum DatabaseObjectsBatchAction {
    DeleteConnection,
    DeleteDatabase,
    DeleteSchema,
    DeleteTable,
    DeleteView,
    DeleteQuery,
}

pub struct DatabaseObjects {
    loaded_data: Entity<ObjectView>,
    // 直接管理表格数据
    columns: Vec<Column>,
    rows: Vec<Vec<String>>,
    filtered_rows: Vec<usize>,
    db_node_type: DbNodeType,
    focus_handle: FocusHandle,
    workspace: Option<Workspace>,
    search_input: Entity<InputState>,
    search_query: String,
    search_seq: u64,
    search_debouncer: Arc<Debouncer>,
    current_node: Option<DbNode>,
    selected_indices: HashSet<usize>,
    context_menu_row: Option<usize>,
    _subscriptions: Vec<Subscription>,
}

impl DatabaseObjects {
    fn on_action_focus_search(
        &mut self,
        _: &FocusSearchInput,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        focus_search_input(&self.search_input, window, cx);
    }

    fn on_action_open_selected_table_query(
        &mut self,
        _: &OpenSelectedTableQuery,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let nodes = self.build_nodes_for_selected_rows();
        if nodes.len() != 1 {
            window.push_notification(Notification::warning(t!("Common.select_row")), cx);
            return;
        }

        let node = nodes[0].clone();
        if matches!(node.node_type, DbNodeType::Table | DbNodeType::View) {
            cx.emit(DatabaseObjectsEvent::CreateNewQuery { node });
        }
    }

    pub fn new(workspace: Option<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let loaded_data = cx.new(|_| ObjectView::default());
        let focus_handle = cx.focus_handle();
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Common.search"))
                .clean_on_escape()
        });
        let search_debouncer = Arc::new(Debouncer::new(Duration::from_millis(250)));

        let search_sub = cx.subscribe_in(
            &search_input,
            window,
            |this: &mut Self,
             input: &Entity<InputState>,
             event: &InputEvent,
             _window,
             cx: &mut Context<Self>| {
                if let InputEvent::Change = event {
                    let query = input.read(cx).text().to_string();

                    this.search_seq += 1;
                    let current_seq = this.search_seq;
                    let debouncer = Arc::clone(&this.search_debouncer);
                    let query_for_task = query.clone();

                    cx.spawn(async move |view, cx| {
                        if debouncer.debounce(cx).await {
                            _ = view.update(cx, |this, cx| {
                                if this.search_seq == current_seq {
                                    this.search_query = query_for_task.to_lowercase();
                                    this.selected_indices.clear();
                                    this.apply_filter();
                                    cx.notify();
                                }
                            });
                        }
                    })
                    .detach();
                }
            },
        );

        let storage_manager = cx.global::<GlobalStorageState>().storage.clone();
        let clone_workspace = workspace.clone();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = Self::load_connection_list_view(storage_manager, clone_workspace);
            if let Some(view) = result {
                let columns = ui_columns_from_object_columns(&view.columns);
                let rows = view.rows.clone();
                let db_node_type = view.db_node_type;
                entity
                    .update(cx, move |this, cx| {
                        this.loaded_data.update(cx, |data, _cx| {
                            *data = view;
                        });
                        this.columns = columns;
                        this.rows = rows;
                        this.filtered_rows = (0..this.rows.len()).collect();
                        this.db_node_type = db_node_type;
                        this.selected_indices.clear();
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();

        Self {
            loaded_data,
            columns: vec![],
            rows: vec![],
            filtered_rows: vec![],
            db_node_type: DbNodeType::default(),
            focus_handle,
            workspace,
            search_input,
            search_query: "".to_string(),
            search_seq: 0,
            search_debouncer,
            current_node: None,
            selected_indices: HashSet::new(),
            context_menu_row: None,
            _subscriptions: vec![search_sub],
        }
    }

    fn handle_row_double_click(&self, row: usize, cx: &mut Context<Self>) {
        let Some(node) = self.build_node_for_row(row) else {
            return;
        };

        let Some(event) = Self::event_for_double_click_node(node) else {
            return;
        };

        cx.emit(event);
    }

    fn event_for_double_click_node(node: DbNode) -> Option<DatabaseObjectsEvent> {
        Some(match node.node_type {
            DbNodeType::Table => DatabaseObjectsEvent::OpenTableData { node },
            DbNodeType::View => DatabaseObjectsEvent::OpenViewData { node },
            DbNodeType::NamedQuery => DatabaseObjectsEvent::OpenNamedQuery { node },
            DbNodeType::Database | DbNodeType::Schema => {
                DatabaseObjectsEvent::AddDatabaseToTree { node }
            }
            _ => return None,
        })
    }

    pub fn handle_node_selected(
        &mut self,
        node: DbNode,
        _config: DbConnectionConfig,
        cx: &mut Context<Self>,
    ) {
        match node.node_type {
            DbNodeType::Connection
            | DbNodeType::Database
            | DbNodeType::Schema
            | DbNodeType::TablesFolder
            | DbNodeType::Table
            | DbNodeType::ViewsFolder
            | DbNodeType::View
            | DbNodeType::QueriesFolder
            | DbNodeType::NamedQuery => {}
            _ => return,
        }

        if !node.children_loaded && node.node_type != DbNodeType::Connection {
            return;
        }

        self.current_node = Some(node.clone());
        self.selected_indices.clear();
        self.context_menu_row = None;
        let node_clone = node.clone();
        let storage_manager = cx.global::<GlobalStorageState>().storage.clone();
        let global_state = cx.global::<GlobalDbState>().clone();
        let workspace = self.workspace.clone();
        let connection_id = node.connection_id.clone();
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result: Option<ObjectView> =
                if !node_clone.children_loaded && node_clone.node_type == DbNodeType::Connection {
                    Self::load_connection_list_view(storage_manager, workspace)
                } else if node_clone.node_type == DbNodeType::QueriesFolder
                    || node_clone.node_type == DbNodeType::NamedQuery
                {
                    Self::load_queries_list_view(node_clone.clone()).await
                } else {
                    global_state
                        .load_object_view(cx, connection_id, node_clone)
                        .await
                        .ok()
                        .flatten()
                };

            if let Some(view) = result {
                let columns = ui_columns_from_object_columns(&view.columns);
                let rows = view.rows.clone();
                let db_node_type = view.db_node_type;
                entity
                    .update(cx, move |this, cx| {
                        let search_query = this.search_query.clone();
                        this.loaded_data.update(cx, |data, _cx| {
                            *data = view;
                        });
                        this.columns = columns;
                        this.rows = rows;
                        this.db_node_type = db_node_type;
                        if !search_query.is_empty() {
                            this.apply_filter();
                        } else {
                            this.filtered_rows = (0..this.rows.len()).collect();
                        }
                        this.selected_indices.clear();
                        cx.notify();
                    })
                    .ok();
            }
        })
        .detach();
    }

    fn toggle_selection(&mut self, row_ix: usize, multi_select: bool) {
        self.context_menu_row = None;
        if multi_select {
            if self.selected_indices.contains(&row_ix) {
                self.selected_indices.remove(&row_ix);
            } else {
                self.selected_indices.insert(row_ix);
            }
        } else if !self.selected_indices.contains(&row_ix) {
            self.selected_indices.clear();
            self.selected_indices.insert(row_ix);
        }
    }

    fn apply_filter(&mut self) {
        if self.search_query.is_empty() {
            self.filtered_rows = (0..self.rows.len()).collect();
        } else {
            self.filtered_rows = self
                .rows
                .iter()
                .enumerate()
                .filter(|(_, row)| {
                    row.iter()
                        .any(|cell| cell.to_lowercase().contains(&self.search_query))
                })
                .map(|(idx, _)| idx)
                .collect();
        }
    }

    fn load_connection_list_view(
        storage_manager: StorageManager,
        workspace: Option<Workspace>,
    ) -> Option<ObjectView> {
        let conn_repo = storage_manager.get::<ConnectionRepository>()?;
        let w = workspace?;
        let connections = conn_repo.list_by_workspace(w.id).ok()?;

        let rows = connections
            .iter()
            .map(|stored_conn| {
                let created = stored_conn
                    .created_at
                    .map(|ts| format_timestamp(ts))
                    .unwrap_or_default();
                let updated = stored_conn
                    .updated_at
                    .map(|ts| format_timestamp(ts))
                    .unwrap_or_default();
                let remark = stored_conn.remark.clone().unwrap_or_default();
                let db_type = stored_conn
                    .to_db_connection()
                    .map(|c| c.database_type)
                    .unwrap_or(DatabaseType::MySQL);
                let connection_id = stored_conn.id.map(|id| id.to_string()).unwrap_or_default();
                vec![
                    stored_conn.name.clone(),
                    connection_id,
                    db_type.storage_key(),
                    created,
                    updated,
                    remark,
                ]
            })
            .collect();

        Some(ObjectView {
            db_node_type: DbNodeType::Connection,
            columns: vec![
                ObjectViewColumn::new("name", t!("ConnectionForm.connection_name")).width(200.0),
                ObjectViewColumn::new("id", "ID").width(80.0),
                ObjectViewColumn::new("type", t!("Common.type")),
                ObjectViewColumn::new("created_at", t!("Table.created_at")).width(200.0),
                ObjectViewColumn::new("updated_at", t!("Table.updated_at")).width(200.0),
                ObjectViewColumn::new("remark", t!("ConnectionForm.remark")).width(250.0),
            ],
            rows,
            title: t!("Connection.connection_list").to_string(),
        })
    }

    async fn load_queries_list_view(node: DbNode) -> Option<ObjectView> {
        use std::time::UNIX_EPOCH;

        let database_name = node.get_database_name().unwrap_or_default();
        let database_type = node.database_type.path_key();
        let connection_id = node.connection_id.clone();

        let queries_dir = get_queries_dir().ok()?;
        let query_path = queries_dir
            .join(database_type)
            .join(&connection_id)
            .join(&database_name);

        if !query_path.exists() {
            return Some(ObjectView {
                db_node_type: DbNodeType::NamedQuery,
                columns: vec![
                    ObjectViewColumn::new("name", t!("Query.query_name")).width(200.0),
                    ObjectViewColumn::new("created_at", t!("Table.created_at")).width(180.0),
                    ObjectViewColumn::new("updated_at", t!("Table.updated_at")).width(180.0),
                ],
                rows: vec![],
                title: t!("Query.query_list").to_string(),
            });
        }

        let entries = std::fs::read_dir(&query_path).ok()?;
        let mut rows: Vec<Vec<String>> = Vec::new();

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().map_or(false, |ext| ext == "sql") {
                let file_name = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                let (created, modified) = if let Ok(metadata) = std::fs::metadata(&path) {
                    let created_time = metadata
                        .created()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| format_timestamp(d.as_millis() as i64))
                        .unwrap_or_default();
                    let modified_time = metadata
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| format_timestamp(d.as_millis() as i64))
                        .unwrap_or_default();
                    (created_time, modified_time)
                } else {
                    (String::new(), String::new())
                };

                rows.push(vec![file_name, created, modified]);
            }
        }

        rows.sort_by(|a, b| a[0].cmp(&b[0]));

        Some(ObjectView {
            db_node_type: DbNodeType::NamedQuery,
            columns: vec![
                ObjectViewColumn::new("name", t!("Query.query_name")).width(200.0),
                ObjectViewColumn::new("created_at", t!("Table.created_at")).width(180.0),
                ObjectViewColumn::new("updated_at", t!("Table.updated_at")).width(180.0),
            ],
            rows,
            title: t!("Query.query_list").to_string(),
        })
    }

    fn build_node_for_row(&self, row_ix: usize) -> Option<DbNode> {
        let db_node_type = self.db_node_type;
        let original_row = self.filtered_rows.get(row_ix).copied()?;
        let row_data = self.rows.get(original_row)?;

        Self::build_node_from_object_row(db_node_type, self.current_node.as_ref(), row_data)
    }

    fn build_node_from_object_row(
        db_node_type: DbNodeType,
        current_node: Option<&DbNode>,
        row_data: &[String],
    ) -> Option<DbNode> {
        let name = row_data.first().cloned()?;

        // 特殊处理：当 current_node 为 None 且显示连接列表时
        if current_node.is_none() && db_node_type == DbNodeType::Connection {
            let connection_id = row_data.get(1).cloned().unwrap_or_default();
            let db_type_str = row_data.get(2).cloned().unwrap_or_default();
            let database_type =
                DatabaseType::from_storage_key(&db_type_str).unwrap_or(DatabaseType::MySQL);

            return Some(DbNode::new(
                connection_id.clone(),
                name,
                DbNodeType::Connection,
                connection_id,
                database_type,
            ));
        }

        let current_node = current_node?;
        let connection_id = current_node.connection_id.clone();
        let database_type = current_node.database_type.clone();

        let mut metadata: HashMap<String, String> = current_node.metadata.clone();
        let database = metadata.get("database").cloned().unwrap_or_default();

        let (node_id, target_node_type) = match db_node_type {
            DbNodeType::Connection => {
                if current_node.children_loaded {
                    metadata.insert("database".to_string(), name.clone());
                    (format!("{}:{}", connection_id, name), DbNodeType::Database)
                } else {
                    // 连接未展开时，从行数据获取 connection_id
                    let row_connection_id = row_data.get(1).cloned().unwrap_or_default();
                    let db_type_str = row_data.get(2).cloned().unwrap_or_default();
                    let row_database_type =
                        DatabaseType::from_storage_key(&db_type_str).unwrap_or(DatabaseType::MySQL);
                    return Some(DbNode::new(
                        row_connection_id.clone(),
                        name,
                        DbNodeType::Connection,
                        row_connection_id,
                        row_database_type,
                    ));
                }
            }
            DbNodeType::Database => {
                if current_node.node_type == DbNodeType::Connection {
                    metadata.insert("database".to_string(), name.clone());
                    (format!("{}:{}", connection_id, name), DbNodeType::Database)
                } else {
                    let db = if database.is_empty() {
                        current_node.name.clone()
                    } else {
                        database.clone()
                    };
                    metadata.insert("database".to_string(), db.clone());
                    metadata.insert("table".to_string(), name.clone());
                    (
                        format!("{}:{}:table_folder:{}", connection_id, db, name),
                        DbNodeType::Table,
                    )
                }
            }
            DbNodeType::TablesFolder | DbNodeType::Table => {
                let db = if database.is_empty() {
                    current_node.name.clone()
                } else {
                    database.clone()
                };
                let schema = current_node.get_schema_name();
                metadata.insert("database".to_string(), db.clone());
                if let Some(schema) = schema.as_ref().filter(|schema| !schema.trim().is_empty()) {
                    metadata.insert("schema".to_string(), schema.clone());
                }
                metadata.insert("table".to_string(), name.clone());
                let node_id = if let Some(schema) =
                    schema.as_ref().filter(|schema| !schema.trim().is_empty())
                {
                    format!("{}:{}:{}:table_folder:{}", connection_id, db, schema, name)
                } else {
                    format!("{}:{}:table_folder:{}", connection_id, db, name)
                };
                (node_id, DbNodeType::Table)
            }
            DbNodeType::Schema => {
                if current_node.node_type == DbNodeType::Connection {
                    metadata.insert("schema".to_string(), name.clone());
                    (format!("{}:{}", connection_id, name), DbNodeType::Schema)
                } else if current_node.node_type == DbNodeType::Database {
                    let db = if database.is_empty() {
                        current_node.name.clone()
                    } else {
                        database.clone()
                    };
                    metadata.insert("database".to_string(), db.clone());
                    metadata.insert("schema".to_string(), name.clone());
                    (
                        format!("{}:{}:{}", connection_id, db, name),
                        DbNodeType::Schema,
                    )
                } else {
                    let db = metadata
                        .get("database")
                        .cloned()
                        .unwrap_or_else(|| current_node.name.clone());
                    let schema = current_node.name.clone();
                    metadata.insert("database".to_string(), db.clone());
                    metadata.insert("schema".to_string(), schema.clone());
                    metadata.insert("table".to_string(), name.clone());
                    (
                        format!("{}:{}:{}:table_folder:{}", connection_id, db, schema, name),
                        DbNodeType::Table,
                    )
                }
            }
            DbNodeType::ViewsFolder | DbNodeType::View => {
                let db = if database.is_empty() {
                    current_node.name.clone()
                } else {
                    database.clone()
                };
                let schema = current_node.get_schema_name();
                metadata.insert("database".to_string(), db.clone());
                if let Some(schema) = schema.as_ref().filter(|schema| !schema.trim().is_empty()) {
                    metadata.insert("schema".to_string(), schema.clone());
                }
                metadata.insert("view".to_string(), name.clone());
                let node_id = if let Some(schema) =
                    schema.as_ref().filter(|schema| !schema.trim().is_empty())
                {
                    format!("{}:{}:{}:views_folder:{}", connection_id, db, schema, name)
                } else {
                    format!("{}:{}:views_folder:{}", connection_id, db, name)
                };
                (node_id, DbNodeType::View)
            }
            DbNodeType::QueriesFolder | DbNodeType::NamedQuery => {
                let query_id = row_data.get(1).cloned().unwrap_or_default();
                metadata.insert("query_name".to_string(), name.clone());
                metadata.insert("query_id".to_string(), query_id.clone());
                (
                    format!("{}:queries:{}", connection_id, query_id),
                    DbNodeType::NamedQuery,
                )
            }
            _ => return None,
        };

        Some(
            DbNode::new(
                node_id,
                name,
                target_node_type,
                connection_id,
                database_type,
            )
            .with_metadata(metadata),
        )
    }

    fn build_nodes_for_selected_rows(&self) -> Vec<DbNode> {
        let mut selected_rows: Vec<usize> = self.selected_indices.iter().copied().collect();
        selected_rows.sort_unstable();
        selected_rows
            .into_iter()
            .filter_map(|row_ix| self.build_node_for_row(row_ix))
            .collect()
    }

    fn batch_action_for_event(event: &DatabaseObjectsEvent) -> Option<DatabaseObjectsBatchAction> {
        match event {
            DatabaseObjectsEvent::DeleteConnection { .. } => {
                Some(DatabaseObjectsBatchAction::DeleteConnection)
            }
            DatabaseObjectsEvent::DeleteDatabase { .. } => {
                Some(DatabaseObjectsBatchAction::DeleteDatabase)
            }
            DatabaseObjectsEvent::DeleteSchema { .. } => {
                Some(DatabaseObjectsBatchAction::DeleteSchema)
            }
            DatabaseObjectsEvent::DeleteTable { .. } => {
                Some(DatabaseObjectsBatchAction::DeleteTable)
            }
            DatabaseObjectsEvent::DeleteView { .. } => Some(DatabaseObjectsBatchAction::DeleteView),
            DatabaseObjectsEvent::DeleteQuery { .. } => {
                Some(DatabaseObjectsBatchAction::DeleteQuery)
            }
            _ => None,
        }
    }

    fn allow_multi_event(event: &DatabaseObjectsEvent) -> bool {
        matches!(
            event,
            DatabaseObjectsEvent::OpenTableData { .. }
                | DatabaseObjectsEvent::OpenViewData { .. }
                | DatabaseObjectsEvent::OpenNamedQuery { .. }
                | DatabaseObjectsEvent::DesignTable { .. }
                | DatabaseObjectsEvent::CloseConnection { .. }
        )
    }

    fn render_context_menu_items(
        mut menu: PopupMenu,
        items: Vec<ContextMenuItem>,
        is_active: bool,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        for item in items {
            menu = match item {
                ContextMenuItem::Item {
                    label,
                    event,
                    requires_active,
                } => Self::render_context_menu_action(
                    menu,
                    label,
                    event,
                    requires_active && !is_active,
                    view,
                    window,
                ),
                ContextMenuItem::Separator => menu.separator(),
                ContextMenuItem::Submenu {
                    label,
                    items,
                    requires_active,
                } => {
                    let view = view.clone();
                    let submenu = PopupMenu::build(window, cx, move |menu, window, cx| {
                        Self::render_context_menu_items(
                            menu,
                            items.clone(),
                            is_active,
                            &view,
                            window,
                            cx,
                        )
                    });
                    menu.item(
                        PopupMenuItem::submenu(label, submenu)
                            .disabled(requires_active && !is_active),
                    )
                }
            };
        }
        menu
    }

    fn render_context_menu_action(
        menu: PopupMenu,
        label: String,
        event: ContextMenuEvent,
        disabled: bool,
        view: &Entity<Self>,
        window: &mut Window,
    ) -> PopupMenu {
        let ContextMenuEvent::TreeEvent(event) = event else {
            return menu;
        };
        let view = view.clone();
        menu.item(
            PopupMenuItem::new(label)
                .disabled(disabled)
                .on_click(window.listener_for(&view, move |_this, _, _, cx| {
                    cx.emit(DatabaseObjectsEvent::TreeEvent {
                        event: event.clone(),
                    });
                })),
        )
    }

    fn render_extension_menu_items(
        mut menu: PopupMenu,
        items: Vec<DbTreeExtensionMenuItem>,
        is_active: bool,
        node: &DbNode,
    ) -> PopupMenu {
        for item in items {
            let context = DbTreeExtensionActionContext {
                extension_id: item.extension_id.clone(),
                command_id: item.command_id.clone(),
                node_id: node.id.clone(),
                node_name: node.name.clone(),
                node_type: node.node_type,
                database_type: node.database_type.clone(),
                connection_id: node.connection_id.clone(),
            };
            let disabled = item.requires_active && !is_active;
            menu = menu.item(PopupMenuItem::new(item.label).disabled(disabled).on_click(
                move |_, window, cx| {
                    if let Some(handler) = cx
                        .try_global::<GlobalDbTreeExtensionActionHandler>()
                        .cloned()
                    {
                        handler.run(context.clone(), window, cx);
                    } else {
                        warn!(
                            "db tree extension action handler is not registered for {}",
                            context.command_id
                        );
                    }
                },
            ));
        }
        menu
    }

    fn build_context_menu_for_target(
        menu: PopupMenu,
        view: &Entity<Self>,
        window: &mut Window,
        cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let Some(row_ix) = view.read(cx).context_menu_row else {
            return menu;
        };
        let Some(node) = view.read(cx).build_node_for_row(row_ix) else {
            return menu;
        };
        let is_active = node
            .connection_id
            .parse::<i64>()
            .ok()
            .map(|id| cx.global::<ActiveConnections>().is_active(id))
            .unwrap_or(false);
        let items =
            build_context_menu_for(node.database_type.clone(), &node.id, node.node_type, cx);
        let mut menu = Self::render_context_menu_items(menu, items, is_active, view, window, cx);
        let extension_items = cx
            .try_global::<DbTreeExtensionMenuRegistry>()
            .map(|registry| {
                registry.items_for_context(&DbTreeExtensionMenuContext {
                    node_type: node.node_type,
                    node_name: node.name.clone(),
                    connection_id: node.connection_id.clone(),
                    database_type: node.database_type.clone(),
                })
            })
            .unwrap_or_default();
        if !extension_items.is_empty() {
            menu = menu.separator();
            menu = Self::render_extension_menu_items(menu, extension_items, is_active, &node);
        }
        let refresh_node = view.read(cx).current_node.clone().unwrap_or(node);
        let view = view.clone();
        menu.item(
            PopupMenuItem::new(t!("Common.refresh")).on_click(window.listener_for(
                &view,
                move |_this, _, _, cx| {
                    cx.emit(DatabaseObjectsEvent::Refresh {
                        node: refresh_node.clone(),
                    });
                },
            )),
        )
    }

    fn render_header(
        &self,
        columns: &[Column],
        show_row_number: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let mut header = h_flex()
            .w_full()
            .h(px(32.))
            .px_2()
            .items_center()
            .border_b_1()
            .border_color(cx.theme().border)
            .text_color(cx.theme().table_head_foreground)
            .bg(cx.theme().table_head);

        if show_row_number {
            header = header.child(
                div()
                    .w(px(48.))
                    .px_2()
                    .text_sm()
                    .text_color(cx.theme().table_head_foreground)
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .justify_end()
                            .child("#"),
                    ),
            );
        }

        for (col_ix, column) in columns.iter().enumerate() {
            header = header.child(
                div()
                    .relative()
                    .w(column.width)
                    .h_full()
                    .px_2()
                    .text_sm()
                    .text_color(cx.theme().table_head_foreground)
                    .child(
                        div()
                            .size_full()
                            .flex()
                            .items_center()
                            .child(column.name.clone()),
                    )
                    .child(self.render_column_resize_handle(col_ix, column, cx)),
            );
        }

        header.into_any_element()
    }

    fn render_column_resize_handle(
        &self,
        col_ix: usize,
        column: &Column,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        render_table_column_resize_handle(
            "database-object-column-resize",
            "database-object-column-resize",
            col_ix,
            column,
            cx,
            |this: &Self, col_ix| this.columns.get(col_ix).map(|column| column.width),
            |this: &mut Self, col_ix, width| {
                resize_table_column(&mut this.columns, col_ix, width);
            },
        )
    }

    fn render_row(&self, args: ObjectRowRenderArgs<'_>, cx: &App) -> impl IntoElement {
        let mut row = h_flex()
            .w_full()
            .h(one_ui::table_row_height(cx))
            .px_2()
            .items_center()
            .when(args.is_selected, |el| el.bg(cx.theme().selection));

        if args.show_row_number {
            row = row.child(
                div()
                    .w(px(48.))
                    .px_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child((args.row_ix + 1).to_string()),
            );
        }

        for (col_ix, column) in args.columns.iter().enumerate() {
            let cell_value = args.row_values.get(col_ix).cloned().unwrap_or_default();
            let tooltip_text = cell_value.clone();
            let cell = if col_ix == 0 {
                let icon = get_icon_for_node_type(&args.db_node_type, cx.theme()).color();
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(icon)
                    .child(render_object_name_text(cell_value, args.search_query, cx))
                    .into_any_element()
            } else {
                div().child(cell_value).into_any_element()
            };

            let cell_id = SharedString::from(format!("cell-{}-{}", args.row_ix, col_ix));
            row = row.child(
                div()
                    .id(cell_id)
                    .w(column.width)
                    .px_2()
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .when(!tooltip_text.is_empty(), |el| {
                        el.tooltip(move |window, cx| {
                            Tooltip::new(tooltip_text.clone()).build(window, cx)
                        })
                    })
                    .child(cell),
            );
        }

        row
    }

    fn render_toolbar_buttons(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Vec<AnyElement> {
        let mut buttons: Vec<AnyElement> = vec![];
        let current_node = self.current_node.clone();
        let data_db_node_type = self.db_node_type;
        let node_type = current_node
            .as_ref()
            .map(|n| n.node_type)
            .unwrap_or(DbNodeType::Connection);
        let database_type = current_node
            .as_ref()
            .map(|n| n.database_type.clone())
            .unwrap_or(DatabaseType::MySQL);

        buttons.push({
            let node = current_node.clone();
            Button::new("refresh-data")
                .with_size(Size::Medium)
                .icon(IconName::Refresh)
                .tooltip(t!("Common.refresh"))
                .on_click(window.listener_for(&cx.entity(), move |_this, _, _, cx| {
                    if let Some(ref node) = node {
                        cx.emit(DatabaseObjectsEvent::Refresh { node: node.clone() });
                    }
                }))
                .into_any_element()
        });

        let toolbar_buttons =
            build_toolbar_buttons_for(database_type, node_type, data_db_node_type, cx);

        for btn_config in toolbar_buttons {
            let button = match btn_config.button_type {
                ToolbarButtonType::CurrentNode => {
                    let node = current_node.clone();
                    let event_fn = btn_config.event_fn;
                    Button::new(btn_config.id)
                        .with_size(Size::Medium)
                        .icon(btn_config.icon)
                        .tooltip(btn_config.tooltip)
                        .on_click(window.listener_for(&cx.entity(), move |_this, _, _, cx| {
                            if let Some(ref node) = node {
                                let event = event_fn(node.clone());
                                cx.emit(event);
                            }
                        }))
                        .into_any_element()
                }
                ToolbarButtonType::SelectedRow => {
                    let event_fn = btn_config.event_fn;
                    Button::new(btn_config.id)
                        .with_size(Size::Medium)
                        .icon(btn_config.icon)
                        .tooltip(btn_config.tooltip)
                        .on_click(
                            window.listener_for(&cx.entity(), move |this, _, window, cx| {
                                let nodes = this.build_nodes_for_selected_rows();
                                if nodes.is_empty() {
                                    window.push_notification(
                                        Notification::warning(t!("Common.select_row")),
                                        cx,
                                    );
                                    return;
                                }
                                if nodes.len() == 1 {
                                    let event = event_fn(nodes[0].clone());
                                    cx.emit(event);
                                    return;
                                }

                                let sample_event = event_fn(nodes[0].clone());
                                if let Some(action) = Self::batch_action_for_event(&sample_event) {
                                    cx.emit(DatabaseObjectsEvent::Batch { action, nodes });
                                    return;
                                }

                                if !Self::allow_multi_event(&sample_event) {
                                    window.push_notification(
                                        Notification::warning(
                                            t!("DatabaseObjects.batch_not_supported").to_string(),
                                        ),
                                        cx,
                                    );
                                    return;
                                }

                                for node in nodes {
                                    let event = event_fn(node);
                                    cx.emit(event);
                                }
                            }),
                        )
                        .into_any_element()
                }
            };
            buttons.push(button);
        }

        buttons
    }
}

struct ObjectRowRenderArgs<'a> {
    row_ix: usize,
    row_values: &'a [String],
    columns: &'a [Column],
    show_row_number: bool,
    is_selected: bool,
    search_query: &'a str,
    db_node_type: DbNodeType,
}

impl Render for DatabaseObjects {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let loaded_data = self.loaded_data.read(cx);
        let title = loaded_data.title.clone();
        let toolbar_buttons = self.render_toolbar_buttons(window, cx);
        let columns = self.columns.clone();
        let row_count = self.filtered_rows.len();
        let show_row_number = true;
        let search_query = self.search_query.clone();
        let header = self.render_header(&columns, show_row_number, cx);
        let list_columns = columns.clone();
        let list_search_query = search_query.clone();

        v_flex()
            .size_full()
            .track_focus(&self.focus_handle)
            .key_context(DB_SEARCH_CONTEXT)
            .on_action(cx.listener(Self::on_action_focus_search))
            .on_action(cx.listener(Self::on_action_open_selected_table_query))
            .child(
                h_flex()
                    .gap_1()
                    .items_center()
                    .px_2()
                    .py_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .bg(cx.theme().background)
                    .children(toolbar_buttons)
                    .child(div().flex_1())
                    .child({
                        div().flex_1().child(
                            Input::new(&self.search_input)
                                .prefix(
                                    Icon::new(IconName::Search)
                                        .text_color(cx.theme().muted_foreground),
                                )
                                .cleanable(true)
                                .small()
                                .w_full(),
                        )
                    })
                    .into_any_element(),
            )
            .child(
                v_flex().size_full().gap_2().child(header).child(
                    div().flex_1().overflow_hidden().child(
                        uniform_list("database-objects-list", row_count, {
                            cx.processor(
                                move |state: &mut Self, range: Range<usize>, _window, cx| {
                                    let db_node_type = state.db_node_type;
                                    let show_row_number = true;
                                    range
                                        .map(|list_ix| {
                                            let Some(original_row) =
                                                state.filtered_rows.get(list_ix).copied()
                                            else {
                                                return div().id(list_ix).into_any_element();
                                            };
                                            let Some(row_values) = state.rows.get(original_row)
                                            else {
                                                return div().id(list_ix).into_any_element();
                                            };

                                            let is_selected =
                                                state.selected_indices.contains(&list_ix);
                                            let row_ix = list_ix;
                                            let view = cx.entity();
                                            div()
                                                    .id(list_ix)
                                                    .cursor_pointer()
                                                    .on_mouse_down(
                                                        MouseButton::Left,
                                                        cx.listener(
                                                            move |this,
                                                                  event: &MouseDownEvent,
                                                                  _window,
                                                                  cx| {
                                                                let multi_select =
                                                                    event.modifiers.secondary();
                                                                this.toggle_selection(
                                                                    row_ix,
                                                                    multi_select,
                                                                );
                                                                cx.notify();
                                                            },
                                                        ),
                                                    )
                                                    .on_mouse_down(
                                                        MouseButton::Right,
                                                        cx.listener(
                                                            move |this,
                                                                  _event: &MouseDownEvent,
                                                                  _window,
                                                                  cx| {
                                                                apply_object_context_menu_target(
                                                                    &mut this.selected_indices,
                                                                    &mut this.context_menu_row,
                                                                    row_ix,
                                                                );
                                                                cx.notify();
                                                            },
                                                        ),
                                                    )
                                                    .on_double_click(cx.listener(
                                                        move |this, _, _window, cx| {
                                                            this.handle_row_double_click(
                                                                row_ix, cx,
                                                            );
                                                        },
                                                    ))
                                                    .child(state.render_row(
                                                        ObjectRowRenderArgs {
                                                            row_ix,
                                                            row_values,
                                                            columns: &list_columns,
                                                            show_row_number,
                                                            is_selected,
                                                            search_query: &list_search_query,
                                                            db_node_type,
                                                        },
                                                        cx,
                                                    ))
                                                    .context_menu(move |menu, window, cx| {
                                                        Self::build_context_menu_for_target(
                                                            menu, &view, window, cx,
                                                        )
                                                    })
                                                    .into_any_element()
                                        })
                                        .collect()
                                },
                            )
                        })
                        .flex_grow(1.0)
                        .size_full()
                        .with_sizing_behavior(ListSizingBehavior::Auto),
                    ),
                ),
            )
            .child(div().p_2().text_sm().child(title))
    }
}

impl Clone for DatabaseObjects {
    fn clone(&self) -> Self {
        Self {
            loaded_data: self.loaded_data.clone(),
            columns: self.columns.clone(),
            rows: self.rows.clone(),
            filtered_rows: self.filtered_rows.clone(),
            db_node_type: self.db_node_type,
            focus_handle: self.focus_handle.clone(),
            workspace: self.workspace.clone(),
            search_input: self.search_input.clone(),
            search_seq: self.search_seq,
            search_query: self.search_query.clone(),
            search_debouncer: self.search_debouncer.clone(),
            current_node: self.current_node.clone(),
            selected_indices: self.selected_indices.clone(),
            context_menu_row: self.context_menu_row,
            _subscriptions: vec![],
        }
    }
}

impl EventEmitter<DatabaseObjectsEvent> for DatabaseObjects {}

impl Focusable for DatabaseObjects {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

pub struct DatabaseObjectsPanel {
    database_objects: Entity<DatabaseObjects>,
}

impl DatabaseObjectsPanel {
    pub fn new(workspace: Option<Workspace>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let database_objects = cx.new(|cx| DatabaseObjects::new(workspace, window, cx));

        Self { database_objects }
    }

    pub fn database_objects(&self) -> &Entity<DatabaseObjects> {
        &self.database_objects
    }

    pub fn handle_node_selected(&self, node: DbNode, config: DbConnectionConfig, cx: &mut App) {
        self.database_objects.update(cx, |database_objects, cx| {
            database_objects.handle_node_selected(node, config, cx);
        })
    }

    pub fn refresh(&self, global_state: GlobalDbState, cx: &mut App) {
        self.database_objects.update(cx, |database_objects, cx| {
            if let Some(node) = database_objects.current_node.clone() {
                let connection_id = node.connection_id.clone();
                if let Some(config) = global_state.get_config(&connection_id) {
                    database_objects.handle_node_selected(node, config, cx);
                }
            }
        });
    }
}

impl EventEmitter<TabContentEvent> for DatabaseObjectsPanel {}

impl Render for DatabaseObjectsPanel {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        self.database_objects.clone()
    }
}

impl Focusable for DatabaseObjectsPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.database_objects.focus_handle(cx)
    }
}

impl TabContent for DatabaseObjectsPanel {
    fn content_key(&self) -> &'static str {
        "DatabaseObjects"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("DatabaseObjects.title"))
    }

    fn closeable(&self, _cx: &App) -> bool {
        false
    }

    fn width_size(&self, _cx: &App) -> Option<Size> {
        Some(Size::XSmall)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn database_node() -> DbNode {
        let mut metadata = HashMap::new();
        metadata.insert("database".to_string(), "app_db".to_string());
        DbNode::new(
            "conn1:app_db",
            "app_db",
            DbNodeType::Database,
            "conn1".to_string(),
            DatabaseType::PostgreSQL,
        )
        .with_metadata(metadata)
    }

    fn schema_node() -> DbNode {
        let mut metadata = HashMap::new();
        metadata.insert("database".to_string(), "app_db".to_string());
        metadata.insert("schema".to_string(), "public".to_string());
        DbNode::new(
            "conn1:app_db:public",
            "public",
            DbNodeType::Schema,
            "conn1".to_string(),
            DatabaseType::PostgreSQL,
        )
        .with_metadata(metadata)
    }

    #[test]
    fn database_object_schema_row_builds_schema_node() {
        let row = vec![
            "public".to_string(),
            "owner".to_string(),
            "3".to_string(),
            String::new(),
        ];

        let node = DatabaseObjects::build_node_from_object_row(
            DbNodeType::Schema,
            Some(&database_node()),
            &row,
        )
        .expect("schema row should produce a node");

        assert_eq!(DbNodeType::Schema, node.node_type);
        assert_eq!("public", node.name);
        assert_eq!(
            Some("app_db"),
            node.metadata.get("database").map(String::as_str)
        );
        assert_eq!(
            Some("public"),
            node.metadata.get("schema").map(String::as_str)
        );
        assert_eq!("conn1:app_db:public", node.id);
    }

    #[test]
    fn schema_object_table_row_builds_table_node() {
        let row = vec!["users".to_string()];

        let node = DatabaseObjects::build_node_from_object_row(
            DbNodeType::Table,
            Some(&schema_node()),
            &row,
        )
        .expect("table row under schema should produce a node");

        assert_eq!(DbNodeType::Table, node.node_type);
        assert_eq!("users", node.name);
        assert_eq!(
            Some("app_db"),
            node.metadata.get("database").map(String::as_str)
        );
        assert_eq!(
            Some("public"),
            node.metadata.get("schema").map(String::as_str)
        );
        assert_eq!(
            Some("users"),
            node.metadata.get("table").map(String::as_str)
        );
        assert_eq!("conn1:app_db:public:table_folder:users", node.id);
    }

    #[test]
    fn schema_object_view_row_builds_view_node() {
        let row = vec!["active_users".to_string()];

        let node = DatabaseObjects::build_node_from_object_row(
            DbNodeType::View,
            Some(&schema_node()),
            &row,
        )
        .expect("view row under schema should produce a node");

        assert_eq!(DbNodeType::View, node.node_type);
        assert_eq!("active_users", node.name);
        assert_eq!(
            Some("app_db"),
            node.metadata.get("database").map(String::as_str)
        );
        assert_eq!(
            Some("public"),
            node.metadata.get("schema").map(String::as_str)
        );
        assert_eq!(
            Some("active_users"),
            node.metadata.get("view").map(String::as_str)
        );
        assert_eq!("conn1:app_db:public:views_folder:active_users", node.id);
    }

    #[test]
    fn schema_double_click_adds_node_to_tree() {
        let event = DatabaseObjects::event_for_double_click_node(schema_node())
            .expect("schema double click should emit an event");

        assert!(matches!(
            event,
            DatabaseObjectsEvent::AddDatabaseToTree { node }
                if node.node_type == DbNodeType::Schema && node.name == "public"
        ));
    }
}

impl Clone for DatabaseObjectsPanel {
    fn clone(&self) -> Self {
        Self {
            database_objects: self.database_objects.clone(),
        }
    }
}
