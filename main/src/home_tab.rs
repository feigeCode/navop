use std::collections::HashSet;
use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    Anchor, AnyElement, App, AppContext, AsyncApp, ClipboardItem, Context, Entity, EventEmitter,
    FocusHandle, Focusable, FontWeight, InteractiveElement, IntoElement, KeyBinding, ParentElement,
    Pixels, Render, SharedString, StatefulInteractiveElement, Styled, Subscription, WeakEntity,
    Window, actions, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, InteractiveElementExt, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants as _, DropdownButton},
    checkbox::Checkbox,
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    list::{List, ListState},
    menu::{ContextMenuExt, DropdownMenu as _, PopupMenuItem},
    notification::Notification,
    popover::Popover,
    tooltip::Tooltip,
    v_flex,
};
use one_assets::IconName;
use one_core::cloud_sync::{
    CloudAccountScope, CloudApiClient, CloudSyncService, SyncConflict, SyncEngine, SyncError,
    TeamOption, UserInfo, get_cached_team_display_options_for_scope, get_cached_team_options,
};
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event, get_notifier};
use one_core::crypto;
use one_core::gpui_tokio::Tokio;
use one_core::key_storage;
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};
use one_core::license::Feature;
use one_core::popup_window::{PopupWindowOptions, open_reusable_popup_window};
use one_core::settings::{AppSettings, HomeConnectionLayout, SyncProvider};
use one_core::storage::traits::Repository;
use one_core::storage::{
    ActiveConnections, ConnectionRepository, ConnectionType, CredentialResolutionError,
    DatabaseType, GlobalStorageState, PendingCloudDeletionRepository, RedisMode,
    RemoteDesktopParams, SshAuthMethod, StoredConnection, TeamMembershipState, TelnetLoginStep,
    Workspace, WorkspaceRepository,
};
use one_core::tab_container::{TabContainer, TabContent, TabContentEvent, TabOpenMode};
use one_ui::{IconButton, IconButtonRole};
#[cfg(feature = "shell")]
use port_forwarding::PortForwardingRuntime;
use rust_i18n::t;
#[cfg(feature = "shell")]
use terminal_view::{SshFormWindow, SshFormWindowConfig};

use crate::auth::{AuthService, load_auth_data, show_auth_dialog};
use crate::connection_visuals::{ConnectionVisualSize, connection_type_navigation_icon};
use crate::home::connection_import_window::show_connection_import_window;
use crate::home::home_connection_quick_open::ConnectionQuickOpenDelegate;
use crate::home::home_strategy::build_connection_open_strategy;
use crate::home::home_workspace_filter::{
    WorkspaceDialogConfig, WorkspaceFilterDelegate, show_workspace_dialog,
};
use crate::license::{get_license_service, is_feature_enabled, show_upgrade_dialog};
#[cfg(feature = "shell")]
use crate::local_terminal_profiles::{
    LocalTerminalLaunchTarget, effective_kind, launch_options, launch_target_is_default,
};
use crate::setting_tab::GlobalCurrentUser;

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
pub(crate) const GLOBAL_NAV_TREE_WIDTH: Pixels = px(248.0);
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
    /// 全局导航视图：左侧连接树，右侧主页功能入口。
    Navigation,
}

impl From<HomeConnectionLayout> for ConnectionLayout {
    fn from(layout: HomeConnectionLayout) -> Self {
        match layout {
            HomeConnectionLayout::Card => Self::Card,
            HomeConnectionLayout::List => Self::List,
            HomeConnectionLayout::Tree => Self::Tree,
            HomeConnectionLayout::Navigation => Self::Navigation,
        }
    }
}

impl From<ConnectionLayout> for HomeConnectionLayout {
    fn from(layout: ConnectionLayout) -> Self {
        match layout {
            ConnectionLayout::Card => Self::Card,
            ConnectionLayout::List => Self::List,
            ConnectionLayout::Tree => Self::Tree,
            ConnectionLayout::Navigation => Self::Navigation,
        }
    }
}

impl HomePage {
    /// 本地终端按钮；不含终端领域的构建下返回空占位。
    pub(crate) fn local_terminal_button_element(
        &self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        #[cfg(feature = "shell")]
        {
            return self.render_local_terminal_button(window, cx);
        }
        #[cfg(not(feature = "shell"))]
        {
            let _ = (window, cx);
            gpui::div().into_any_element()
        }
    }

