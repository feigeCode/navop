//! Redis 连接表单窗口（多标签页）

use connection_form::team::{
    TeamSelectItem, create_team_select, refresh_team_options, refresh_teams_tooltip,
    resolve_team_assignment, selected_team_id, team_label,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    radio::Radio,
    select::{Select, SelectItem, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionType, RedisClusterConfig, RedisMode, RedisParams, RedisSentinelConfig,
    RedisSshTunnelConfig, StoredConnection, Workspace,
};
use rust_i18n::t;
use tracing::error;

use crate::{RedisConnectionConfig, RedisConnectionMode, RedisManager};

/// Redis 表单窗口配置
pub struct RedisFormWindowConfig {
    pub editing_connection: Option<StoredConnection>,
    pub initial_connection: Option<StoredConnection>,
    pub on_saved: Option<RedisFormSavedCallback>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
}

pub type RedisFormSavedCallback =
    std::sync::Arc<dyn Fn(StoredConnection, &mut App) + Send + Sync + 'static>;

impl RedisFormWindowConfig {
    fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    fn connection_to_load(&self) -> Option<&StoredConnection> {
        self.editing_connection
            .as_ref()
            .or(self.initial_connection.as_ref())
    }
}

#[derive(Clone, Default, PartialEq)]
struct WorkspaceSelectItem {
    id: Option<i64>,
    name: String,
}

impl WorkspaceSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("Common.none").to_string(),
        }
    }

    fn from_workspace(ws: &Workspace) -> Self {
        Self {
            id: ws.id,
            name: ws.name.clone(),
        }
    }
}

impl SelectItem for WorkspaceSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone, Default, PartialEq)]
struct SshConnectionSelectItem {
    id: Option<i64>,
    name: String,
}

impl SshConnectionSelectItem {
    fn none() -> Self {
        Self {
            id: None,
            name: t!("ConnectionForm.ssh_connection_manual").to_string(),
        }
    }

    fn from_connection(connection: &StoredConnection) -> Self {
        let host = connection.to_ssh_params().ok().map(|params| params.host);
        let name = match host.as_deref().filter(|host| !host.trim().is_empty()) {
            Some(host) => format!("{} ({})", connection.name, host),
            None => connection.name.clone(),
        };

        Self {
            id: connection.id,
            name,
        }
    }
}

impl SelectItem for SshConnectionSelectItem {
    type Value = Option<i64>;

    fn title(&self) -> SharedString {
        self.name.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

/// 连接模式选择
#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ModeSelection {
    #[default]
    Standalone,
    Sentinel,
    Cluster,
}

impl ModeSelection {
    fn to_redis_mode(&self) -> RedisMode {
        match self {
            ModeSelection::Standalone => RedisMode::Standalone,
            ModeSelection::Sentinel => RedisMode::Sentinel,
            ModeSelection::Cluster => RedisMode::Cluster,
        }
    }

    fn from_redis_mode(mode: &RedisMode) -> Self {
        match mode {
            RedisMode::Standalone => ModeSelection::Standalone,
            RedisMode::Sentinel => ModeSelection::Sentinel,
            RedisMode::Cluster => ModeSelection::Cluster,
        }
    }
}

/// Redis 连接表单窗口
pub struct RedisFormWindow {
    focus_handle: FocusHandle,
    is_editing: bool,
    editing_id: Option<i64>,
    editing_cloud_id: Option<String>,
    editing_last_synced_at: Option<i64>,
    editing_owner_id: Option<String>,

    // 当前活动标签页索引
    active_tab: usize,

    // 基本信息
    name_input: Entity<InputState>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    db_index_input: Entity<InputState>,

    // 工作区选择
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,

    // 连接模式
    mode: ModeSelection,

    // 哨兵配置
    sentinel_master_name_input: Entity<InputState>,
    sentinel_nodes_input: Entity<InputState>,
    sentinel_password_input: Entity<InputState>,

    // 集群配置
    cluster_nodes_input: Entity<InputState>,

