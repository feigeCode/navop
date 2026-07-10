use std::ops::Deref;
use std::path::PathBuf;

use crate::database_objects_tab::DatabaseObjectsPanel;
use crate::database_toolbar::{
    DatabaseToolbarAction, DatabaseToolbarItem, WORKSPACE_TOOLBAR_HEIGHT,
    WORKSPACE_TOOLBAR_HOVER_ALPHA, WORKSPACE_TOOLBAR_ITEM_HEIGHT, WORKSPACE_TOOLBAR_ITEM_RADIUS,
    WORKSPACE_TOOLBAR_ITEM_WIDTH, database_toolbar_items, toolbar_item_icon, toolbar_item_label,
    toolbar_tone_color,
};
use crate::database_users_tab::DatabaseUsersTab;
use crate::db_tree_event::DatabaseEventHandler;
use crate::db_tree_view::{DbTreeView, DbTreeViewEvent, SqlDumpMode};
use crate::sidebar::{DatabaseSidebar, DatabaseSidebarEvent};
use crate::sql_editor_view::SqlEditorTab;
use ai_chat_view::{CodeBlockAction, LanguageMatcher};
use db::{
    DbNodeType, GlobalDbState,
    ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path},
};
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Axis, Bounds, Context, Element, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, Hsla, InteractiveElement, IntoElement, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString, StatefulInteractiveElement,
    Style, Styled, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::WindowExt;
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, h_flex, notification::Notification, v_flex,
};
use one_core::layout::{
    SIDEBAR_DEFAULT_WIDTH, SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH,
};
use one_core::sidebar_contribution::{
    SidebarContribution, SidebarPanelChrome, SidebarPanelId, SidebarPanelPolicy, SidebarPanelSize,
    SidebarPanelStyle, SidebarPlacement, SidebarPlacementSet,
};
use one_core::storage::{ActiveConnections, DbConnectionConfig, Workspace};
use one_core::{
    storage::StoredConnection,
    tab_container::{TabContainer, TabContent, TabContentEvent, TabItem},
};
use one_ui::resize_handle::{HandlePlacement, ResizePanel, resize_handle};
use rust_i18n::t;
use uuid::Uuid;

const PANEL_MIN_SIZE: Pixels = px(100.0);
const TREE_PANEL_DEFAULT_SIZE: Pixels = px(250.0);
const CHAT_SIDEBAR_MIN_WIDTH: Pixels = px(360.0);

fn database_tools_sidebar_policy() -> SidebarPanelPolicy {
    SidebarPanelPolicy {
        hideable: true,
        movable: true,
        allowed_placements: SidebarPlacementSet::all(),
        initially_visible: true,
    }
}

fn database_tools_sidebar_size(panel_visible: bool, panel_size: Pixels) -> SidebarPanelSize {
    if panel_visible {
        SidebarPanelSize {
            side_width: Some(panel_size + TOOLBAR_WIDTH),
            bottom_height: Some(panel_size),
        }
    } else {
        SidebarPanelSize {
            side_width: Some(TOOLBAR_WIDTH),
            bottom_height: Some(TOOLBAR_WIDTH),
        }
    }
}