    pub(crate) fn set_connection_sidebar(
        &mut self,
        sidebar: Entity<crate::persistent_connection_sidebar::PersistentConnectionSidebar>,
        cx: &mut Context<Self>,
    ) {
        self.connection_sidebar = Some(sidebar);
        self.sync_sidebar_home_embedded(cx);
    }

    pub(crate) fn recent_connections_collapsed(&self) -> bool {
        self.recent_collapsed
    }

    pub(crate) fn toggle_recent_connections(&mut self, cx: &mut Context<Self>) {
        self.recent_collapsed = !self.recent_collapsed;
        cx.notify();
    }

    /// 主页连接树的“未分组”分区是否折叠。
    ///
    /// 复用卡片/列表布局里 `collapsed_groups` 的 `None` 键（语义正是“未分组”），
    /// 这样“全部折叠/全部展开”对三种布局同时生效，无需新增 HomePage 字段。
    pub(crate) fn unassigned_connections_collapsed(&self) -> bool {
        self.collapsed_groups.contains(&None)
    }

    pub(crate) fn toggle_unassigned_connections(&mut self, cx: &mut Context<Self>) {
        if !self.collapsed_groups.remove(&None) {
            self.collapsed_groups.insert(None);
        }
        cx.notify();
    }

    pub(crate) fn uses_global_navigation_layout(&self) -> bool {
        self.connection_layout == ConnectionLayout::Navigation
    }
}

pub struct HomePage {
    focus_handle: FocusHandle,
    /// 类型筛选目标：All / 内置类型 / 某个扩展连接贡献。
    pub(crate) selected_filter: ConnectionFilter,
    /// 连接类型筛选条的动态布局状态（可见类型数量按容器宽度实测决定）。
    connection_type_filter: connection_type_filter_bar::ConnectionTypeFilterBar,
    connection_layout: ConnectionLayout,
    sidebar_collapsed: bool,
    collapsed_groups: HashSet<Option<i64>>,
    recent_collapsed: bool,
    pub(crate) workspaces: Vec<Workspace>,
    pub(crate) connections: Vec<StoredConnection>,
    pub(crate) tab_container: Entity<TabContainer>,
    /// 常驻连接侧栏（Tree 布局复用其树视图）；在 NavopApp 创建侧栏后注入。
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
    #[cfg(feature = "shell")]
    port_forwarding_runtime: Arc<tokio::sync::Mutex<PortForwardingRuntime>>,
    /// Windows 已识别的 WSL 发行版缓存（issue #182）；None=识别中，空列表=无发行版。
    #[cfg(feature = "shell")]
    #[cfg_attr(not(target_os = "windows"), allow(dead_code))]
    pub(crate) wsl_distributions: Option<Arc<Vec<terminal::WslDistribution>>>,
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
pub(crate) use connection_filter::ConnectionFilter;
mod account_menu;
mod connection_forms;
mod connection_grouping;
mod connection_icon;
mod connection_info;
mod connection_list;
mod connection_list_actions;
mod connection_open;
pub(crate) mod connection_selection;
mod connection_type_filter_bar;
mod content;
mod data;
pub(crate) mod domain_host;
mod encryption;
mod forwarding;
mod grid;
mod home_layout;
mod home_shortcuts;
mod keybindings;
mod lifecycle;
#[cfg(feature = "shell")]
mod local_terminal;
mod navigation;
mod recent;
mod render;
mod sidebar;
mod sidebar_navigation;
mod sync_route;
mod team_permissions;
mod toolbar;
mod workbench;
mod workspace;
mod workspace_filter;

pub(crate) use recent::recent_connections;

use connection_badge::ConnectionTeamBadge;
pub(crate) use connection_badge::connection_team_badge;
use connection_badge::render_team_badge;
pub(crate) use connection_filter::connection_matches_query;
#[cfg(test)]
pub(crate) use connection_grouping::can_manage_connection_with_permissions;
use connection_info::{
    card_connection_info, connection_display_name, generate_duplicate_name,
    port_forwarding_connection_info,
};
#[cfg(feature = "shell")]
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
use sync_route::sync_route_for_provider;
#[cfg(test)]
mod tests;
