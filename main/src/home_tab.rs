use std::collections::HashSet;
use std::ops::Range;
use std::sync::Arc;

use db::ipc::IpcDriverRegistry;
use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, AnyElement, App, AppContext, AsyncApp, ClipboardItem, Context, ElementId, Entity,
    EventEmitter, FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding,
    ListSizingBehavior, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    Subscription, UniformListScrollHandle, WeakEntity, Window, actions, div, px, uniform_list,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, InteractiveElementExt, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _, DropdownButton},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::{List, ListState},
    menu::{DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    popover::Popover,
    tooltip::Tooltip,
    v_flex,
};
use mongodb_view::{MongoFormWindow, MongoFormWindowConfig};
use mqtt_view::{MqttFormConfig, MqttFormWindow};
use one_core::cloud_sync::{
    CloudAccountScope, CloudApiClient, CloudSyncService, ConflictResolution, SyncConflict,
    SyncEngine, TeamOption, UserInfo, get_cached_team_display_options_for_scope,
    get_cached_team_options,
};
use one_core::config::{team_management_url_template, website_base_url};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::key_storage;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::license::Feature;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
use one_core::settings::{AppSettings, HomeConnectionLayout, HomePageStyle, SyncProvider};
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
    ConnectionVisualSize, connection_type_navigation_icon, connection_type_rail_icon,
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
use crate::user_avatar::render_user_avatar;
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

const MODERN_HOME_CARD_MIN_WIDTH: gpui::Pixels = px(220.0);
const MODERN_HOME_CARD_MAX_WIDTH: gpui::Pixels = px(260.0);
const HOME_CONNECTION_LIST_ACTIONS_WIDTH: gpui::Pixels = px(136.0);
const HOME_SIDEBAR_EXPANDED_WIDTH: gpui::Pixels = px(220.0);
const HOME_SIDEBAR_COLLAPSED_WIDTH: gpui::Pixels = px(68.0);
// HomePage Entity - 管理 home 页面的所有状态

/// 连接列表布局模式
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum ConnectionLayout {
    /// 卡片网格视图
    Card,
    /// 长条列表视图
    List,
}

impl ConnectionLayout {
    fn toggle(self) -> Self {
        match self {
            ConnectionLayout::Card => ConnectionLayout::List,
            ConnectionLayout::List => ConnectionLayout::Card,
        }
    }
}

impl From<HomeConnectionLayout> for ConnectionLayout {
    fn from(layout: HomeConnectionLayout) -> Self {
        match layout {
            HomeConnectionLayout::Card => Self::Card,
            HomeConnectionLayout::List => Self::List,
        }
    }
}

impl From<ConnectionLayout> for HomeConnectionLayout {
    fn from(layout: ConnectionLayout) -> Self {
        match layout {
            ConnectionLayout::Card => Self::Card,
            ConnectionLayout::List => Self::List,
        }
    }
}

pub struct HomePage {
    focus_handle: FocusHandle,
    pub(crate) home_active: bool,
    pub(crate) selected_filter: ConnectionType,
    connection_layout: ConnectionLayout,
    home_page_style: HomePageStyle,
    sidebar_collapsed: bool,
    persistent_sidebar_expanded: bool,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) connections: Vec<StoredConnection>,
    pub(crate) tab_container: Entity<TabContainer>,
    search_input: Entity<InputState>,
    search_query: Entity<String>,
    pub(crate) editing_connection_id: Option<i64>,
    pub(crate) selected_connection_id: Option<i64>,
    connection_scroll_handle: UniformListScrollHandle,
    pub(crate) filtered_workspace_ids: HashSet<i64>,
    pub(crate) workspace_filter_open: bool,
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
mod batch_connection_actions;
mod cloud_sync;
mod connection_actions;
mod connection_badge;
mod connection_card;
mod connection_card_actions;
mod connection_card_content;
mod connection_details;
mod connection_filter;
mod connection_form_title;
mod connection_forms;
mod connection_grouping;
mod connection_icon;
mod connection_info;
mod connection_list;
mod connection_list_actions;
mod connection_open;
pub(crate) use connection_open::resolve_connection_credentials;
mod content;
mod data;
mod encryption;
mod forwarding;
mod keybindings;
mod legacy_home;
mod lifecycle;
mod local_terminal;
mod modern_home;
mod modern_home_shortcuts;
mod navigation;
mod render;
mod sidebar;
mod sidebar_navigation;
mod sync_route;
mod team_permissions;
mod toolbar;
mod workspace;
mod workspace_filter;

use connection_badge::ConnectionTeamBadge;
pub(crate) use connection_badge::connection_team_badge;
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
pub(crate) use sync_route::should_show_team_management_entry;
use sync_route::{
    HomeSyncButtonContext, HomeSyncButtonState, HomeSyncRoute, home_sync_button_state,
    refreshed_pending_conflicts, should_auto_onet_cloud_sync, should_show_team_key_menu_item,
    sync_route,
};
pub(crate) use team_permissions::TeamPermissionSnapshot;

#[cfg(test)]
use connection_info::remote_desktop_connection_info;
#[cfg(test)]
use sync_route::sync_route_for_provider;
#[cfg(test)]
mod tests;