fn database_tools_sidebar_chrome() -> SidebarPanelChrome {
    SidebarPanelChrome::HostNoHeader
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum DatabaseTabIconSource {
    Asset(String),
    File(PathBuf),
}

impl DatabaseTabIconSource {
    fn into_icon(self, size: Size) -> Icon {
        match self {
            DatabaseTabIconSource::Asset(path) => driver_icon_from_asset_path(path, size),
            DatabaseTabIconSource::File(path) => driver_icon_from_file_path(path, size),
        }
    }
}

fn external_driver_tab_icon_source_from_registry(
    config: &DbConnectionConfig,
    registry: &IpcDriverRegistry,
) -> Option<DatabaseTabIconSource> {
    let display = registry.display_for_config(config)?;
    if let Some(path) = display.icon_file_path {
        return Some(DatabaseTabIconSource::File(path));
    }
    display.icon_asset_path.map(DatabaseTabIconSource::Asset)
}

fn database_tab_icon_from_registry(
    config: &DbConnectionConfig,
    registry: &IpcDriverRegistry,
) -> Icon {
    external_driver_tab_icon_source_from_registry(config, registry)
        .map(|source| source.into_icon(Size::Medium))
        .unwrap_or_else(|| config.database_type.as_node_icon().with_size(Size::Medium))
}

fn database_tab_icon(config: &DbConnectionConfig) -> Icon {
    database_tab_icon_with_registry_loader(config, IpcDriverRegistry::load_default)
}

fn database_tab_icon_with_registry_loader(
    config: &DbConnectionConfig,
    load_registry: impl FnOnce() -> IpcDriverRegistry,
) -> Icon {
    if !config.database_type.is_external() {
        return config.database_type.as_node_icon().with_size(Size::Medium);
    }

    let registry = load_registry();
    database_tab_icon_from_registry(config, &registry)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResizingPanel {
    TreePanel,
    Sidebar,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DatabaseSidebarRenderMode {
    Embedded,
    External,
}

pub struct DatabaseTabView {
    connections: Vec<StoredConnection>,
    tab_container: Entity<TabContainer>,
    db_tree_view: Entity<DbTreeView>,
    status_msg: Entity<String>,
    is_connected: Entity<bool>,
    _event_handler: Entity<DatabaseEventHandler>,
    workspace: Option<Workspace>,
    focus_handle: FocusHandle,
    sidebar: Entity<DatabaseSidebar>,
    _subscriptions: Vec<gpui::Subscription>,
    tree_panel_size: Pixels,
    sidebar_panel_size: Pixels,
    sidebar_render_mode: DatabaseSidebarRenderMode,
    resizing: Option<ResizingPanel>,
    bounds: Bounds<Pixels>,
}

impl DatabaseTabView {
    pub fn new_with_active_conn(
        workspace: Option<Workspace>,
        connections: Vec<StoredConnection>,
        active_conn_id: Option<i64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let db_tree_view = cx.new(|cx| DbTreeView::new(&connections, window, cx));

        let tab_container = cx.new(|cx| TabContainer::new(window, cx));

        let objects_panel = cx.new(|cx| DatabaseObjectsPanel::new(workspace.clone(), window, cx));

        tab_container.update(cx, |container, cx| {
            let panel = objects_panel.clone();
            let tab = TabItem::new("objects-panel", "database", panel);
            container.set_pinned_tab(tab, cx);
        });

        let status_msg = cx.new(|_| "Ready".to_string());
        let is_connected = cx.new(|_| true);

        let event_handler = cx.new(|cx| {
            DatabaseEventHandler::new(
                &db_tree_view,
                tab_container.clone(),
                objects_panel.clone(),
                window,
                cx,
            )
        });

        let sidebar =
            cx.new(|cx| DatabaseSidebar::new(connections.clone(), active_conn_id, window, cx));

        // 注册 SQL 代码块操作
        Self::register_sql_code_block_actions(&sidebar, tab_container.clone(), &connections, cx);

        let mut subscriptions = Vec::new();
        subscriptions.push(
            cx.subscribe(
                &sidebar,
                |_this, _, event: &DatabaseSidebarEvent, cx| match event {
                    DatabaseSidebarEvent::PanelChanged => {
                        cx.notify();
                    }
                    DatabaseSidebarEvent::AskAi => {
                        cx.notify();
                    }
                },
            ),
        );
        subscriptions.push(cx.subscribe(&db_tree_view, {
            let sidebar = sidebar.clone();
            move |this, tree, event: &DbTreeViewEvent, cx| {
                if let DbTreeViewEvent::NodeSelected { node_id } = event
                    && let Some((connection_id, database, schema)) =
                        tree.read(cx).ai_scope_for_node(node_id)
                {
                    sidebar.update(cx, |sidebar, cx| {
                        sidebar.set_database_scope(&connection_id, database, schema, cx);
                    });
                    this.sidebar_panel_size = this.sidebar_panel_size.max(SIDEBAR_DEFAULT_WIDTH);
                }
            }
        }));

        let mut global_state = cx.global::<GlobalDbState>().clone();

        let connections_clone = connections.clone();
        let clone_db_tree_view = db_tree_view.clone();
        cx.spawn(async move |_handle, cx: &mut AsyncApp| {
            for conn in &connections_clone {
                if let Ok(db_config) = conn.to_db_connection() {
                    let _ = global_state.register_connection(db_config);
                }
            }
            if let Some(id) = active_conn_id {
                _ = clone_db_tree_view.update(cx, |tree_view, cx| {
                    tree_view.active_connection(id.to_string(), cx);
                });
            }
        })
        .detach();

        Self {
            connections: connections.clone(),
            tab_container,
            db_tree_view,
            status_msg,
            is_connected,
            _event_handler: event_handler,
            workspace,
            focus_handle: cx.focus_handle(),
            sidebar,
            _subscriptions: subscriptions,
            tree_panel_size: TREE_PANEL_DEFAULT_SIZE,
            sidebar_panel_size: SIDEBAR_DEFAULT_WIDTH,
            sidebar_render_mode: DatabaseSidebarRenderMode::Embedded,
            resizing: None,
            bounds: Bounds::default(),
        }
    }

    pub fn with_external_sidebar(mut self) -> Self {
        self.sidebar_render_mode = DatabaseSidebarRenderMode::External;
        self
    }

    pub fn new(
        workspace: Option<Workspace>,
        connection: StoredConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let active_conn_id = connection.id;
        Self::new_with_active_conn(workspace, vec![connection], active_conn_id, window, cx)
    }

    pub fn ask_ai(&mut self, message: String, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.ask_ai(message, cx);
        });
        cx.notify();
    }

    /// 注册 SQL 代码块操作
    fn register_sql_code_block_actions(
        sidebar: &Entity<DatabaseSidebar>,
        tab_container: Entity<TabContainer>,
        connections: &[StoredConnection],
        cx: &mut App,
    ) {
        // 获取第一个连接的信息用于创建新编辑器
        let first_conn = connections.first().cloned();

        // 操作1：插入到当前编辑器
        let tab_container_for_insert = tab_container.clone();
        if let Some(insert_action) = CodeBlockAction::new("sql-insert-to-editor")
            .icon(IconName::Edit)
            .label(t!("DatabaseTab.insert_editor").to_string())
            .matcher(LanguageMatcher::sql())
            .on_click(move |code, _lang, window, cx| {
                // 获取当前激活的 tab
                if let Some(active_tab) = tab_container_for_insert.read(cx).active_tab() {
                    // 检查是否是 SQL 编辑器
                    if active_tab.content().content_key(cx) == "SqlEditor" {
                        if let Ok(sql_editor) =
                            active_tab.content().view().downcast::<SqlEditorTab>()
                        {
                            sql_editor.update(cx, |editor, cx| {
                                editor.set_sql(code, window, cx);
                            });
                        }
                    }
                }
            })
            .build()
        {
            sidebar.update(cx, |s, cx| {
                s.register_code_block_action(insert_action, cx);
            });
        }

        // 操作2：打开新编辑器
        let tab_container_for_new = tab_container.clone();
        if let Some(new_editor_action) = CodeBlockAction::new("sql-open-new-editor")
            .icon(IconName::Query)
            .label(t!("DatabaseTab.open_new_editor").to_string())
            .matcher(LanguageMatcher::sql())
            .on_click(move |code, _lang, window, cx| {
                let Some(conn) = first_conn.as_ref() else {
                    return;
                };
                let Ok(db_config) = conn.to_db_connection() else {
                    return;
                };

                let connection_id = conn.id.map(|id| id.to_string()).unwrap_or_default();
                let database_type = db_config.database_type;
                let tab_id = format!("query-ai-{}", Uuid::new_v4());
                let tab_id_clone = tab_id.clone();
                let conn_id_clone = connection_id.clone();
                let code_clone = code.clone();

                tab_container_for_new.update(cx, |container, cx| {
                    container.activate_or_add_tab_lazy(
                        tab_id.clone(),
                        move |window, cx| {
                            let sql_editor = cx.new(|cx| {
                                let editor = SqlEditorTab::new_with_config(
                                    crate::sql_editor_view::SqlEditorTabConfig {
                                        title: "AI Query".into(),
                                        connection_id: connection_id.clone(),
                                        database_type,
                                        file_path: None,
                                        initial_database: None,
                                        initial_schema: None,
                                    },
                                    window,
                                    cx,
                                );
                                editor.set_sql(code_clone.clone(), window, cx);
                                editor
                            });
                            TabItem::new(tab_id_clone.clone(), conn_id_clone.clone(), sql_editor)
                        },
                        window,
                        cx,
                    );
                });
            })
            .build()
        {
            sidebar.update(cx, |s, cx| {
                s.register_code_block_action(new_editor_action, cx);
            });
        }
    }

    fn render_tree_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("tree-resize-handle", Axis::Horizontal)
            .placement(HandlePlacement::Left)
            .on_drag(ResizePanel, move |info, _, _, cx| {
                cx.stop_propagation();
                view.update(cx, |view, cx| {
                    view.resizing = Some(ResizingPanel::TreePanel);
                    cx.notify();
                });
                cx.new(|_| info.deref().clone())
            })
    }

    fn render_sidebar_resize_handle(
        &mut self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>("sidebar-resize-handle", Axis::Horizontal)
            .placement(HandlePlacement::Right)
            .on_drag(ResizePanel, move |info, _, _, cx| {
                cx.stop_propagation();
                view.update(cx, |view, cx| {
                    view.resizing = Some(ResizingPanel::Sidebar);
                    cx.notify();
                });
                cx.new(|_| info.deref().clone())
            })
    }

    fn resize(
        &mut self,
        mouse_position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resizing) = self.resizing else {
            return;
        };

        let available_width = self.bounds.size.width;

        match resizing {
            ResizingPanel::TreePanel => {
                let new_size = mouse_position.x - self.bounds.left();
                let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
                let sidebar_width = if sidebar_visible {
                    self.sidebar_panel_size
                } else {
                    TOOLBAR_WIDTH
                };
                let max_size =
                    (available_width - PANEL_MIN_SIZE - sidebar_width).max(PANEL_MIN_SIZE);
                self.tree_panel_size = new_size.clamp(PANEL_MIN_SIZE, max_size);
            }
            ResizingPanel::Sidebar => {
                let new_size = self.bounds.right() - mouse_position.x;
                let max_size = (available_width - self.tree_panel_size - PANEL_MIN_SIZE)
                    .max(SIDEBAR_MIN_WIDTH);
                let upper = max_size.min(SIDEBAR_MAX_WIDTH);
                let lower = SIDEBAR_MIN_WIDTH.max(CHAT_SIDEBAR_MIN_WIDTH).min(upper);
                self.sidebar_panel_size = new_size.clamp(lower, upper);
            }
        }

        cx.notify();
    }

    fn done_resizing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.resizing = None;
        cx.notify();
    }

    fn render_workspace_content(&mut self, cx: &mut Context<Self>) -> AnyElement {
        v_flex()
            .size_full()
            .min_h_0()
            .child(self.render_workspace_toolbar(cx))
            .child(div().flex_1().min_h_0().child(self.tab_container.clone()))
            .into_any_element()
    }

    fn render_workspace_toolbar(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let items = database_toolbar_items()
            .into_iter()
            .filter(|item| self.should_render_toolbar_item(item, cx))
            .collect::<Vec<_>>();

        h_flex()
            .h(WORKSPACE_TOOLBAR_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .overflow_x_hidden()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .children(
                items
                    .into_iter()
                    .map(|item| self.render_toolbar_item(item, cx)),
            )
            .into_any_element()
    }

    fn should_render_toolbar_item(&self, item: &DatabaseToolbarItem, cx: &App) -> bool {
        match item.action {
            DatabaseToolbarAction::Users => self.toolbar_connection_supports_users(cx),
            _ => true,
        }
    }

    fn render_toolbar_item(&self, item: DatabaseToolbarItem, cx: &mut Context<Self>) -> AnyElement {
        let color = toolbar_tone_color(item.tone, cx);
        let hover_bg = cx.theme().muted.opacity(WORKSPACE_TOOLBAR_HOVER_ALPHA);
        let border = cx.theme().border.opacity(0.0);
        let hover_border = cx.theme().border;
        let action = item.action;

        div()
            .id(item.id)
            .w(WORKSPACE_TOOLBAR_ITEM_WIDTH)
            .h(WORKSPACE_TOOLBAR_ITEM_HEIGHT)
            .flex_shrink_0()
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap_1()
            .rounded(WORKSPACE_TOOLBAR_ITEM_RADIUS)
            .border_1()
            .border_color(border)
            .cursor_pointer()
            .hover(move |this| this.bg(hover_bg).border_color(hover_border))
            .on_click(cx.listener(move |this, _, window, cx| {
                this.handle_toolbar_action(action, window, cx);
            }))
            .child(toolbar_item_icon(item.icon, color))
            .child(toolbar_item_label(t!(item.label_i18n_key).to_string(), cx))
            .into_any_element()
    }

    fn handle_toolbar_action(
        &mut self,
        action: DatabaseToolbarAction,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match action {
            DatabaseToolbarAction::ShowObjects => self.activate_objects_panel(window, cx),
            DatabaseToolbarAction::CreateQuery => self.emit_tree_event_or_notify(
                |node_id| DbTreeViewEvent::CreateNewQuery { node_id },
                "没有可用的数据库连接，无法新建查询。",
                window,
                cx,
            ),
            DatabaseToolbarAction::CompareSchema => self.emit_tree_event_or_notify(
                |node_id| DbTreeViewEvent::CompareSchema { node_id },
                "没有可用的数据库连接，无法比较结构。",
                window,
                cx,
            ),
            DatabaseToolbarAction::CompareData => self.emit_tree_event_or_notify(
                |node_id| DbTreeViewEvent::CompareData { node_id },
                "没有可用的数据库连接，无法比较数据。",
                window,
                cx,
            ),
            DatabaseToolbarAction::Backup => self.emit_backup_event(window, cx),
            DatabaseToolbarAction::Users => self.open_users_tab(window, cx),
            DatabaseToolbarAction::DataGenerator => {
                self.notify_unimplemented_action("数据生成", window, cx)
            }
            DatabaseToolbarAction::Automation => {
                self.notify_unimplemented_action("自动运行", window, cx)
            }
            DatabaseToolbarAction::Model => self.notify_unimplemented_action("模型", window, cx),
            DatabaseToolbarAction::Bi => self.notify_unimplemented_action("BI", window, cx),
        }
    }

    fn activate_objects_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.tab_container.update(cx, |container, cx| {
            container.activate_pinned_tab(window, cx);
        });
    }

    fn open_users_tab(&self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(config) = self.toolbar_connection_config(cx) else {
            self.notify_info(
                &t!("DatabaseUsers.no_available_connection").to_string(),
                window,
                cx,
            );
            return;
        };

        if !self.connection_supports_users(&config, cx) {
            self.notify_info(
                &t!("DatabaseUsers.unsupported_database").to_string(),
                window,
                cx,
            );
            return;
        }

        let tab_id = format!("db-users-{}", config.id);
        let tab_id_clone = tab_id.clone();
        let connection_id = config.id.clone();

        self.tab_container.update(cx, |container, cx| {
            container.activate_or_add_tab_lazy(
                tab_id.clone(),
                move |_window, cx| {
                    let users_tab = cx.new(|cx| DatabaseUsersTab::new(config.clone(), cx));
                    TabItem::new(tab_id_clone.clone(), connection_id.clone(), users_tab)
                },
                window,
                cx,
            );
        });
    }

    fn toolbar_connection_supports_users(&self, cx: &App) -> bool {
        self.toolbar_connection_config(cx)
            .as_ref()
            .is_some_and(|config| self.connection_supports_users(config, cx))
    }

    fn connection_supports_users(&self, config: &DbConnectionConfig, cx: &App) -> bool {
        cx.global::<GlobalDbState>()
            .get_plugin(&config.database_type)
            .map(|plugin| plugin.ui_manifest().capabilities.supports_users)
            .unwrap_or(false)
    }

    fn toolbar_connection_config(&self, cx: &App) -> Option<DbConnectionConfig> {
        let selected_connection_id = self.db_tree_view.read(cx).selected_or_first_connection_id();
        let connection = selected_connection_id
            .as_deref()
            .and_then(|id| {
                self.connections.iter().find(|connection| {
                    connection
                        .id
                        .is_some_and(|conn_id| conn_id.to_string() == id)
                })
            })
            .or_else(|| self.connections.first())?;

        connection.to_db_connection().ok()
    }

    fn emit_tree_event_or_notify(
        &self,
        build_event: impl FnOnce(String) -> DbTreeViewEvent,
        message: &'static str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(node_id) = self.db_tree_view.read(cx).selected_or_first_node_id() else {
            self.notify_info(message, window, cx);
            return;
        };
        self.emit_tree_event(build_event(node_id), cx);
    }

    fn emit_backup_event(&self, window: &mut Window, cx: &mut Context<Self>) {
        let node_types = [DbNodeType::Database, DbNodeType::Schema, DbNodeType::Table];
        let node_id = self
            .db_tree_view
            .read(cx)
            .selected_or_first_node_id_for_types(&node_types);
        let Some(node_id) = node_id else {
            self.notify_info("请选择数据库、Schema 或表后再备份。", window, cx);
            return;
        };
        self.emit_tree_event(
            DbTreeViewEvent::DumpSqlFile {
                node_id,
                mode: SqlDumpMode::StructureAndData,
            },
            cx,
        );
    }

    fn emit_tree_event(&self, event: DbTreeViewEvent, cx: &mut Context<Self>) {
        self.db_tree_view.update(cx, |_tree, cx| cx.emit(event));
    }

    fn notify_unimplemented_action(
        &self,
        label: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.notify_info(&format!("{label} 功能暂未实现。"), window, cx);
    }

    fn notify_info(&self, message: &str, window: &mut Window, cx: &mut Context<Self>) {
        window.push_notification(Notification::info(message.to_string()).autohide(true), cx);
    }

    fn render_connection_status(&self, cx: &App) -> AnyElement {
        let status_text = self.status_msg.read(cx).clone();
        let is_error = status_text.contains("Failed") || status_text.contains("failed");

        let first_conn = self.connections.first();
        let conn_name = first_conn
            .map(|c| c.name.clone())
            .unwrap_or_else(|| "Unknown".to_string());
        let (conn_host, conn_port, conn_username, conn_database) = first_conn
            .and_then(|c| c.to_db_connection().ok())
            .map(|p| (p.host, p.port, p.username, p.database))
            .unwrap_or_default();

        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_6()
            .child(
                div()
                    .w(px(64.0))
                    .h(px(64.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .w(px(48.0))
                            .h(px(48.0))
                            .rounded(px(24.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .when(!is_error, |this| {
                                this.border_4()
                                    .border_color(cx.theme().accent)
                                    .text_2xl()
                                    .text_color(cx.theme().accent)
                                    .child("⟳")
                            })
                            .when(is_error, |this| {
                                this.bg(Hsla::red())
                                    .text_color(gpui::white())
                                    .text_2xl()
                                    .child("✕")
                            }),
                    ),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::BOLD)
                    .child(format!("Database Connection: {}", conn_name)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .p_4()
                    .bg(cx.theme().muted)
                    .rounded(px(8.0))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Host:"))
                            .child(conn_host),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Port:"))
                            .child(format!("{}", conn_port)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(div().font_weight(FontWeight::SEMIBOLD).child("Username:"))
                            .child(conn_username),
                    )
                    .when_some(conn_database, |this, db| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .child(div().font_weight(FontWeight::SEMIBOLD).child("Database:"))
                                .child(db),
                        )
                    }),
            )
            .child(
                div()
                    .text_lg()
                    .when(!is_error, |this| this.text_color(cx.theme().accent))
                    .when(is_error, |this| this.text_color(Hsla::red()))
                    .child(status_text),
            )
            .into_any_element()
    }
}