    // 高级设置
    use_tls: bool,
    connect_timeout_input: Entity<InputState>,
    ssh_connections: Vec<StoredConnection>,
    ssh_connection_select: Entity<SelectState<Vec<SshConnectionSelectItem>>>,
    ssh_tunnel_enabled: bool,
    ssh_host_input: Entity<InputState>,
    ssh_port_input: Entity<InputState>,
    ssh_username_input: Entity<InputState>,
    ssh_auth_type: String,
    ssh_password_input: Entity<InputState>,
    ssh_private_key_path_input: Entity<InputState>,
    ssh_private_key_content_input: Entity<InputState>,
    ssh_private_key_passphrase_input: Entity<InputState>,
    ssh_target_host_input: Entity<InputState>,
    ssh_target_port_input: Entity<InputState>,
    ssh_timeout_input: Entity<InputState>,

    // 备注
    remark_input: Entity<InputState>,

    // 云同步开关
    sync_enabled: bool,

    // 测试状态
    is_testing: bool,
    test_result: Option<Result<(), String>>,
    on_saved: Option<RedisFormSavedCallback>,
}

impl RedisFormWindow {
    pub fn new(config: RedisFormWindowConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let is_editing = config.is_editing();
        let connection_to_load = config.connection_to_load().cloned();
        let editing_id = config.editing_connection.as_ref().and_then(|c| c.id);
        let editing_cloud_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.cloud_id.clone());
        let editing_last_synced_at = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.last_synced_at);
        let editing_owner_id = config
            .editing_connection
            .as_ref()
            .and_then(|c| c.owner_id.clone());

        // 解析现有连接参数
        let existing_params = connection_to_load
            .as_ref()
            .and_then(|c| c.to_redis_params().ok());

