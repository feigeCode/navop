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
use crate::sidebar::execution_history_panel::ExecutionHistoryPanel;
use crate::sidebar::{DatabaseSidebar, DatabaseSidebarEvent};
use crate::sql_editor_view::SqlEditorTab;
use crate::sql_library::{SqlLibraryEvent, SqlLibraryPanel};
use ai_chat_view::{CodeBlockAction, LanguageMatcher};
use db::{
    DbNodeType, GlobalDbState,
    ipc::{IpcDriverRegistry, driver_icon_from_asset_path, driver_icon_from_file_path},
};
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Axis, Bounds, Context, Element, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, MouseMoveEvent,
    MouseUpEvent, ParentElement, Pixels, Point, Render, SharedString,
    StatefulInteractiveElement as _, Style, Styled, Task, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::WindowExt;
use gpui_component::{
    ActiveTheme, ElementExt as _, Icon, Sizable, Size, h_flex, notification::Notification, v_flex,
};
use one_assets::IconName;
use one_core::layout::{SIDEBAR_MAX_WIDTH, SIDEBAR_MIN_WIDTH, TOOLBAR_WIDTH};
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
const DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH: Pixels = px(400.0);
/// 扩展面板停靠底部时的初始高度（通栏，不参与左右侧的拖拽调宽）。
const DATABASE_EXTENSION_BOTTOM_PANEL_HEIGHT: Pixels = px(300.0);
/// 底部扩展面板可拖到的最小高度，避免被拖成一条看不见的缝。
const DATABASE_EXTENSION_BOTTOM_PANEL_MIN_HEIGHT: Pixels = px(120.0);