impl Focusable for DatabaseTabView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.tab_container.focus_handle(cx)
    }
}

impl EventEmitter<TabContentEvent> for DatabaseTabView {}

impl TabContent for DatabaseTabView {
    fn content_key(&self) -> &'static str {
        "Database"
    }

    fn title(&self, _cx: &App) -> SharedString {
        if let Some(workspace) = &self.workspace {
            workspace.name.clone().into()
        } else {
            self.connections
                .first()
                .map(|c| c.name.clone())
                .unwrap_or_else(|| "Database".to_string())
                .into()
        }
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        if self.workspace.is_some() {
            Some(IconName::AppsColor.color().with_size(Size::Medium))
        } else {
            let db_connection = self.connections.first().map(|c| c.to_db_connection());
            match db_connection {
                None => Some(IconName::Database.color()),
                Some(result) => match result {
                    Ok(conn) => Some(database_tab_icon(&conn)),
                    Err(_) => Some(IconName::Database.color().with_size(Size::Medium)),
                },
            }
        }
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn can_split(&self, _cx: &App) -> bool {
        true
    }

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        if self.sidebar_render_mode != DatabaseSidebarRenderMode::External {
            return Vec::new();
        }

        vec![
            SidebarContribution {
                id: SidebarPanelId::new(self.db_tree_view.entity_id(), "database.tree"),
                title: SharedString::from("Database"),
                icon: IconName::Database,
                view: self.db_tree_view.clone().into(),
                default_placement: SidebarPlacement::Left,
                policy: SidebarPanelPolicy {
                    hideable: false,
                    movable: true,
                    allowed_placements: SidebarPlacementSet::left_right(),
                    initially_visible: true,
                },
                style: SidebarPanelStyle::default(),
                size: SidebarPanelSize {
                    side_width: Some(self.tree_panel_size),
                    bottom_height: None,
                },
                chrome: SidebarPanelChrome::HostNoHeader,
                actions: Default::default(),
            },
            SidebarContribution {
                id: SidebarPanelId::new(self.sidebar.entity_id(), "database.sidebar"),
                title: SharedString::from("Database Tools"),
                icon: IconName::Bot,
                view: self.sidebar.clone().into(),
                default_placement: SidebarPlacement::Right,
                policy: database_tools_sidebar_policy(),
                style: SidebarPanelStyle::default(),
                size: database_tools_sidebar_size(
                    self.sidebar.read(_cx).is_panel_visible(),
                    self.sidebar_panel_size,
                ),
                chrome: database_tools_sidebar_chrome(),
                actions: Default::default(),
            },
        ]
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(true, cx);
        });
    }

    fn on_deactivate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_active(false, cx);
        });
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<bool> {
        let tab_container = self.tab_container.clone();
        let connections = self.connections.clone();

        let tabs_info: Vec<_> = tab_container
            .read(cx)
            .tabs()
            .iter()
            .map(|t| (t.id().to_string(), t.content().clone()))
            .collect();

        let tasks: Vec<_> = tabs_info
            .iter()
            .map(|(id, content)| content.try_close(id, window, cx))
            .collect();

        cx.spawn(async move |_handle, cx: &mut AsyncApp| {
            for task in tasks {
                if !task.await {
                    return false;
                }
            }

            let _ = cx.update(|cx| {
                let global_state = cx.global_mut::<ActiveConnections>();
                for conn in &connections {
                    if let Some(id) = conn.id {
                        global_state.remove(id);
                    }
                }
            });

            let global_state = cx.update(|cx| cx.global::<GlobalDbState>().clone());
            let connection_ids: Vec<String> = connections
                .iter()
                .filter_map(|conn| conn.id.map(|id| id.to_string()))
                .collect();

            for connection_id in connection_ids {
                if let Err(e) = global_state.disconnect_all(cx, connection_id.clone()).await {
                    tracing::warn!("Failed to disconnect connection {}: {}", connection_id, e);
                }
            }

            true
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use db::ipc::{IpcDriverManifest, IpcDriverRegistry};
    use one_core::storage::DatabaseType;
    use std::collections::HashMap;

    fn external_config(driver_id: &str) -> DbConnectionConfig {
        DbConnectionConfig {
            id: "1".to_string(),
            database_type: DatabaseType::external(driver_id),
            name: "saved".to_string(),
            host: "localhost".to_string(),
            port: 0,
            username: String::new(),
            password: String::new(),
            database: None,
            service_name: None,
            sid: None,
            workspace_id: None,
            proxy: None,
            extra_params: HashMap::new(),
        }
    }

    fn driver_manifest(icon: &str) -> IpcDriverManifest {
        let mut manifest: IpcDriverManifest = serde_json::from_str(&format!(
            r#"{{
                "id": "demo",
                "name": "DemoDB",
                "entry": {{ "command": "driver" }},
                "transport": {{ "name": "demo.sock" }},
                "ui": {{ "icon": "{icon}" }}
            }}"#,
        ))
        .unwrap();
        manifest.manifest_dir = PathBuf::from("/drivers/demo");
        manifest
    }

    #[test]
    fn external_database_tab_icon_uses_builtin_driver_asset_icon() {
        let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest("DuckDB")]);
        let source =
            external_driver_tab_icon_source_from_registry(&external_config("demo"), &registry);

        assert_eq!(
            Some(DatabaseTabIconSource::Asset("icons/duckdb.svg".to_string())),
            source
        );
    }

    #[test]
    fn external_database_tab_icon_prefers_driver_file_icon() {
        let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest("icons/demo.svg")]);
        let source =
            external_driver_tab_icon_source_from_registry(&external_config("demo"), &registry);

        assert_eq!(
            Some(DatabaseTabIconSource::File(PathBuf::from(
                "/drivers/demo/icons/demo.svg"
            ))),
            source
        );
    }

    #[test]
    fn builtin_database_tab_icon_does_not_use_external_driver_source() {
        let registry = IpcDriverRegistry::from_drivers(vec![driver_manifest("DuckDB")]);
        let mut config = external_config("demo");
        config.database_type = DatabaseType::MySQL;

        assert_eq!(
            None,
            external_driver_tab_icon_source_from_registry(&config, &registry)
        );
    }

    #[test]
    fn builtin_database_tab_icon_bypasses_external_driver_registry() {
        let mut config = external_config("demo");
        config.database_type = DatabaseType::MySQL;

        let _icon = database_tab_icon_with_registry_loader(&config, || {
            panic!("builtin database tab icon should not load external driver registry")
        });
    }

    #[test]
    fn database_tools_sidebar_keeps_toolbar_visible_by_default() {
        let policy = database_tools_sidebar_policy();

        assert!(policy.hideable);
        assert!(policy.initially_visible);
    }

    #[test]
    fn database_tools_sidebar_uses_toolbar_width_until_panel_opens() {
        let collapsed = database_tools_sidebar_size(false, SIDEBAR_DEFAULT_WIDTH);
        let expanded = database_tools_sidebar_size(true, SIDEBAR_DEFAULT_WIDTH);

        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.side_width);
        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.bottom_height);
        assert_eq!(
            Some(SIDEBAR_DEFAULT_WIDTH + TOOLBAR_WIDTH),
            expanded.side_width
        );
        assert_eq!(Some(SIDEBAR_DEFAULT_WIDTH), expanded.bottom_height);
    }

    #[test]
    fn database_tools_sidebar_does_not_render_host_header() {
        assert_eq!(
            SidebarPanelChrome::HostNoHeader,
            database_tools_sidebar_chrome()
        );
    }
}