        // 基本信息输入框
        let name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(t!("Redis.name_placeholder"));
            if let Some(ref c) = connection_to_load {
                state.set_value(c.name.clone(), window, cx);
            }
            state
        });

        let host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder(t!("Redis.host_placeholder"));
            if let Some(ref p) = existing_params {
                state.set_value(p.host.clone(), window, cx);
            } else {
                state.set_value("127.0.0.1".to_string(), window, cx);
            }
            state
        });

        let port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("6379");
            if let Some(ref p) = existing_params {
                state.set_value(p.port.to_string(), window, cx);
            } else {
                state.set_value("6379".to_string(), window, cx);
            }
            state
        });

        let username_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("Redis.username_placeholder"));
            if let Some(ref p) = existing_params {
                if let Some(ref user) = p.username {
                    state.set_value(user.clone(), window, cx);
                }
            }
            state
        });

        let password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.password_placeholder"))
                .masked(true);
            if let Some(ref p) = existing_params {
                if let Some(ref pwd) = p.password {
                    state.set_value(pwd.clone(), window, cx);
                }
            }
            state
        });

        let db_index_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("Redis.db_index_placeholder"));
            if let Some(ref p) = existing_params {
                state.set_value(p.db_index.to_string(), window, cx);
            } else {
                state.set_value("0".to_string(), window, cx);
            }
            state
        });

        // 哨兵配置
        let sentinel_master_name_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.sentinel_master_name_placeholder"));
            if let Some(ref p) = existing_params {
                if let Some(ref sentinel) = p.sentinel {
                    state.set_value(sentinel.master_name.clone(), window, cx);
                }
            }
            state
        });

        let sentinel_nodes_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.sentinel_nodes_placeholder"))
                .auto_grow(3, 6);
            if let Some(ref p) = existing_params {
                if let Some(ref sentinel) = p.sentinel {
                    state.set_value(sentinel.sentinels.join("\n"), window, cx);
                }
            }
            state
        });

        let sentinel_password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.sentinel_password_placeholder"))
                .masked(true);
            if let Some(ref p) = existing_params {
                if let Some(ref sentinel) = p.sentinel {
                    if let Some(ref pwd) = sentinel.sentinel_password {
                        state.set_value(pwd.clone(), window, cx);
                    }
                }
            }
            state
        });

        // 集群配置
        let cluster_nodes_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.cluster_nodes_placeholder"))
                .auto_grow(3, 6);
            if let Some(ref p) = existing_params {
                if let Some(ref cluster) = p.cluster {
                    state.set_value(cluster.nodes.join("\n"), window, cx);
                }
            }
            state
        });

        // 高级设置
        let connect_timeout_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("10");
            if let Some(ref p) = existing_params {
                if let Some(timeout) = p.connect_timeout {
                    state.set_value(timeout.to_string(), window, cx);
                } else {
                    state.set_value("10".to_string(), window, cx);
                }
            } else {
                state.set_value("10".to_string(), window, cx);
            }
            state
        });

        let existing_ssh = existing_params.as_ref().and_then(|p| p.ssh_tunnel.as_ref());
        let mut ssh_items = vec![SshConnectionSelectItem::none()];
        let ssh_connections: Vec<StoredConnection> = config
            .ssh_connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .collect();
        ssh_items.extend(
            ssh_connections
                .iter()
                .map(SshConnectionSelectItem::from_connection),
        );
        let selected_ssh_connection_id = existing_ssh.and_then(|ssh| ssh.connection_id);
        let ssh_connection_select = cx.new(|cx| {
            let mut state = SelectState::new(ssh_items, Some(Default::default()), window, cx);
            if let Some(id) = selected_ssh_connection_id {
                state.set_selected_value(&Some(id), window, cx);
            }
            state
        });

        let ssh_host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("jump.example.com");
            if let Some(ssh) = existing_ssh {
                state.set_value(ssh.host.clone(), window, cx);
            }
            state
        });

        let ssh_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("22");
            state.set_value(
                existing_ssh.map(|ssh| ssh.port).unwrap_or(22).to_string(),
                window,
                cx,
            );
            state
        });

        let ssh_username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("root");
            if let Some(ssh) = existing_ssh {
                state.set_value(ssh.username.clone(), window, cx);
            }
            state
        });

        let ssh_password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.ssh_password"))
                .masked(true);
            if let Some(password) = existing_ssh.and_then(|ssh| ssh.password.as_ref()) {
                state.set_value(password.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_path_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("~/.ssh/id_rsa");
            if let Some(path) = existing_ssh.and_then(|ssh| ssh.private_key_path.as_ref()) {
                state.set_value(path.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_content_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.ssh_private_key_content_placeholder"))
                .auto_grow(5, 14);
            if let Some(private_key) = existing_ssh.and_then(|ssh| ssh.private_key_content.as_ref())
            {
                state.set_value(private_key.clone(), window, cx);
            }
            state
        });

        let ssh_private_key_passphrase_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("ConnectionForm.ssh_private_key_passphrase"))
                .masked(true);
            if let Some(passphrase) =
                existing_ssh.and_then(|ssh| ssh.private_key_passphrase.as_ref())
            {
                state.set_value(passphrase.clone(), window, cx);
            }
            state
        });

        let ssh_target_host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("127.0.0.1");
            if let Some(host) = existing_ssh.and_then(|ssh| ssh.target_host.as_ref()) {
                state.set_value(host.clone(), window, cx);
            }
            state
        });

        let ssh_target_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("6379");
            if let Some(port) = existing_ssh.and_then(|ssh| ssh.target_port) {
                state.set_value(port.to_string(), window, cx);
            }
            state
        });

        let ssh_timeout_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("30");
            if let Some(timeout) = existing_ssh.and_then(|ssh| ssh.timeout) {
                state.set_value(timeout.to_string(), window, cx);
            }
            state
        });

        let remark_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Redis.remark_placeholder"))
                .auto_grow(3, 10);
            if let Some(ref c) = connection_to_load {
                if let Some(ref remark) = c.remark {
                    state.set_value(remark.clone(), window, cx);
                }
            }
            state
        });

        // 工作区选择
        let mut workspace_items: Vec<WorkspaceSelectItem> = vec![WorkspaceSelectItem::none()];
        workspace_items.extend(
            config
                .workspaces
                .iter()
                .map(WorkspaceSelectItem::from_workspace),
        );

        let selected_workspace_id = connection_to_load.as_ref().and_then(|c| c.workspace_id);

        let workspace_select = cx.new(|cx| {
            let mut state = SelectState::new(workspace_items, None, window, cx);
            if let Some(ws_id) = selected_workspace_id {
                state.set_selected_value(&Some(ws_id), window, cx);
            }
            state
        });

        let selected_team_id = connection_to_load.as_ref().and_then(|c| c.team_id.clone());
        let team_select =
            create_team_select(&config.teams, selected_team_id.as_deref(), window, cx);

        // 加载模式和高级设置
        let mut mode = ModeSelection::Standalone;
        let mut use_tls = false;
        let mut sync_enabled = true;

        if let Some(ref p) = existing_params {
            mode = ModeSelection::from_redis_mode(&p.mode);
            use_tls = p.use_tls;
        }

        if let Some(ref c) = connection_to_load {
            sync_enabled = c.sync_enabled;
        }

        Self {
            focus_handle: cx.focus_handle(),
            is_editing,
            editing_id,
            editing_cloud_id,
            editing_last_synced_at,
            editing_owner_id,
            active_tab: 0,
            name_input,
            host_input,
            port_input,
            username_input,
            password_input,
            db_index_input,
            workspace_select,
            team_select,
            mode,
            sentinel_master_name_input,
            sentinel_nodes_input,
            sentinel_password_input,
            cluster_nodes_input,
            use_tls,
            connect_timeout_input,
            ssh_connections,
            ssh_connection_select,
            ssh_tunnel_enabled: existing_ssh.is_some_and(|ssh| ssh.enabled),
            ssh_host_input,
            ssh_port_input,
            ssh_username_input,
            ssh_auth_type: existing_ssh
                .map(|ssh| ssh.auth_type.clone())
                .unwrap_or_else(|| "password".to_string()),
            ssh_password_input,
            ssh_private_key_path_input,
            ssh_private_key_content_input,
            ssh_private_key_passphrase_input,
            ssh_target_host_input,
            ssh_target_port_input,
            ssh_timeout_input,
            remark_input,
            sync_enabled,
            is_testing: false,
            test_result: None,
            on_saved: config.on_saved,
        }
    }

    /// 获取工作区 ID
    fn get_workspace_id(&self, cx: &App) -> Option<i64> {
        self.workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    /// 获取团队 ID
    fn get_team_id(&self, cx: &App) -> Option<String> {
        selected_team_id(&self.team_select, cx)
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    fn selected_ssh_connection(&self, cx: &App) -> Option<&StoredConnection> {
        let selected_id = self
            .ssh_connection_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()?;
        self.ssh_connections
            .iter()
            .find(|connection| connection.id == Some(selected_id))
    }

    fn optional_input_value(input: &Entity<InputState>, cx: &App) -> Option<String> {
        let value = input.read(cx).text().to_string().trim().to_string();
        if value.is_empty() { None } else { Some(value) }
    }

    fn optional_input_u16(input: &Entity<InputState>, cx: &App) -> Option<u16> {
        input.read(cx).text().to_string().trim().parse::<u16>().ok()
    }

    fn optional_input_u64(input: &Entity<InputState>, cx: &App) -> Option<u64> {
        input.read(cx).text().to_string().trim().parse::<u64>().ok()
    }

    fn build_ssh_tunnel_config(&self, cx: &App) -> Option<RedisSshTunnelConfig> {
        if !self.ssh_tunnel_enabled {
            return None;
        }

        let connection_id = self
            .ssh_connection_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten();

        Some(RedisSshTunnelConfig {
            enabled: true,
            connection_id,
            host: self
                .ssh_host_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            port: Self::optional_input_u16(&self.ssh_port_input, cx).unwrap_or(22),
            username: self
                .ssh_username_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            auth_type: self.ssh_auth_type.clone(),
            password: Self::optional_input_value(&self.ssh_password_input, cx),
            private_key_path: Self::optional_input_value(&self.ssh_private_key_path_input, cx),
            private_key_content: Self::optional_input_value(
                &self.ssh_private_key_content_input,
                cx,
            ),
            private_key_passphrase: Self::optional_input_value(
                &self.ssh_private_key_passphrase_input,
                cx,
            ),
            target_host: Self::optional_input_value(&self.ssh_target_host_input, cx),
            target_port: Self::optional_input_u16(&self.ssh_target_port_input, cx),
            timeout: Self::optional_input_u64(&self.ssh_timeout_input, cx),
        })
    }

    /// 构建 RedisParams
    fn build_redis_params(&self, cx: &App) -> RedisParams {
        let host = self.host_input.read(cx).text().to_string();
        let port: u16 = self
            .port_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .unwrap_or(6379);
        let password = {
            let pwd = self.password_input.read(cx).text().to_string();
            if pwd.is_empty() { None } else { Some(pwd) }
        };
        let username = {
            let user = self.username_input.read(cx).text().to_string();
            if user.is_empty() { None } else { Some(user) }
        };
        let db_index: u8 = self
            .db_index_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .unwrap_or(0);
        let connect_timeout: Option<u64> = self
            .connect_timeout_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .ok();

        // 哨兵配置
        let sentinel = if self.mode == ModeSelection::Sentinel {
            let master_name = self.sentinel_master_name_input.read(cx).text().to_string();
            let nodes_text = self.sentinel_nodes_input.read(cx).text().to_string();
            let sentinels: Vec<String> = nodes_text
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let sentinel_password = {
                let pwd = self.sentinel_password_input.read(cx).text().to_string();
                if pwd.is_empty() { None } else { Some(pwd) }
            };

            if !master_name.is_empty() && !sentinels.is_empty() {
                Some(RedisSentinelConfig {
                    master_name,
                    sentinels,
                    sentinel_password,
                })
            } else {
                None
            }
        } else {
            None
        };

        // 集群配置
        let cluster = if self.mode == ModeSelection::Cluster {
            let nodes_text = self.cluster_nodes_input.read(cx).text().to_string();
            let nodes: Vec<String> = nodes_text
                .lines()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();

            if !nodes.is_empty() {
                Some(RedisClusterConfig { nodes })
            } else {
                None
            }
        } else {
            None
        };

        let mut params = RedisParams {
            host,
            port,
            password,
            username,
            db_index,
            mode: self.mode.to_redis_mode(),
            use_tls: self.use_tls,
            connect_timeout,
            sentinel,
            cluster,
            ssh_tunnel: self.build_ssh_tunnel_config(cx),
        };

        if let Some(ssh_connection) = self.selected_ssh_connection(cx) {
            if let Err(error) = params.apply_referenced_ssh_tunnel(ssh_connection) {
                tracing::warn!("Failed to apply Redis SSH tunnel reference: {}", error);
            }
        }

        params
    }

    /// 获取 Redis 连接配置（用于测试连接）
    fn get_config(&self, cx: &App) -> RedisConnectionConfig {
        let params = self.build_redis_params(cx);
        let name = self.name_input.read(cx).text().to_string();

        RedisConnectionConfig {
            id: self.editing_id.map(|id| id.to_string()).unwrap_or_default(),
            name,
            host: params.host,
            port: params.port,
            password: params.password,
            username: params.username,
            db_index: params.db_index,
            use_tls: params.use_tls,
            timeout: params.connect_timeout.unwrap_or(10),
            mode: match self.mode {
                ModeSelection::Standalone => RedisConnectionMode::Standalone,
                ModeSelection::Sentinel => RedisConnectionMode::Sentinel,
                ModeSelection::Cluster => RedisConnectionMode::Cluster,
            },
            ssh_tunnel: params.ssh_tunnel,
        }
    }

    /// 测试连接
    fn on_test(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.is_testing = true;
        self.test_result = None;
        cx.notify();

        let config = self.get_config(cx);

        cx.spawn(async move |this, cx| {
            let test_result: Result<(), String> = Tokio::spawn_result(cx, async move {
                RedisManager::test_connection(&config)
                    .await
                    .map_err(anyhow::Error::new)
            })
            .await
            .map_err(|e| {
                let detailed = format!("{:#}", e);
                error!("Redis 连接测试失败: {}", detailed);
                detailed
            });

            let _ = this.update(cx, |this, cx| {
                this.is_testing = false;
                this.test_result = Some(test_result);
                cx.notify();
            });
        })
        .detach();
    }

    /// 保存连接
    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let params = self.build_redis_params(cx);
        let name = self.name_input.read(cx).text().to_string();
        let name = if name.is_empty() {
            format!("{}:{}", params.host, params.port)
        } else {
            name
        };

        let workspace_id = self.get_workspace_id(cx);
        let team_id = self.get_team_id(cx);
        let assignment = match resolve_team_assignment(
            team_id,
            self.is_editing,
            self.editing_owner_id.clone(),
            cx,
        ) {
            Ok(assignment) => assignment,
            Err(error) => {
                self.test_result = Some(Err(error.to_string()));
                cx.notify();
                return;
            }
        };
        let remark = {
            let r = self.remark_input.read(cx).text().to_string();
            if r.is_empty() { None } else { Some(r) }
        };
        let sync_enabled = self.sync_enabled;
        let is_editing = self.is_editing;
        let editing_id = self.editing_id;
        let editing_cloud_id = self.editing_cloud_id.clone();
        let editing_last_synced_at = self.editing_last_synced_at;
        let on_saved = self.on_saved.clone();

        let storage = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .clone();

        cx.spawn(async move |_this, cx| {
            let result = Tokio::spawn_result(cx, async move {
                let repo = storage
                    .get::<one_core::storage::ConnectionRepository>()
                    .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

                let mut conn = StoredConnection::new_redis(name, params, workspace_id);
                conn.sync_enabled = sync_enabled;
                conn.remark = remark;
                conn.team_id = assignment.team_id;
                conn.owner_id = assignment.owner_id;

                if is_editing {
                    conn.id = editing_id;
                    conn.cloud_id = editing_cloud_id;
                    conn.last_synced_at = editing_last_synced_at;
                    repo.update(&mut conn)?;
                } else {
                    repo.insert(&mut conn)?;
                }
                Ok::<StoredConnection, anyhow::Error>(conn)
            })
            .await;

            match result {
                Ok(saved_conn) => {
                    let _ = cx.update(|cx| {
                        if let Some(notifier) = get_notifier(cx) {
                            let event = if is_editing {
                                ConnectionDataEvent::ConnectionUpdated {
                                    connection: saved_conn.clone(),
                                }
                            } else {
                                ConnectionDataEvent::ConnectionCreated {
                                    connection: saved_conn.clone(),
                                }
                            };
                            notifier.update(cx, |_, cx| {
                                cx.emit(event);
                            });
                        }
                        if let Some(on_saved) = &on_saved {
                            on_saved(saved_conn, cx);
                        }
                    });
                }
                Err(e) => {
                    error!(
                        "{}",
                        t!("Redis.save_connection_failed", error = e).to_string()
                    );
                }
            }
        })
        .detach();

        window.remove_window();
    }

    fn render_form_row(&self, label: &str, child: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .w(px(100.0))
                    .text_sm()
                    .text_right()
                    .child(label.to_string()),
            )
            .child(div().flex_1().child(child))
    }

    /// 渲染基本信息标签页
    fn render_basic_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(&t!("Redis.name"), Input::new(&self.name_input)))
            .child(self.render_form_row(&t!("Redis.host"), Input::new(&self.host_input)))
            .child(self.render_form_row(&t!("Redis.port"), Input::new(&self.port_input)))
            .child(self.render_form_row(&t!("Redis.username"), Input::new(&self.username_input)))
            .child(self.render_form_row(
                &t!("Redis.password"),
                Input::new(&self.password_input).mask_toggle(),
            ))
            .child(self.render_form_row(&t!("Redis.db_index"), Input::new(&self.db_index_input)))
            .child(self.render_form_row(
                &t!("Redis.workspace"),
                Select::new(&self.workspace_select).w_full(),
            ))
            .child(
                self.render_form_row(
                    &team_label(),
                    h_flex()
                        .gap_2()
                        .child(Select::new(&self.team_select).w_full())
                        .child(
                            Button::new("sync-redis-teams")
                                .icon(IconName::Refresh)
                                .ghost()
                                .tooltip(refresh_teams_tooltip())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_team_sync(window, cx);
                                })),
                        ),
                ),
            )
            .child(
                self.render_form_row(
                    &t!("ConnectionForm.cloud_sync"),
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new("sync-enabled")
                                .checked(self.sync_enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sync_enabled = !this.sync_enabled;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("ConnectionForm.cloud_sync_desc").to_string()),
                        ),
                ),
            )
    }

    /// 渲染连接模式标签页
    fn render_mode_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let mode = self.mode;

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("Redis.mode"),
                    h_flex()
                        .gap_4()
                        .child(
                            Radio::new("standalone")
                                .label(t!("Redis.mode_standalone").to_string())
                                .checked(mode == ModeSelection::Standalone)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mode = ModeSelection::Standalone;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("sentinel")
                                .label(t!("Redis.mode_sentinel").to_string())
                                .checked(mode == ModeSelection::Sentinel)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mode = ModeSelection::Sentinel;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("cluster")
                                .label(t!("Redis.mode_cluster").to_string())
                                .checked(mode == ModeSelection::Cluster)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.mode = ModeSelection::Cluster;
                                    cx.notify();
                                })),
                        ),
                ),
            )
            // 哨兵模式配置
            .when(mode == ModeSelection::Sentinel, |this| {
                this.child(self.render_form_row(
                    &t!("Redis.sentinel_master_name"),
                    Input::new(&self.sentinel_master_name_input),
                ))
                .child(self.render_form_row(
                    &t!("Redis.sentinel_nodes"),
                    Input::new(&self.sentinel_nodes_input),
                ))
                .child(self.render_form_row(
                    &t!("Redis.sentinel_password"),
                    Input::new(&self.sentinel_password_input).mask_toggle(),
                ))
            })
            // 集群模式配置
            .when(mode == ModeSelection::Cluster, |this| {
                this.child(self.render_form_row(
                    &t!("Redis.cluster_nodes"),
                    Input::new(&self.cluster_nodes_input),
                ))
            })
    }

    /// 渲染高级设置标签页
    fn render_advanced_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("Redis.use_tls"),
                    Checkbox::new("use-tls")
                        .checked(self.use_tls)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.use_tls = !this.use_tls;
                            cx.notify();
                        })),
                ),
            )
            .child(self.render_form_row(
                &t!("Redis.connect_timeout"),
                Input::new(&self.connect_timeout_input),
            ))
            .child(self.render_ssh_tunnel_settings(cx))
    }

    fn render_ssh_tunnel_settings(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let using_ssh_reference = self
            .ssh_connection_select
            .read(cx)
            .selected_value()
            .is_some_and(|value| value.is_some());
        let auth_type = match self.ssh_auth_type.as_str() {
            "private_key_material" => "private_key_content",
            value => value,
        };

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("ConnectionForm.ssh_tunnel_enabled"),
                    Checkbox::new("redis-ssh-tunnel-enabled")
                        .checked(self.ssh_tunnel_enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ssh_tunnel_enabled = !this.ssh_tunnel_enabled;
                            cx.notify();
                        })),
                ),
            )
            .when(self.ssh_tunnel_enabled, |this| {
                this.child(self.render_form_row(
                    &t!("ConnectionForm.ssh_connection_id"),
                    Select::new(&self.ssh_connection_select).w_full(),
                ))
                .when(!using_ssh_reference, |this| {
                    this.child(self.render_form_row(
                        &t!("ConnectionForm.ssh_host"),
                        Input::new(&self.ssh_host_input),
                    ))
                    .child(self.render_form_row(
                        &t!("ConnectionForm.ssh_port"),
                        Input::new(&self.ssh_port_input),
                    ))
                    .child(self.render_form_row(
                        &t!("ConnectionForm.ssh_username"),
                        Input::new(&self.ssh_username_input),
                    ))
                    .child(
                        self.render_form_row(
                            &t!("ConnectionForm.ssh_auth_type"),
                            h_flex()
                                .flex_wrap()
                                .gap_4()
                                .child(
                                    Radio::new("redis-ssh-auth-password")
                                        .label(t!("ConnectionForm.ssh_auth_password").to_string())
                                        .checked(auth_type == "password")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "password".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("redis-ssh-auth-private-key")
                                        .label(
                                            t!("ConnectionForm.ssh_auth_private_key").to_string(),
                                        )
                                        .checked(auth_type == "private_key")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "private_key".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("redis-ssh-auth-private-key-content")
                                        .label(
                                            t!("ConnectionForm.ssh_auth_private_key_content")
                                                .to_string(),
                                        )
                                        .checked(auth_type == "private_key_content")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "private_key_content".to_string();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Radio::new("redis-ssh-auth-agent")
                                        .label(t!("ConnectionForm.ssh_auth_agent").to_string())
                                        .checked(auth_type == "agent")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.ssh_auth_type = "agent".to_string();
                                            cx.notify();
                                        })),
                                ),
                        ),
                    )
                    .when(auth_type == "password", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_password"),
                            Input::new(&self.ssh_password_input).mask_toggle(),
                        ))
                    })
                    .when(auth_type == "private_key", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_path"),
                            Input::new(&self.ssh_private_key_path_input),
                        ))
                        .child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_passphrase"),
                            Input::new(&self.ssh_private_key_passphrase_input).mask_toggle(),
                        ))
                    })
                    .when(auth_type == "private_key_content", |this| {
                        this.child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_content"),
                            Input::new(&self.ssh_private_key_content_input),
                        ))
                        .child(self.render_form_row(
                            &t!("ConnectionForm.ssh_private_key_passphrase"),
                            Input::new(&self.ssh_private_key_passphrase_input).mask_toggle(),
                        ))
                    })
                })
                .child(self.render_form_row(
                    &t!("ConnectionForm.ssh_target_host"),
                    Input::new(&self.ssh_target_host_input),
                ))
                .child(self.render_form_row(
                    &t!("ConnectionForm.ssh_target_port"),
                    Input::new(&self.ssh_target_port_input),
                ))
                .child(self.render_form_row("SSH Timeout", Input::new(&self.ssh_timeout_input)))
            })
    }

    /// 渲染其他设置标签页
    fn render_other_tab(&self) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(&t!("Redis.remark"), Input::new(&self.remark_input)))
    }
}

