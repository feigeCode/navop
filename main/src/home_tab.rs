use std::collections::HashSet;
use std::sync::Arc;

use db::ipc::IpcDriverRegistry;
use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use gpui:: as _;
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, AnyElement, App, AppContext, AsyncApp, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    Window, actions, div, px,
};
use gpui_component::{ActiveTheme, Icon, InteractiveElementExt, Sizable, Size, WindowExt, button::{Button, ButtonVariants as _, DropdownButton}, checkbox::Checkbox, dialog::DialogButtonProps, h_flex, input::{Input, InputEvent, InputState}, list::{List, ListState}, menu::{ContextMenuExt, DropdownMenu as _, PopupMenuItem}, notification::Notification, popover::Popover, tooltip::Tooltip, v_flex};
use one_ui::IconSize;
use one_assets::IconName;
use mongodb_view::{MongoFormWindow, MongoFormWindowConfig};
use mqtt_view::{MqttFormConfig, MqttFormWindow};
use one_core::cloud_sync::{
    CloudAccountScope, CloudApiClient, CloudSyncService, SyncConflict, SyncEngine, TeamOption,
    UserInfo, get_cached_team_display_options_for_scope, get_cached_team_options,
};
use one_core::config::{team_management_url_template, website_base_url};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::key_storage;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::license::Feature;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::settings::{AppSettings, HomeConnectionLayout, SyncProvider};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ActiveConnections, ConnectionRepository, ConnectionType, CredentialResolutionError,
    DatabaseType, GlobalStorageState, PendingCloudDeletionRepository, RedisMode,
    RemoteDesktopParams, RemoteDesktopProtocol as StoredRemoteDesktopProtocol, SshAuthMethod,
    StoredConnection, TeamMembershipState, TelnetLoginStep, Workspace, WorkspaceRepository,
};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent, TabItem, TabOpenMode};
use one_ui::{IconButton, IconButtonRole};
use port_forwarding::PortForwardingRuntime;
use port_forwarding_view::{
    PortForwardingFormWindow, PortForwardingFormWindowConfig, PortForwardingTab,
    PortForwardingTabConfig,
};
use redis_view::{RedisFormWindow, RedisFormWindowConfig};
use rust_i18n::t;
use terminal_view::{SerialFormWindow, SerialFormWindowConfig};
use terminal_view::{SshFormWindow, SshFormWindowConfig};
use terminal_view::{TelnetFormWindow, TelnetFormWindowConfig};

use crate::auth::{AuthService, load_auth_data, show_auth_dialog};
use crate::connection_visuals::{
    ConnectionVisualSize, connection_type_label, connection_type_navigation_icon,
};
use crate::home::connection_import_window::show_connection_import_window;
use crate::home::home_connection_quick_open::ConnectionQuickOpenDelegate;
use crate::home::home_strategy::build_connection_open_strategy;
use crate::home::home_workspace_filter::{
    WorkspaceDialogConfig, WorkspaceFilterDelegate, show_workspace_dialog,
};
use crate::license::{get_license_service, is_feature_enabled, show_upgrade_dialog};
use crate::local_terminal_profiles::{
    LocalTerminalLaunchTarget, effective_kind, launch_options, launch_target_is_default,
};
use crate::new_connection::NewConnectionWindow;
use crate::setting_tab::GlobalCurrentUser;
use crate::team_management::{build_team_management_url, resolve_team_management_url};
use remote_desktop_view::remote_desktop_form::{
    RemoteDesktopFormWindow, RemoteDesktopFormWindowConfig,
};

actions!(
    home_tab,
    [
        OpenConnectionQuickOpen,
        NewConnectionShortcut,
        OpenLocalTerminalShortcut
    ]
);

const HOME_CONNECTION_LIST_ACTIONS_WIDTH: gpui::Pixels = px(96.0);
// 侧栏舒适宽度收窄（redesign §8.2），把空间让给主工作面。
const HOME_SIDEBAR_EXPANDED_WIDTH: gpui::Pixels = px(184.0);
const HOME_SIDEBAR_COLLAPSED_WIDTH: gpui::Pixels = px(58.0);
/// 最近连接区的历史语义图标（gpui-component IconName 无 history 变体，内嵌提供）。
pub(crate) const NAVOP_HISTORY_ICON: &str = "navop/history.svg";
/// 首页导航/Tab 的线性 Home 图标（依赖库 home.svg 为固定填充色，改用自有线稿）。
pub(crate) const NAVOP_HOME_LINE_ICON: &str = "navop/home-line.svg";
// HomePage Entity - 管理 home 页面的所有状态

/// 连接列表布局模式
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionLayout {
    /// 卡片网格视图
    Card,
    /// 长条列表视图
    List,
    /// 分组树视图（复用常驻侧栏连接树）
    Tree,
}

impl From<HomeConnectionLayout> for ConnectionLayout {
    fn from(layout: HomeConnectionLayout) -> Self {
        match layout {
            HomeConnectionLayout::Card => Self::Card,
            HomeConnectionLayout::List => Self::List,
            HomeConnectionLayout::Tree => Self::Tree,
        }
    }
}

impl From<ConnectionLayout> for HomeConnectionLayout {
    fn from(layout: ConnectionLayout) -> Self {
        match layout {
            ConnectionLayout::Card => Self::Card,
            ConnectionLayout::List => Self::List,
            ConnectionLayout::Tree => Self::Tree,
        }
    }
}