impl Render for DatabaseTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_connected_flag = *self.is_connected.read(cx);
        let view = cx.entity().clone();
        let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
        let sidebar_panel_size = self.sidebar_panel_size;

        if is_connected_flag && self.sidebar_render_mode == DatabaseSidebarRenderMode::External {
            return div()
                .track_focus(&self.focus_handle)
                .size_full()
                .child(self.render_workspace_content(cx));
        }

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .when(!is_connected_flag, |el: gpui::Div| {
                el.child(self.render_connection_status(cx))
            })
            .when(is_connected_flag, |el: gpui::Div| {
                let border_color = cx.theme().border;
                let tree_panel_size = self.tree_panel_size;

                el.child(
                    h_flex()
                        .size_full()
                        .child(
                            div()
                                .relative()
                                .h_full()
                                .w(tree_panel_size)
                                .flex_shrink_0()
                                .border_r_1()
                                .border_color(border_color)
                                .child(self.db_tree_view.clone())
                                .child(self.render_tree_resize_handle(window, cx)),
                        )
                        .child(
                            div()
                                .flex_1()
                                .h_full()
                                .min_w_0()
                                .child(self.render_workspace_content(cx)),
                        )
                        .when(sidebar_visible, |this| {
                            this.child(
                                div()
                                    .relative()
                                    .h_full()
                                    .w(sidebar_panel_size)
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(self.render_sidebar_resize_handle(window, cx))
                                    .child(self.sidebar.clone()),
                            )
                        })
                        .when(!sidebar_visible, |this| {
                            this.child(
                                div()
                                    .h_full()
                                    .w(TOOLBAR_WIDTH)
                                    .flex_shrink_0()
                                    .overflow_hidden()
                                    .child(self.sidebar.clone()),
                            )
                        })
                        .child(ResizeEventHandler { view }),
                )
            })
    }
}

struct ResizeEventHandler {
    view: Entity<DatabaseTabView>,
}

impl IntoElement for ResizeEventHandler {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for ResizeEventHandler {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<gpui::ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static std::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (gpui::LayoutId, Self::RequestLayoutState) {
        (window.request_layout(Style::default(), None, cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        let bounds = window.bounds();
        self.view.update(cx, |view, _| {
            view.bounds = Bounds {
                origin: Point::default(),
                size: bounds.size,
            };
        });
    }

    fn paint(
        &mut self,
        _: Option<&gpui::GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut Self::RequestLayoutState,
        _: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        window.on_mouse_event({
            let view = self.view.clone();
            let resizing = view.read(cx).resizing;
            move |e: &MouseMoveEvent, phase, window, cx| {
                if resizing.is_none() {
                    return;
                }
                if !phase.bubble() {
                    return;
                }
                view.update(cx, |view, cx| view.resize(e.position, window, cx));
            }
        });

        window.on_mouse_event({
            let view = self.view.clone();
            move |_: &MouseUpEvent, phase, window, cx| {
                if phase.bubble() {
                    view.update(cx, |view, cx| view.done_resizing(window, cx));
                }
            }
        });
    }
}