impl Focusable for RedisFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RedisFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.is_testing;
        let active_tab = self.active_tab;

        let test_result_element = match &self.test_result {
            Some(Ok(())) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("Redis.test_success").to_string()),
            ),
            Some(Err(e)) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(e.clone()),
            ),
            None => None,
        };

        v_flex()
            .justify_center()
            .size_full()
            // TabBar
            .child(
                div().flex().justify_center().px_3().pt_2().child(
                    TabBar::new("redis-form-tabs")
                        .with_size(Size::Small)
                        .underline()
                        .selected_index(active_tab)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.active_tab = *ix;
                            cx.notify();
                        }))
                        .child(Tab::new().label(t!("Redis.tab_basic").to_string()))
                        .child(Tab::new().label(t!("Redis.tab_mode").to_string()))
                        .child(Tab::new().label(t!("Redis.tab_advanced").to_string()))
                        .child(Tab::new().label(t!("Redis.tab_other").to_string())),
                ),
            )
            // 标签页内容
            .child(
                div()
                    .id("redis-form-content")
                    .flex_1()
                    .p_3()
                    .overflow_y_scroll()
                    .child(match active_tab {
                        0 => self.render_basic_tab(cx).into_any_element(),
                        1 => self.render_mode_tab(cx).into_any_element(),
                        2 => self.render_advanced_tab(cx).into_any_element(),
                        3 => self.render_other_tab().into_any_element(),
                        _ => div().into_any_element(),
                    }),
            )
            // 测试结果
            .when_some(test_result_element, |this, elem| {
                this.child(h_flex().justify_center().pb_2().child(elem))
            })
            // 底部按钮
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .py_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(|_, window, _cx| {
                                window.remove_window();
                            }),
                    )
                    .child(
                        Button::new("test")
                            .small()
                            .outline()
                            .label(if is_testing {
                                t!("Connection.testing").to_string()
                            } else {
                                t!("Connection.test").to_string()
                            })
                            .disabled(is_testing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn redis_connection(name: &str) -> StoredConnection {
        StoredConnection::new_redis(
            name.to_string(),
            RedisParams {
                host: "127.0.0.1".to_string(),
                port: 6379,
                password: None,
                username: None,
                db_index: 0,
                mode: RedisMode::Standalone,
                use_tls: false,
                connect_timeout: None,
                sentinel: None,
                cluster: None,
                ssh_tunnel: None,
            },
            None,
        )
    }

    #[test]
    fn initial_connection_prefills_without_edit_mode() {
        let connection = redis_connection("imported redis");
        let config = RedisFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(connection),
            on_saved: None,
            workspaces: Vec::new(),
            teams: Vec::new(),
            ssh_connections: Vec::new(),
        };

        assert!(!config.is_editing());
        assert_eq!(
            Some("imported redis"),
            config
                .connection_to_load()
                .map(|connection| connection.name.as_str())
        );
    }

    #[test]
    fn editing_connection_takes_precedence_over_initial_connection() {
        let config = RedisFormWindowConfig {
            editing_connection: Some(redis_connection("existing redis")),
            initial_connection: Some(redis_connection("imported redis")),
            on_saved: None,
            workspaces: Vec::new(),
            teams: Vec::new(),
            ssh_connections: Vec::new(),
        };

        assert!(config.is_editing());
        assert_eq!(
            Some("existing redis"),
            config
                .connection_to_load()
                .map(|connection| connection.name.as_str())
        );
    }
}