fn resized_tree_panel_size(
    mouse_x: Pixels,
    view_bounds: Bounds<Pixels>,
    sidebar_width: Pixels,
) -> Pixels {
    let new_size = mouse_x - view_bounds.left();
    let max_size = (view_bounds.size.width - PANEL_MIN_SIZE - sidebar_width).max(PANEL_MIN_SIZE);
    new_size.clamp(PANEL_MIN_SIZE, max_size)
}

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
    /// 底部扩展面板的上下拖拽。
    BottomExtension,
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
    bottom_extension_panel_height: Pixels,
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
        crate::db_tree_registry::register_db_tree_view(db_tree_view.clone(), cx);

        let tab_container =
            cx.new(|cx| TabContainer::new(window, cx).with_background_task_panel(false));
        // 右键菜单里 `kind: "nsh"` 的扩展页要开在这个页签容器里（与查询页签并排），
        // 而不是主窗口的页签栏。
        db_tree_view.update(cx, |view, _| {
            view.set_extension_tab_container(tab_container.downgrade())
        });

        let objects_panel = cx.new(|cx| DatabaseObjectsPanel::new(workspace.clone(), window, cx));

        tab_container.update(cx, |container, cx| {
            let panel = objects_panel.clone();
            let tab = TabItem::new("objects-panel", "database", panel);
            container.set_pinned_tab(tab, cx);
        });

        let status_msg = cx.new(|_| "Ready".to_string());
        let is_connected = cx.new(|_| true);
        let connection_ids = connections
            .iter()
            .filter_map(|connection| connection.id.map(|id| id.to_string()))
            .collect::<Vec<_>>();
        let execution_history = cx.new({
            let connection_ids = connection_ids.clone();
            move |cx| ExecutionHistoryPanel::new(connection_ids, cx)
        });

        let sql_library = cx.new(SqlLibraryPanel::new);

        let event_handler = cx.new(|cx| {
            DatabaseEventHandler::new(
                &db_tree_view,
                tab_container.clone(),
                objects_panel.clone(),
                connection_ids.clone(),
                execution_history.clone(),
                window,
                cx,
            )
        });

        let sidebar = cx.new(|cx| {
            DatabaseSidebar::new(
                connections.clone(),
                active_conn_id,
                execution_history.clone(),
                sql_library.clone(),
                window,
                cx,
            )
        });
        // 面板里 `App.open` 要落在数据库页签栏里，不能跳到主窗口。
        sidebar.update(cx, |sidebar, _| {
            sidebar.set_extension_tab_container(tab_container.downgrade())
        });

        // 扩展面板（anchor: "db-sidebar"）：构造时对齐一次宿主提供的规格。
        sidebar.update(cx, |sidebar, cx| sidebar.sync_extension_panels(cx));

        // 注册 SQL 代码块操作
        Self::register_sql_code_block_actions(
            &sidebar,
            tab_container.clone(),
            &connections,
            execution_history,
            cx,
        );

        let mut subscriptions = Vec::new();
        subscriptions.push(cx.subscribe_in(
            &sql_library,
            window,
            |this, _library, event: &SqlLibraryEvent, window, cx| {
                let SqlLibraryEvent::OpenSqlFile(path) = event;
                this.open_sql_library_file(path.clone(), window, cx);
            },
        ));
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
                    this.sidebar_panel_size = this
                        .sidebar_panel_size
                        .max(DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH);
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
            sidebar_panel_size: DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH,
            bottom_extension_panel_height: DATABASE_EXTENSION_BOTTOM_PANEL_HEIGHT,
            sidebar_render_mode: DatabaseSidebarRenderMode::Embedded,
            resizing: None,
            bounds: Bounds::default(),
        }
    }

    pub fn with_external_sidebar(mut self) -> Self {
        self.sidebar_render_mode = DatabaseSidebarRenderMode::External;
        self
    }

    /// 只移动既有对象树的宿主，不重建连接、编辑器或 AI 工具面板。
    pub fn set_external_sidebar(&mut self, external: bool, cx: &mut Context<Self>) {
        let mode = if external {
            DatabaseSidebarRenderMode::External
        } else {
            DatabaseSidebarRenderMode::Embedded
        };
        if self.sidebar_render_mode != mode {
            self.sidebar_render_mode = mode;
            cx.notify();
        }
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

    /// SQL 库文件默认用的连接：当前选中节点所属连接，否则第一个连接。
    fn library_connection(&self, cx: &Context<Self>) -> Option<(String, DbConnectionConfig)> {
        let selected_id = self.db_tree_view.read(cx).selected_or_first_connection_id();
        let connection = self
            .connections
            .iter()
            .find(|connection| {
                connection.id.map(|id| id.to_string()).as_deref() == selected_id.as_deref()
            })
            .or_else(|| self.connections.first())?
            .clone();
        let connection_id = connection.id.map(|id| id.to_string()).unwrap_or_default();
        let config = connection.to_db_connection().ok()?;
        Some((connection_id, config))
    }

    /// 用 SQL 编辑器打开 SQL 库里的文件。
    ///
    /// 库文件不绑定连接：默认用当前选中的连接（可在编辑器里再切），
    /// 同一个文件复用同一个页签。
    fn open_sql_library_file(
        &mut self,
        file_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some((connection_id, config)) = self.library_connection(cx) else {
            window.push_notification(
                Notification::info(t!("SqlLibrary.no_connection").to_string()),
                cx,
            );
            return;
        };

        let title: SharedString = file_path
            .file_stem()
            .map(|stem| stem.to_string_lossy().into_owned())
            .unwrap_or_else(|| file_path.display().to_string())
            .into();
        let tab_id = format!("sql-library-{}", file_path.display());
        let tab_id_for_tab = tab_id.clone();
        let connection_id_for_tab = connection_id.clone();
        let available_connection_ids = self
            .connections
            .iter()
            .filter_map(|connection| connection.id.map(|id| id.to_string()))
            .collect::<Vec<_>>();
        let execution_history = self.sidebar.read(cx).execution_history();
        let host_tab_container = self.tab_container.clone();

        self.tab_container.update(cx, |container, cx| {
            container.activate_or_add_tab_lazy(
                tab_id,
                move |window, cx| {
                    let editor = cx.new(|cx| {
                        SqlEditorTab::new_with_config(
                            crate::sql_editor_view::SqlEditorTabConfig {
                                title: title.clone(),
                                connection_id: connection_id.clone(),
                                available_connection_ids: available_connection_ids.clone(),
                                database_type: config.database_type.clone(),
                                file_path: Some(file_path.clone()),
                                new_file_directory: None,
                                initial_database: None,
                                initial_schema: None,
                                execution_history: execution_history.clone(),
                                tab_container: host_tab_container.clone(),
                            },
                            window,
                            cx,
                        )
                    });
                    TabItem::new(
                        tab_id_for_tab.clone(),
                        connection_id_for_tab.clone(),
                        editor,
                    )
                },
                window,
                cx,
            );
        });
    }

    /// 注册 SQL 代码块操作
    fn register_sql_code_block_actions(
        sidebar: &Entity<DatabaseSidebar>,
        tab_container: Entity<TabContainer>,
        connections: &[StoredConnection],
        execution_history: Entity<ExecutionHistoryPanel>,
        cx: &mut App,
    ) {
        // 获取第一个连接的信息用于创建新编辑器
        let first_conn = connections.first().cloned();
        let available_connection_ids = connections
            .iter()
            .filter_map(|connection| connection.id.map(|id| id.to_string()))
            .collect::<Vec<_>>();

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
        let execution_history_for_new = execution_history;
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
                let available_connection_ids = available_connection_ids.clone();
                let execution_history = execution_history_for_new.clone();
                // 详情页签是编辑器页签的兄弟：先把这个容器句柄带进内层闭包。
                let host_tab_container = tab_container_for_new.clone();

                tab_container_for_new.update(cx, |container, cx| {
                    container.activate_or_add_tab_lazy(
                        tab_id.clone(),
                        move |window, cx| {
                            let sql_editor = cx.new(|cx| {
                                let editor = SqlEditorTab::new_with_config(
                                    crate::sql_editor_view::SqlEditorTabConfig {
                                        title: "AI Query".into(),
                                        connection_id: connection_id.clone(),
                                        available_connection_ids: available_connection_ids.clone(),
                                        database_type,
                                        file_path: None,
                                        new_file_directory: None,
                                        initial_database: None,
                                        initial_schema: None,
                                        execution_history,
                                        tab_container: host_tab_container.clone(),
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

    fn render_bottom_extension_resize_handle(
        &mut self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();

        resize_handle::<ResizePanel, ResizePanel>(
            "database-extension-bottom-resize",
            Axis::Vertical,
        )
        .inline()
        .on_drag(ResizePanel, move |info, _, _, cx| {
            cx.stop_propagation();
            view.update(cx, |view, cx| {
                view.resizing = Some(ResizingPanel::BottomExtension);
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
                let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
                let sidebar_width = if sidebar_visible {
                    self.sidebar_panel_size
                } else {
                    TOOLBAR_WIDTH
                };
                self.tree_panel_size =
                    resized_tree_panel_size(mouse_position.x, self.bounds, sidebar_width);
            }
            ResizingPanel::Sidebar => {
                let new_size = self.bounds.right() - mouse_position.x;
                let max_size = (available_width - self.tree_panel_size - PANEL_MIN_SIZE)
                    .max(SIDEBAR_MIN_WIDTH);
                let upper = max_size.min(SIDEBAR_MAX_WIDTH);
                let lower = SIDEBAR_MIN_WIDTH.max(CHAT_SIDEBAR_MIN_WIDTH).min(upper);
                self.sidebar_panel_size = new_size.clamp(lower, upper);
            }
            ResizingPanel::BottomExtension => {
                // 面板贴着内容区底部，鼠标位置决定它的高度。
                let new_size = self.bounds.bottom() - mouse_position.y;
                let upper = (self.bounds.size.height - PANEL_MIN_SIZE)
                    .max(DATABASE_EXTENSION_BOTTOM_PANEL_MIN_HEIGHT);
                self.bottom_extension_panel_height =
                    new_size.clamp(DATABASE_EXTENSION_BOTTOM_PANEL_MIN_HEIGHT, upper);
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
            .id("database-workspace-toolbar")
            .h(WORKSPACE_TOOLBAR_HEIGHT)
            .w_full()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_3()
            .py_1()
            .overflow_x_scroll()
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
                                this.bg(gpui::red())
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
                    .child(t!("Connection.connection_details", name = conn_name).to_string()),
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
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{}:", t!("ConnectionForm.host"))),
                            )
                            .child(conn_host),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{}:", t!("ConnectionForm.port"))),
                            )
                            .child(format!("{}", conn_port)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .font_weight(FontWeight::SEMIBOLD)
                                    .child(format!("{}:", t!("ConnectionForm.username"))),
                            )
                            .child(conn_username),
                    )
                    .when_some(conn_database, |this, db| {
                        this.child(
                            h_flex()
                                .gap_2()
                                .child(
                                    div()
                                        .font_weight(FontWeight::SEMIBOLD)
                                        .child(format!("{}:", t!("ConnectionForm.database"))),
                                )
                                .child(db),
                        )
                    }),
            )
            .child(
                div()
                    .text_lg()
                    .when(!is_error, |this| this.text_color(cx.theme().accent))
                    .when(is_error, |this| this.text_color(gpui::red()))
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
    fn set_external_primary_sidebar(&mut self, external: bool, cx: &mut Context<Self>) {
        self.set_external_sidebar(external, cx);
    }

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

    fn sidebar_contributions(&self, _cx: &App) -> Vec<SidebarContribution> {
        if self.sidebar_render_mode != DatabaseSidebarRenderMode::External {
            return Vec::new();
        }

        // 底部通栏面板由页签自己渲染（外置侧栏模式下宿主 dock 看不到内容区），
        // 此时侧栏只剩工具条，不能按「面板可见」给它留出整条宽度。
        let database_tools_panel_visible = {
            let sidebar = self.sidebar.read(_cx);
            sidebar.is_panel_visible()
                && sidebar.active_extension_placement() != Some(SidebarPlacement::Bottom)
        };

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
                // Database Tools 的初始停靠与扩展面板的 placement 无关：
                // 扩展要求的是它自己的面板位置，不是工具侧栏的位置。
                default_placement: SidebarPlacement::Right,
                policy: database_tools_sidebar_policy(),
                style: SidebarPanelStyle::default(),
                size: database_tools_sidebar_size(
                    database_tools_panel_visible,
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
            credential_reference: None,
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
        let collapsed = database_tools_sidebar_size(false, DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH);
        let expanded = database_tools_sidebar_size(true, DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH);

        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.side_width);
        assert_eq!(Some(TOOLBAR_WIDTH), collapsed.bottom_height);
        assert_eq!(
            Some(DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH + TOOLBAR_WIDTH),
            expanded.side_width
        );
        assert_eq!(
            Some(DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH),
            expanded.bottom_height
        );
        assert_eq!(px(400.0), DATABASE_TOOLS_SIDEBAR_DEFAULT_WIDTH);
    }

    #[test]
    fn database_tools_sidebar_does_not_render_host_header() {
        assert_eq!(
            SidebarPanelChrome::HostNoHeader,
            database_tools_sidebar_chrome()
        );
    }

    /// Database Tools 的初始停靠是它自己的事，不跟着扩展面板的 placement 走。
    ///
    /// 这里只看生产代码（`include_str!` 会把测试模块也读进来）。
    #[test]
    fn database_tools_tab_placement_does_not_follow_extension_placement() {
        let source = include_str!("database_tab.rs");
        let production = source.split("#[cfg(test)]").next().unwrap_or(source);

        assert!(
            production.contains("default_placement: SidebarPlacement::Right,"),
            "Database Tools 页签的初始停靠应固定为右侧"
        );

        let tools_start = production
            .find("SharedString::from(\"Database Tools\")")
            .expect("Database Tools 页签贡献");
        let tools = &production[tools_start..];
        let tools = &tools[..tools
            .find("database_tools_sidebar_size(")
            .expect("Database Tools 尺寸规格")];
        assert!(
            !tools.contains("active_extension_placement()"),
            "Database Tools 的默认停靠又变回跟随扩展面板了"
        );
    }

    /// 外置侧栏模式下内容区由宿主 dock 包着，底部面板只能由页签自己渲染。
    #[test]
    fn external_sidebar_mode_still_renders_the_bottom_extension_panel() {
        let source = include_str!("database_tab.rs");
        // `impl Render` 后面紧跟测试模块；不截掉测试自身，断言里的字面量会自指恒真。
        let render = source
            .rsplit_once("impl Render for DatabaseTabView")
            .expect("database tab render implementation")
            .1;
        let render = render.split("#[cfg(test)]").next().unwrap_or(render);

        assert!(render.contains("DatabaseSidebarRenderMode::External"));
        assert!(
            render.contains("render_bottom_extension_panel("),
            "外置侧栏模式没有画底部扩展面板，`placement: bottom` 会彻底不可见"
        );
        assert!(
            render.contains(".on_prepaint("),
            "外置侧栏模式需要自己的 bounds 才能算出底部面板的拖拽高度"
        );
    }

    #[test]
    fn tree_panel_resize_uses_database_tab_origin_in_nested_layouts() {
        let view_bounds = Bounds {
            origin: Point::new(px(340.0), px(80.0)),
            size: gpui::Size {
                width: px(1000.0),
                height: px(700.0),
            },
        };

        assert_eq!(
            px(250.0),
            resized_tree_panel_size(px(590.0), view_bounds, px(360.0))
        );
    }

    #[test]
    fn database_tab_captures_rendered_bounds_instead_of_window_bounds() {
        let source = include_str!("database_tab.rs");
        let render_implementation = source
            .rsplit_once("impl Render for DatabaseTabView")
            .expect("database tab render implementation")
            .1;

        // bounds 捕获在 connected-workspace 助手里（底部停靠面板需要包一层），
        // 因此 on_prepaint 断言针对整个文件，window.bounds 禁令仍只看 render。
        assert!(source.contains(".on_prepaint("));
        assert!(source.contains("view.bounds = bounds"));
        assert!(!render_implementation.contains("window.bounds()"));
    }
}

impl DatabaseTabView {
    /// 侧栏在内容区左/右边缘的容器；`visible = false` 时只留工具条宽度。
    fn render_side_sidebar(
        &mut self,
        visible: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        if visible {
            return div()
                .relative()
                .h_full()
                .w(self.sidebar_panel_size)
                .flex_shrink_0()
                .overflow_hidden()
                .child(self.render_sidebar_resize_handle(window, cx))
                .child(self.sidebar.clone())
                .into_any_element();
        }
        div()
            .h_full()
            .w(TOOLBAR_WIDTH)
            .flex_shrink_0()
            .overflow_hidden()
            .child(self.sidebar.clone())
            .into_any_element()
    }

    /// 扩展声明 `placement: "bottom"` 时，面板渲染成内容区下方的通栏。
    fn render_bottom_extension_panel(
        &mut self,
        id: SharedString,
        title: SharedString,
        view: gpui::AnyView,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sidebar = self.sidebar.clone();
        let theme = cx.theme();
        let border_color = theme.border;
        let background = theme.background;
        v_flex()
            .w_full()
            .h(self.bottom_extension_panel_height)
            .flex_shrink_0()
            .bg(background)
            .child(self.render_bottom_extension_resize_handle(cx))
            .child(
                h_flex()
                    .flex_shrink_0()
                    .h(one_ui::theme_geometry().layout.panel_header)
                    .items_center()
                    .px_2()
                    .border_b_1()
                    .border_color(border_color)
                    .child(div().flex_1().min_w_0().truncate().text_sm().child(title))
                    .child(
                        one_ui::IconButton::new(
                            format!("database-sidebar-ext-bottom-close-{id}"),
                            IconName::Close,
                        )
                        .glyph_size(one_ui::IconSize::Small)
                        .tooltip(t!("DatabaseSidebar.close_extension").to_string())
                        .on_click(move |_, _, cx| {
                            sidebar.update(cx, |sidebar, cx| {
                                sidebar.close_extension_panel(&id, cx);
                            });
                        }),
                    ),
            )
            .child(div().flex_1().min_h_0().overflow_hidden().child(view))
            .into_any_element()
    }

    fn render_connected_workspace(
        &mut self,
        view: Entity<Self>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let border_color = cx.theme().border;
        let tree_panel_size = self.tree_panel_size;
        let sidebar_visible = self.sidebar.read(cx).is_panel_visible();
        let placement = self
            .sidebar
            .read(cx)
            .active_extension_placement()
            .unwrap_or(SidebarPlacement::Right);
        let bottom_panel = self.sidebar.read(cx).bottom_extension_panel();
        let docked_bottom = bottom_panel.is_some();
        let sidebar_left = placement == SidebarPlacement::Left && !docked_bottom;

        let row = h_flex()
            .size_full()
            .on_prepaint({
                let bounds_view = view.clone();
                move |bounds, _, cx| {
                    bounds_view.update(cx, |view, _| {
                        view.bounds = bounds;
                    });
                }
            })
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
            .when(sidebar_left, |this| {
                this.child(self.render_side_sidebar(sidebar_visible, window, cx))
            })
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_w_0()
                    .child(self.render_workspace_content(cx)),
            )
            .when(!sidebar_left, |this| {
                this.child(self.render_side_sidebar(sidebar_visible && !docked_bottom, window, cx))
            })
            .child(ResizeEventHandler { view });

        let Some((id, title, content)) = bottom_panel else {
            return row.into_any_element();
        };
        v_flex()
            .size_full()
            .min_h_0()
            .child(div().flex_1().min_h_0().child(row))
            .child(self.render_bottom_extension_panel(id, title, content, cx))
            .into_any_element()
    }
}

impl Render for DatabaseTabView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_connected_flag = *self.is_connected.read(cx);
        let view = cx.entity().clone();

        if is_connected_flag && self.sidebar_render_mode == DatabaseSidebarRenderMode::External {
            // 外置侧栏模式下内容区由宿主 dock 包着，底部通栏只能由页签自己画，
            // 否则侧栏与页签都以为对方会渲染，`placement: "bottom"` 就完全不可见。
            let bottom_panel = self.sidebar.read(cx).bottom_extension_panel();
            let bounds_view = view.clone();
            let mut workspace = v_flex()
                .track_focus(&self.focus_handle)
                .size_full()
                .min_h_0()
                .on_prepaint(move |bounds, _, cx| {
                    bounds_view.update(cx, |view, _| {
                        view.bounds = bounds;
                    });
                })
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .child(self.render_workspace_content(cx)),
                );
            if let Some((id, title, content)) = bottom_panel {
                workspace =
                    workspace.child(self.render_bottom_extension_panel(id, title, content, cx));
            }
            // 拖拽调高底部面板需要这个元素持续上报鼠标位移。
            return workspace.child(ResizeEventHandler { view });
        }

        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .when(!is_connected_flag, |el: gpui::Div| {
                el.child(self.render_connection_status(cx))
            })
            .when(is_connected_flag, |el: gpui::Div| {
                el.child(self.render_connected_workspace(view.clone(), window, cx))
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
        _window: &mut Window,
        _cx: &mut App,
    ) -> Self::PrepaintState {
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