impl HomePage {
    pub(crate) fn set_connection_sidebar(
        &mut self,
        sidebar: Entity<crate::persistent_connection_sidebar::PersistentConnectionSidebar>,
    ) {
        self.connection_sidebar = Some(sidebar);
    }

    pub(crate) fn recent_connections_collapsed(&self) -> bool {
        self.recent_collapsed
    }

    pub(crate) fn toggle_recent_connections(&mut self, cx: &mut Context<Self>) {
        self.recent_collapsed = !self.recent_collapsed;
        cx.notify();
    }
}

pub struct HomePage {
    focus_handle: FocusHandle,
    pub(crate) selected_filter: ConnectionType,
    connection_layout: ConnectionLayout,
    sidebar_collapsed: bool,
    collapsed_groups: HashSet<Option<i64>>,
    recent_collapsed: bool,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) connections: Vec<StoredConnection>,
    pub(crate) tab_container: Entity<TabContainer>,
    /// 常驻连接侧栏（Tree 布局复用其树视图）；在 OnetCliApp 创建侧栏后注入。
    connection_sidebar:
        Option<Entity<crate::persistent_connection_sidebar::PersistentConnectionSidebar>>,
    search_input: Entity<InputState>,
    /// 工具栏搜索词（嵌入主页的连接树也以此为过滤输入，树内不再重复搜索框）。
    pub(crate) search_query: Entity<String>,
    pub(crate) editing_connection_id: Option<i64>,
    pub(crate) selected_connection_id: Option<i64>,
    /// 批量操作选择状态；卡片/列表/树三种布局共享（参考常驻侧栏连接树）。
    pub(crate) connection_selection: connection_selection::ConnectionSelection,
    pub(crate) filtered_workspace_ids: HashSet<i64>,
    pub(crate) workspace_filter_open: bool,
    pub(crate) account_menu_open: bool,
    workspace_filter_list: Option<Entity<ListState<WorkspaceFilterDelegate>>>,
    pub(crate) _subscriptions: Vec<Subscription>,
    /// 云同步服务
    cloud_sync_service: Arc<std::sync::RwLock<CloudSyncService>>,
    /// 云端加载错误信息
    cloud_error: Option<String>,
    /// 是否正在同步
    syncing: bool,
    /// 同步期间收到的新同步请求
    sync_requested: bool,
    /// 待处理的同步冲突
    pending_conflicts: Vec<SyncConflict>,
    /// 认证服务
    auth_service: Arc<AuthService>,
    /// 当前登录用户
    pub(crate) current_user: Option<UserInfo>,
    /// 是否正在登录
    logging_in: bool,
    /// 认证错误消息（登录/注册失败时设置）
    auth_error: Option<String>,
    /// 启动恢复主密钥失败后，在首帧延迟弹出解锁对话框。
    master_key_unlock_prompt_pending: bool,
    /// 防止主密钥对话框被启动提示和用户点击重复打开。
    master_key_dialog_open: bool,
    team_permissions: TeamPermissionSnapshot,
    port_forwarding_runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    pub(crate) external_driver_registry: IpcDriverRegistry,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ConnectionCredentialExportIdentity {
    team_id: Option<String>,
    owner_id: Option<String>,
}

impl ConnectionCredentialExportIdentity {
    fn from_connection(connection: &StoredConnection) -> Self {
        Self {
            team_id: connection.team_id.clone(),
            owner_id: connection.owner_id.clone(),
        }
    }

    fn matches(&self, connection: &StoredConnection) -> bool {
        self.team_id == connection.team_id && self.owner_id == connection.owner_id
    }
}

mod auth;
mod batch_bar;
mod batch_connection_actions;
mod cloud_sync;
mod connection_actions;
mod connection_badge;
mod connection_card;
mod connection_card_actions;
mod connection_card_content;
mod connection_details;
pub(crate) mod connection_filter;
mod connection_form_title;
mod connection_forms;
mod connection_grouping;
mod connection_icon;
mod connection_info;
mod connection_list;
mod connection_list_actions;
mod connection_open;
pub(crate) use connection_open::resolve_connection_credentials;
mod account_menu;
pub(crate) mod connection_selection;
mod content;
mod data;
mod encryption;
mod forwarding;
mod grid;
mod home_layout;
mod home_shortcuts;
mod keybindings;
mod lifecycle;
mod local_terminal;
mod navigation;
mod recent;
mod render;
mod sidebar;
mod sidebar_navigation;
mod sync_route;
mod team_permissions;
mod toolbar;
mod workspace;
mod workspace_filter;

pub(crate) use recent::recent_connections;

use connection_badge::ConnectionTeamBadge;
pub(crate) use connection_badge::connection_team_badge;
use connection_badge::render_team_badge;
pub(crate) use connection_filter::connection_matches_query;
use connection_form_title::{external_driver_id_for_connection_form, non_empty_name};
#[cfg(test)]
pub(crate) use connection_grouping::can_manage_connection_with_permissions;
use connection_info::{
    card_connection_info, connection_display_name, generate_duplicate_name,
    port_forwarding_connection_info,
};
pub(super) use keybindings::{
    OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS, OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER,
};
pub use keybindings::{init, refresh_keybindings};
use sync_route::{
    HomeSyncRoute, refreshed_pending_conflicts, should_auto_onet_cloud_sync,
    should_show_team_key_menu_item, sync_route,
};
pub(crate) use team_permissions::TeamPermissionSnapshot;

#[cfg(test)]
use connection_info::remote_desktop_connection_info;
#[cfg(test)]
use sync_route::sync_route_for_provider;
#[cfg(test)]
mod tests;
