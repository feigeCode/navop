use connection_form::team::{
    TeamSelectItem, create_team_select, refresh_team_options, refresh_teams_tooltip,
    resolve_team_assignment, selected_team_id, team_label, team_management_enabled,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, InteractiveElement,
    IntoElement, ParentElement, Render, SharedString, StatefulInteractiveElement, Styled,
    WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputState},
    radio::Radio,
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    tab::{Tab, TabBar},
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, get_notifier};
use one_core::gpui_tokio::Tokio;
use one_core::storage::traits::Repository;
use one_core::storage::{
    JumpServerConfig, ProxyConfig, ProxyType as StorageProxyType, SSH_ICON_IDS, SshAuthMethod,
    SshParams, StoredConnection, Workspace, ssh_os_icon,
};
use rust_i18n::t;
use ssh::{
    ChannelEvent, JumpServerConnectConfig, ProxyConnectConfig, ProxyType, RusshClient, SshAuth,
    SshChannel, SshClient, SshConnectConfig, SshSessionManager,
};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;
use terminal::SshBackend;

use crate::ssh_form_mfa::{
    CapturedMfaRequest, FormMfaPrompt, FormMfaRequest, JumpServerMfaResponder,
    form_mfa_request_from_keyboard_interactive, is_jump_mfa_required_error,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SshFormPostSaveAction {
    Close,
    Continue,
}

pub type SshFormSavedCallback = Arc<
    dyn Fn(StoredConnection, SshFormPostSaveAction, &mut Window, &mut App) + Send + Sync + 'static,
>;

pub struct SshFormWindowConfig {
    pub editing_connection: Option<StoredConnection>,
    pub initial_connection: Option<StoredConnection>,
    pub on_saved: Option<SshFormSavedCallback>,
    pub workspaces: Vec<Workspace>,
    pub teams: Vec<TeamOption>,
}

impl SshFormWindowConfig {
    pub fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    pub fn supports_save_and_continue(&self) -> bool {
        self.on_saved.is_some()
    }

    fn connection_to_load(&self) -> Option<&StoredConnection> {
        self.editing_connection
            .as_ref()
            .or(self.initial_connection.as_ref())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SaveAction {
    Close,
    Continue,
}

fn post_save_action(action: SaveAction) -> SshFormPostSaveAction {
    match action {
        SaveAction::Close => SshFormPostSaveAction::Close,
        SaveAction::Continue => SshFormPostSaveAction::Continue,
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

pub struct SshFormWindow {
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
    key_path_input: Entity<InputState>,
    private_key_content_input: Entity<InputState>,
    passphrase_input: Entity<InputState>,

    auth_method: AuthMethodSelection,
    workspace_select: Entity<SelectState<Vec<WorkspaceSelectItem>>>,
    team_select: Entity<SelectState<Vec<TeamSelectItem>>>,

    // 跳板机设置
    enable_jump_server: bool,
    jump_auth_method: AuthMethodSelection,
    jump_host_input: Entity<InputState>,
    jump_port_input: Entity<InputState>,
    jump_username_input: Entity<InputState>,
    jump_password_input: Entity<InputState>,
    jump_key_path_input: Entity<InputState>,
    jump_private_key_content_input: Entity<InputState>,
    jump_passphrase_input: Entity<InputState>,
    jump_mfa_request: Option<FormMfaRequest>,
    jump_mfa_inputs: Vec<JumpMfaInput>,
    jump_mfa_signature: Option<String>,

    // 代理设置
    enable_proxy: bool,
    proxy_type: ProxyTypeSelection,
    proxy_host_input: Entity<InputState>,
    proxy_port_input: Entity<InputState>,
    proxy_username_input: Entity<InputState>,
    proxy_password_input: Entity<InputState>,

    // 高级设置
    connect_timeout_input: Entity<InputState>,
    keepalive_interval_input: Entity<InputState>,
    keepalive_max_input: Entity<InputState>,

    // 初始化
    init_script_input: Entity<InputState>,
    default_directory_input: Entity<InputState>,

    // 其他设置
    remark_input: Entity<InputState>,

    last_tested_signature: Option<String>,
    /// 测试连接成功时探测到的远端操作系统 ID（/etc/os-release 的 ID 字段）
    detected_os_id: Option<String>,
    /// 手动指定的连接图标 ID（None = 自动跟随探测结果）
    manual_icon: Option<String>,

    // 云同步开关
    sync_enabled: bool,

    // 关闭 shell integration 注入(走裸 request_shell,失去 OSC 集成)
    disable_shell_integration: bool,

    is_testing: bool,
    is_uninstalling_shell_integration: bool,
    test_result: Option<Result<Option<String>, String>>,
    shell_integration_uninstall_result: Option<Result<(), String>>,
    on_saved: Option<SshFormSavedCallback>,
    save_action: SaveAction,
}

#[derive(Clone)]
struct JumpMfaInput {
    prompt: FormMfaPrompt,
    input: Entity<InputState>,
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum AuthMethodSelection {
    #[default]
    Password,
    PrivateKey,
    PrivateKeyContent,
    Agent,
    AutoPublicKey,
}

fn build_connection_test_signature(params: &SshParams) -> String {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash as _, Hasher as _};

    let mut hasher = DefaultHasher::new();
    format!("{:?}", params).hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

fn format_connection_error(error: &anyhow::Error, host: &str) -> String {
    format_connection_error_for_platform(error, host, cfg!(target_os = "macos"))
}

/// 测试连接成功后尽力探测远端操作系统 ID（/etc/os-release 的 ID 字段）。
/// 任何失败都返回 None，不影响连接测试结果。
async fn detect_remote_os_id(client: &mut RusshClient) -> Option<String> {
    const DETECT_COMMAND: &str =
        "cat /etc/os-release 2>/dev/null || cat /usr/lib/os-release 2>/dev/null || uname -s";

    let output = tokio::time::timeout(Duration::from_secs(5), async {
        let mut channel = client.open_channel().await.ok()?;
        channel.exec(DETECT_COMMAND).await.ok()?;

        let mut stdout = Vec::new();
        while let Some(event) = channel.recv().await {
            match event {
                ChannelEvent::Data(data) => stdout.extend(data),
                ChannelEvent::Eof | ChannelEvent::Close => break,
                _ => {}
            }
        }
        let _ = channel.close().await;
        String::from_utf8(stdout).ok()
    })
    .await
    .ok()??;

    parse_os_release_id(&output).or_else(|| match output.trim() {
        // 无 os-release 的系统回退到 uname（如 macOS）
        "Darwin" => Some("macos".to_string()),
        _ => None,
    })
}

/// 从 /etc/os-release 内容中解析 ID 字段（统一小写）。
fn parse_os_release_id(content: &str) -> Option<String> {
    content
        .lines()
        .find_map(|line| {
            line.strip_prefix("ID=").map(|value| {
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_lowercase()
            })
        })
        .filter(|id| !id.is_empty())
}

fn format_connection_error_for_platform(
    error: &anyhow::Error,
    host: &str,
    is_macos: bool,
) -> String {
    let detail = format!("{error:#}");
    if is_macos && is_private_network_host(host) && is_host_unreachable(error) {
        return format!("{detail}\n\n{}", t!("SSH.local_network_route_hint"));
    }
    detail
}

fn is_private_network_host(host: &str) -> bool {
    match host.trim().parse::<IpAddr>() {
        Ok(IpAddr::V4(address)) => address.is_private() || address.is_link_local(),
        Ok(IpAddr::V6(address)) => address.is_unique_local() || address.is_unicast_link_local(),
        Err(_) => false,
    }
}

fn is_host_unreachable(error: &anyhow::Error) -> bool {
    error.chain().any(|cause| {
        cause
            .downcast_ref::<std::io::Error>()
            .is_some_and(|error| error.kind() == std::io::ErrorKind::HostUnreachable)
    })
}

fn validate_save_state(
    is_testing: bool,
    is_uninstalling_shell_integration: bool,
) -> Result<(), &'static str> {
    if is_testing {
        Err("testing")
    } else if is_uninstalling_shell_integration {
        Err("uninstalling_shell_integration")
    } else {
        Ok(())
    }
}

fn save_block_message(reason: &str) -> String {
    match reason {
        "testing" => t!("SSH.save_while_testing").to_string(),
        "uninstalling_shell_integration" => {
            t!("SSH.save_while_uninstalling_shell_integration").to_string()
        }
        _ => t!("SSH.validation_error").to_string(),
    }
}

fn build_jump_auth_method(
    auth_method: AuthMethodSelection,
    password: String,
    key_path: String,
    private_key: String,
    passphrase: String,
) -> SshAuthMethod {
    match auth_method {
        AuthMethodSelection::Password => SshAuthMethod::Password { password },
        AuthMethodSelection::PrivateKey => SshAuthMethod::PrivateKey {
            key_path,
            passphrase: if passphrase.is_empty() {
                None
            } else {
                Some(passphrase)
            },
        },
        AuthMethodSelection::PrivateKeyContent => SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase: if passphrase.is_empty() {
                None
            } else {
                Some(passphrase)
            },
        },
        AuthMethodSelection::Agent => SshAuthMethod::Agent,
        AuthMethodSelection::AutoPublicKey => SshAuthMethod::AutoPublicKey,
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum ProxyTypeSelection {
    #[default]
    Socks5,
    Http,
}

impl SshFormWindow {
    pub fn new(config: SshFormWindowConfig, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let is_editing = config.is_editing();
        let on_saved = config.on_saved.clone();
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

        // 基本信息
        let name_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.name_placeholder")));
        let host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.host_placeholder")));
        let port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("22");
            state.set_value("22", window, cx);
            state
        });
        let username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.username_placeholder")));
        let password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.password_placeholder"))
                .masked(true)
        });
        let key_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.key_path_placeholder")));
        let private_key_content_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.private_key_content_placeholder"))
                .auto_grow(6, 12)
        });
        let passphrase_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.passphrase_placeholder"))
                .masked(true)
        });

        // 跳板机设置
        let jump_host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.host_placeholder")));
        let jump_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("22");
            state.set_value("22", window, cx);
            state
        });
        let jump_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.username_placeholder")));
        let jump_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.password_placeholder"))
                .masked(true)
        });
        let jump_key_path_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.key_path_placeholder")));
        let jump_private_key_content_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.private_key_content_placeholder"))
                .auto_grow(6, 12)
        });
        let jump_passphrase_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.passphrase_placeholder"))
                .masked(true)
        });

        // 代理设置
        let proxy_host_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.proxy_host")));
        let proxy_port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("1080");
            state.set_value("1080", window, cx);
            state
        });
        let proxy_username_input =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("SSH.proxy_username")));
        let proxy_password_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.proxy_password"))
                .masked(true)
        });

        // 高级设置
        let connect_timeout_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("30");
            state.set_value("30", window, cx);
            state
        });
        let keepalive_interval_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("20");
            state.set_value("20", window, cx);
            state
        });
        let keepalive_max_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("3");
            state.set_value("3", window, cx);
            state
        });

        // 初始化
        let init_script_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.init_script_placeholder"))
                .auto_grow(3, 8)
        });
        let default_directory_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("SSH.default_directory_placeholder"))
        });

        // 其他设置
        let remark_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("SSH.remark_placeholder"))
                .auto_grow(3, 10)
        });

        let mut workspace_items = vec![WorkspaceSelectItem::none()];
        workspace_items.extend(
            config
                .workspaces
                .iter()
                .map(WorkspaceSelectItem::from_workspace),
        );
        let workspace_select =
            cx.new(|cx| SelectState::new(workspace_items, Some(Default::default()), window, cx));

        let team_select = create_team_select(&config.teams, None, window, cx);

        let mut auth_method = AuthMethodSelection::Password;
        let mut jump_auth_method = AuthMethodSelection::Password;
        let mut workspace_id: Option<i64> = None;
        let mut enable_jump_server = false;
        let mut enable_proxy = false;
        let mut proxy_type = ProxyTypeSelection::default();
        let mut sync_enabled = true; // 默认启用云同步
        let mut disable_shell_integration = false;
        let mut detected_os_id: Option<String> = None;
        let mut manual_icon: Option<String> = None;

        if let Some(conn) = config.connection_to_load() {
            // 加载同步状态
            sync_enabled = conn.sync_enabled;

            if let Ok(params) = conn.to_ssh_params() {
                detected_os_id = params.os_id.clone();
                manual_icon = params.icon.clone();
                name_input.update(cx, |s, cx| s.set_value(&conn.name, window, cx));
                host_input.update(cx, |s, cx| s.set_value(&params.host, window, cx));
                port_input.update(cx, |s, cx| {
                    s.set_value(&params.port.to_string(), window, cx)
                });
                username_input.update(cx, |s, cx| s.set_value(&params.username, window, cx));

                match params.auth_method {
                    SshAuthMethod::Password { ref password } => {
                        auth_method = AuthMethodSelection::Password;
                        password_input.update(cx, |s, cx| s.set_value(password, window, cx));
                    }
                    SshAuthMethod::PrivateKey {
                        ref key_path,
                        ref passphrase,
                    } => {
                        auth_method = AuthMethodSelection::PrivateKey;
                        key_path_input.update(cx, |s, cx| s.set_value(key_path, window, cx));
                        if let Some(ref pass) = passphrase {
                            passphrase_input.update(cx, |s, cx| s.set_value(pass, window, cx));
                        }
                    }
                    SshAuthMethod::PrivateKeyContent {
                        ref private_key,
                        ref passphrase,
                    } => {
                        auth_method = AuthMethodSelection::PrivateKeyContent;
                        private_key_content_input
                            .update(cx, |s, cx| s.set_value(private_key, window, cx));
                        if let Some(ref pass) = passphrase {
                            passphrase_input.update(cx, |s, cx| s.set_value(pass, window, cx));
                        }
                    }
                    SshAuthMethod::Agent => {
                        auth_method = AuthMethodSelection::Agent;
                    }
                    SshAuthMethod::AutoPublicKey => {
                        auth_method = AuthMethodSelection::AutoPublicKey;
                    }
                }

                // 加载高级设置
                if let Some(timeout) = params.connect_timeout {
                    connect_timeout_input
                        .update(cx, |s, cx| s.set_value(&timeout.to_string(), window, cx));
                }
                if let Some(interval) = params.keepalive_interval {
                    keepalive_interval_input
                        .update(cx, |s, cx| s.set_value(&interval.to_string(), window, cx));
                }
                if let Some(max) = params.keepalive_max {
                    keepalive_max_input
                        .update(cx, |s, cx| s.set_value(&max.to_string(), window, cx));
                }

                // 加载初始化设置
                if let Some(ref dir) = params.default_directory {
                    default_directory_input.update(cx, |s, cx| s.set_value(dir, window, cx));
                }
                if let Some(ref script) = params.init_script {
                    init_script_input.update(cx, |s, cx| s.set_value(script, window, cx));
                }
                disable_shell_integration = params.disable_shell_integration.unwrap_or(false);

                // 加载跳板机设置
                if let Some(ref jump) = params.jump_server {
                    enable_jump_server = true;
                    jump_host_input.update(cx, |s, cx| s.set_value(&jump.host, window, cx));
                    jump_port_input
                        .update(cx, |s, cx| s.set_value(&jump.port.to_string(), window, cx));
                    jump_username_input.update(cx, |s, cx| s.set_value(&jump.username, window, cx));
                    match jump.auth_method {
                        SshAuthMethod::Password { ref password } => {
                            jump_auth_method = AuthMethodSelection::Password;
                            jump_password_input
                                .update(cx, |s, cx| s.set_value(password, window, cx));
                        }
                        SshAuthMethod::PrivateKey {
                            ref key_path,
                            ref passphrase,
                        } => {
                            jump_auth_method = AuthMethodSelection::PrivateKey;
                            jump_key_path_input
                                .update(cx, |s, cx| s.set_value(key_path, window, cx));
                            if let Some(ref pass) = passphrase {
                                jump_passphrase_input
                                    .update(cx, |s, cx| s.set_value(pass, window, cx));
                            }
                        }
                        SshAuthMethod::PrivateKeyContent {
                            ref private_key,
                            ref passphrase,
                        } => {
                            jump_auth_method = AuthMethodSelection::PrivateKeyContent;
                            jump_private_key_content_input
                                .update(cx, |s, cx| s.set_value(private_key, window, cx));
                            if let Some(ref pass) = passphrase {
                                jump_passphrase_input
                                    .update(cx, |s, cx| s.set_value(pass, window, cx));
                            }
                        }
                        SshAuthMethod::Agent => {
                            jump_auth_method = AuthMethodSelection::Agent;
                        }
                        SshAuthMethod::AutoPublicKey => {
                            jump_auth_method = AuthMethodSelection::AutoPublicKey;
                        }
                    }
                }

                // 加载代理设置
                if let Some(ref proxy) = params.proxy {
                    enable_proxy = true;
                    proxy_type = match proxy.proxy_type {
                        StorageProxyType::Socks5 => ProxyTypeSelection::Socks5,
                        StorageProxyType::Http => ProxyTypeSelection::Http,
                    };
                    proxy_host_input.update(cx, |s, cx| s.set_value(&proxy.host, window, cx));
                    proxy_port_input
                        .update(cx, |s, cx| s.set_value(&proxy.port.to_string(), window, cx));
                    if let Some(ref username) = proxy.username {
                        proxy_username_input.update(cx, |s, cx| s.set_value(username, window, cx));
                    }
                    if let Some(ref password) = proxy.password {
                        proxy_password_input.update(cx, |s, cx| s.set_value(password, window, cx));
                    }
                }
            }
            workspace_id = conn.workspace_id;

            // 加载团队归属
            if let Some(ref team_id) = conn.team_id {
                team_select.update(cx, |select, cx| {
                    select.set_selected_value(&Some(team_id.clone()), window, cx);
                });
            }

            // 加载备注
            if let Some(ref remark) = conn.remark {
                remark_input.update(cx, |s, cx| s.set_value(remark, window, cx));
            }
        }

        if let Some(ws_id) = workspace_id {
            workspace_select.update(cx, |select, cx| {
                select.set_selected_value(&Some(ws_id), window, cx);
            });
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
            key_path_input,
            private_key_content_input,
            passphrase_input,
            auth_method,
            workspace_select,
            team_select,
            enable_jump_server,
            jump_auth_method,
            jump_host_input,
            jump_port_input,
            jump_username_input,
            jump_password_input,
            jump_key_path_input,
            jump_private_key_content_input,
            jump_passphrase_input,
            jump_mfa_request: None,
            jump_mfa_inputs: Vec::new(),
            jump_mfa_signature: None,
            enable_proxy,
            proxy_type,
            proxy_host_input,
            proxy_port_input,
            proxy_username_input,
            proxy_password_input,
            connect_timeout_input,
            keepalive_interval_input,
            keepalive_max_input,
            init_script_input,
            default_directory_input,
            remark_input,
            last_tested_signature: None,
            detected_os_id,
            manual_icon,
            sync_enabled,
            disable_shell_integration,
            is_testing: false,
            is_uninstalling_shell_integration: false,
            test_result: None,
            shell_integration_uninstall_result: None,
            on_saved,
            save_action: SaveAction::Close,
        }
    }

    fn get_workspace_id(&self, cx: &App) -> Option<i64> {
        self.workspace_select
            .read(cx)
            .selected_value()
            .cloned()
            .flatten()
    }

    fn get_team_id(&self, cx: &App) -> Option<String> {
        selected_team_id(&self.team_select, cx)
    }

    fn request_team_sync(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        refresh_team_options(&self.team_select, window, cx);
    }

    fn build_ssh_params(&self, cx: &App) -> Option<SshParams> {
        let host = self.host_input.read(cx).text().to_string();
        let port: u16 = self
            .port_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .unwrap_or(22);
        let username = self.username_input.read(cx).text().to_string();

        if host.is_empty() || username.is_empty() {
            return None;
        }

        let auth_method = match self.auth_method {
            AuthMethodSelection::Password => {
                let password = self.password_input.read(cx).text().to_string();
                SshAuthMethod::Password { password }
            }
            AuthMethodSelection::PrivateKey => {
                let key_path = self.key_path_input.read(cx).text().to_string();
                let passphrase = {
                    let p = self.passphrase_input.read(cx).text().to_string();
                    if p.is_empty() { None } else { Some(p) }
                };
                SshAuthMethod::PrivateKey {
                    key_path,
                    passphrase,
                }
            }
            AuthMethodSelection::PrivateKeyContent => {
                let private_key = self.private_key_content_input.read(cx).text().to_string();
                let passphrase = {
                    let p = self.passphrase_input.read(cx).text().to_string();
                    if p.is_empty() { None } else { Some(p) }
                };
                SshAuthMethod::PrivateKeyContent {
                    private_key,
                    passphrase,
                }
            }
            AuthMethodSelection::Agent => SshAuthMethod::Agent,
            AuthMethodSelection::AutoPublicKey => SshAuthMethod::AutoPublicKey,
        };

        // 高级设置
        let connect_timeout: Option<u64> = self
            .connect_timeout_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .ok();
        let keepalive_interval: Option<u64> = self
            .keepalive_interval_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .ok();
        let keepalive_max: Option<usize> = self
            .keepalive_max_input
            .read(cx)
            .text()
            .to_string()
            .parse()
            .ok();

        // 初始化设置
        let default_directory = {
            let d = self.default_directory_input.read(cx).text().to_string();
            if d.is_empty() { None } else { Some(d) }
        };
        let init_script = {
            let s = self.init_script_input.read(cx).text().to_string();
            if s.is_empty() { None } else { Some(s) }
        };

        // 跳板机配置
        let jump_server = if self.enable_jump_server {
            let jump_host = self.jump_host_input.read(cx).text().to_string();
            let jump_username = self.jump_username_input.read(cx).text().to_string();
            if !jump_host.is_empty() && !jump_username.is_empty() {
                let jump_port: u16 = self
                    .jump_port_input
                    .read(cx)
                    .text()
                    .to_string()
                    .parse()
                    .unwrap_or(22);
                let jump_password = self.jump_password_input.read(cx).text().to_string();
                let jump_key_path = self.jump_key_path_input.read(cx).text().to_string();
                let jump_private_key = self
                    .jump_private_key_content_input
                    .read(cx)
                    .text()
                    .to_string();
                let jump_passphrase = self.jump_passphrase_input.read(cx).text().to_string();
                Some(JumpServerConfig {
                    host: jump_host,
                    port: jump_port,
                    username: jump_username,
                    auth_method: build_jump_auth_method(
                        self.jump_auth_method,
                        jump_password,
                        jump_key_path,
                        jump_private_key,
                        jump_passphrase,
                    ),
                })
            } else {
                None
            }
        } else {
            None
        };

        // 代理配置
        let proxy = if self.enable_proxy {
            let proxy_host = self.proxy_host_input.read(cx).text().to_string();
            if !proxy_host.is_empty() {
                let proxy_port: u16 = self
                    .proxy_port_input
                    .read(cx)
                    .text()
                    .to_string()
                    .parse()
                    .unwrap_or(1080);
                let proxy_username = {
                    let u = self.proxy_username_input.read(cx).text().to_string();
                    if u.is_empty() { None } else { Some(u) }
                };
                let proxy_password = {
                    let p = self.proxy_password_input.read(cx).text().to_string();
                    if p.is_empty() { None } else { Some(p) }
                };
                let proxy_type = match self.proxy_type {
                    ProxyTypeSelection::Socks5 => StorageProxyType::Socks5,
                    ProxyTypeSelection::Http => StorageProxyType::Http,
                };
                Some(ProxyConfig {
                    proxy_type,
                    host: proxy_host,
                    port: proxy_port,
                    username: proxy_username,
                    password: proxy_password,
                })
            } else {
                None
            }
        } else {
            None
        };

        Some(SshParams {
            host,
            port,
            username,
            auth_method,
            connect_timeout,
            keepalive_interval,
            keepalive_max,
            default_directory,
            init_script,
            disable_shell_integration: if self.disable_shell_integration {
                Some(true)
            } else {
                None
            },
            jump_server,
            proxy,
            os_id: self.detected_os_id.clone(),
            icon: self.manual_icon.clone(),
        })
    }

    fn build_ssh_connect_config(&self, params: &SshParams) -> SshConnectConfig {
        let auth = match &params.auth_method {
            SshAuthMethod::Password { password } => SshAuth::Password(password.clone()),
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase,
            } => SshAuth::PrivateKey {
                key_path: key_path.clone(),
                passphrase: passphrase.clone(),
                certificate_path: None,
            },
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase,
            } => SshAuth::PrivateKeyContent {
                private_key: private_key.clone(),
                passphrase: passphrase.clone(),
                certificate_path: None,
            },
            SshAuthMethod::Agent => SshAuth::Agent,
            SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
        };

        // 构建跳板机配置
        let jump_server = params.jump_server.as_ref().map(|jump| {
            let jump_auth = match &jump.auth_method {
                SshAuthMethod::Password { password } => SshAuth::Password(password.clone()),
                SshAuthMethod::PrivateKey {
                    key_path,
                    passphrase,
                } => SshAuth::PrivateKey {
                    key_path: key_path.clone(),
                    passphrase: passphrase.clone(),
                    certificate_path: None,
                },
                SshAuthMethod::PrivateKeyContent {
                    private_key,
                    passphrase,
                } => SshAuth::PrivateKeyContent {
                    private_key: private_key.clone(),
                    passphrase: passphrase.clone(),
                    certificate_path: None,
                },
                SshAuthMethod::Agent => SshAuth::Agent,
                SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
            };
            JumpServerConnectConfig {
                host: jump.host.clone(),
                port: jump.port,
                username: jump.username.clone(),
                auth: jump_auth,
            }
        });

        // 构建代理配置
        let proxy = params.proxy.as_ref().map(|p| {
            let proxy_type = match p.proxy_type {
                StorageProxyType::Socks5 => ProxyType::Socks5,
                StorageProxyType::Http => ProxyType::Http,
            };
            ProxyConnectConfig {
                proxy_type,
                host: p.host.clone(),
                port: p.port,
                username: p.username.clone(),
                password: p.password.clone(),
            }
        });

        SshConnectConfig {
            host: params.host.clone(),
            port: params.port,
            username: params.username.clone(),
            auth,
            timeout: params.connect_timeout.map(Duration::from_secs),
            keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
            keepalive_max: params.keepalive_max,
            jump_server,
            proxy,
            keyboard_interactive_responder: None,
        }
    }

    fn collect_jump_mfa_responses(&self, cx: &App, signature: &str) -> Vec<String> {
        if self.jump_mfa_signature.as_deref() != Some(signature) {
            return Vec::new();
        }

        self.jump_mfa_inputs
            .iter()
            .map(|input| input.input.read(cx).text().to_string())
            .collect()
    }

    fn clear_jump_mfa_fields(&mut self) {
        self.jump_mfa_request = None;
        self.jump_mfa_inputs.clear();
        self.jump_mfa_signature = None;
    }

    fn apply_jump_mfa_request(
        &mut self,
        request: ssh::KeyboardInteractiveRequest,
        signature: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(form_request) = form_mfa_request_from_keyboard_interactive(&request) else {
            return;
        };
        let existing_values = self
            .jump_mfa_inputs
            .iter()
            .map(|input| input.input.read(cx).text().to_string())
            .collect::<Vec<_>>();

        self.jump_mfa_inputs = form_request
            .prompts
            .iter()
            .enumerate()
            .map(|(index, prompt)| {
                let existing_value = existing_values.get(index).cloned().unwrap_or_default();
                let input = cx.new(|cx| {
                    let mut state =
                        InputState::new(window, cx).placeholder(mfa_prompt_label(&prompt.prompt));
                    if !prompt.echo {
                        state = state.masked(true);
                    }
                    if !existing_value.is_empty() {
                        state.set_value(&existing_value, window, cx);
                    }
                    state
                });
                JumpMfaInput {
                    prompt: prompt.clone(),
                    input,
                }
            })
            .collect();
        self.jump_mfa_request = Some(form_request);
        self.jump_mfa_signature = Some(signature);
        self.active_tab = 2;
    }

    fn on_test(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(params) = self.build_ssh_params(cx) else {
            self.last_tested_signature = None;
            self.test_result = Some(Err(t!("SSH.validation_error").to_string()));
            cx.notify();
            return;
        };

        self.is_testing = true;
        self.last_tested_signature = None;
        self.test_result = None;
        cx.notify();

        let signature = build_connection_test_signature(&params);
        let target_host = params.host.clone();
        let mut config = self.build_ssh_connect_config(&params);
        let jump_mfa_capture = CapturedMfaRequest::default();
        let jump_mfa_responses = self.collect_jump_mfa_responses(cx, &signature);
        if let Some(jump_server) = &config.jump_server {
            let jump_password = match &jump_server.auth {
                SshAuth::Password(password) => Some(password.clone()),
                _ => None,
            };
            config.keyboard_interactive_responder = Some(Arc::new(JumpServerMfaResponder::new(
                jump_mfa_responses,
                jump_password,
                jump_mfa_capture.clone(),
            )));
        }
        let window_handle = window.window_handle();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let spawn_result = Tokio::spawn_result(cx, async move {
                let mut client = RusshClient::connect(config).await?;
                let os_id = detect_remote_os_id(&mut client).await;
                client.disconnect().await?;
                Ok::<Option<String>, anyhow::Error>(os_id)
            })
            .await;

            let jump_mfa_request = match &spawn_result {
                Err(error) if is_jump_mfa_required_error(error.as_ref()) => jump_mfa_capture.take(),
                _ => None,
            };
            let test_result: Result<Option<String>, String> = match spawn_result {
                Ok(task) => Ok(task),
                Err(error) => Err(format_connection_error(&error, &target_host)),
            };

            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.is_testing = false;
                    if let Some(request) = jump_mfa_request {
                        this.apply_jump_mfa_request(request, signature.clone(), window, cx);
                        this.last_tested_signature = None;
                        this.test_result = Some(Err(t!("SSH.jump_mfa_required").to_string()));
                    } else {
                        this.last_tested_signature =
                            test_result.as_ref().ok().map(|_| signature.clone());
                        if let Ok(os_id) = &test_result {
                            this.detected_os_id = os_id.clone();
                        }
                        this.test_result = Some(test_result);
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn on_uninstall_shell_integration(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(params) = self.build_ssh_params(cx) else {
            self.shell_integration_uninstall_result =
                Some(Err(t!("SSH.validation_error").to_string()));
            cx.notify();
            return;
        };

        self.is_uninstalling_shell_integration = true;
        self.shell_integration_uninstall_result = None;
        cx.notify();

        let signature = build_connection_test_signature(&params);
        let mut config = self.build_ssh_connect_config(&params);
        let jump_mfa_capture = CapturedMfaRequest::default();
        let jump_mfa_responses = self.collect_jump_mfa_responses(cx, &signature);
        if let Some(jump_server) = &config.jump_server {
            let jump_password = match &jump_server.auth {
                SshAuth::Password(password) => Some(password.clone()),
                _ => None,
            };
            config.keyboard_interactive_responder = Some(Arc::new(JumpServerMfaResponder::new(
                jump_mfa_responses,
                jump_password,
                jump_mfa_capture.clone(),
            )));
        }
        let window_handle = window.window_handle();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let spawn_result = Tokio::spawn_result(cx, async move {
                let session_manager = Arc::new(SshSessionManager::new(config));
                SshBackend::uninstall_shell_integration(session_manager).await
            })
            .await;

            let jump_mfa_request = match &spawn_result {
                Err(error) if is_jump_mfa_required_error(error.as_ref()) => jump_mfa_capture.take(),
                _ => None,
            };
            let uninstall_result = spawn_result.map_err(|error| error.to_string());

            let _ = cx.update_window(window_handle, |_, window, cx| {
                let _ = this.update(cx, |this, cx| {
                    this.is_uninstalling_shell_integration = false;
                    if let Some(request) = jump_mfa_request {
                        this.apply_jump_mfa_request(request, signature, window, cx);
                        this.shell_integration_uninstall_result =
                            Some(Err(t!("SSH.jump_mfa_required").to_string()));
                    } else {
                        this.shell_integration_uninstall_result = Some(uninstall_result);
                    }
                    cx.notify();
                });
            });
        })
        .detach();
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_action = SaveAction::Close;
        self.save(window, cx);
    }

    fn on_save_and_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.save_action = SaveAction::Continue;
        self.save(window, cx);
    }

    fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Err(reason) =
            validate_save_state(self.is_testing, self.is_uninstalling_shell_integration)
        {
            self.test_result = Some(Err(save_block_message(reason)));
            cx.notify();
            return;
        }

        let Some(params) = self.build_ssh_params(cx) else {
            self.last_tested_signature = None;
            self.test_result = Some(Err(t!("SSH.validation_error").to_string()));
            cx.notify();
            return;
        };

        let name = self.name_input.read(cx).text().to_string();
        let name = if name.is_empty() {
            format!("{}@{}:{}", params.username, params.host, params.port)
        } else {
            name
        };

        let workspace_id = self.get_workspace_id(cx);
        let mut conn = StoredConnection::new_ssh(name, params, workspace_id);
        conn.sync_enabled = self.sync_enabled; // 设置同步状态
        let assignment = match resolve_team_assignment(
            self.get_team_id(cx),
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
        conn.team_id = assignment.team_id;
        conn.owner_id = assignment.owner_id;
        if self.is_editing {
            conn.id = self.editing_id;
            conn.cloud_id = self.editing_cloud_id.clone();
            conn.last_synced_at = self.editing_last_synced_at;
        }

        // 保存备注
        let remark = self.remark_input.read(cx).text().to_string();
        if !remark.is_empty() {
            conn.remark = Some(remark);
        }

        let storage = cx
            .global::<one_core::storage::GlobalStorageState>()
            .storage
            .clone();
        let is_editing = self.is_editing;

        let result: Result<StoredConnection, anyhow::Error> = (|| {
            let repo = storage
                .get::<one_core::storage::ConnectionRepository>()
                .ok_or_else(|| anyhow::anyhow!("ConnectionRepository not found"))?;

            if is_editing {
                repo.update(&mut conn)?;
            } else {
                repo.insert(&mut conn)?;
            }
            Ok(conn)
        })();

        match result {
            Ok(saved_conn) => {
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
                if let Some(callback) = self.on_saved.as_ref() {
                    callback(saved_conn, post_save_action(self.save_action), window, cx);
                }
                window.remove_window();
            }
            Err(e) => {
                let error_msg = t!("SSH.save_failed", error = e).to_string();
                tracing::error!("{}", error_msg);
                self.test_result = Some(Err(error_msg));
                cx.notify();
            }
        }
    }

    fn on_cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
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

    /// 渲染连接图标选择器：自动（跟随测试连接探测结果）或手动固定图标。
    fn render_icon_picker(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let accent = cx.theme().accent;

        let tile = |id: &str, selected: bool, icon: Option<IconName>| {
            let id_string = (!id.is_empty()).then(|| id.to_string());
            let mut tile = div()
                .id(SharedString::from(format!(
                    "ssh-icon-{}",
                    if id.is_empty() { "auto" } else { id }
                )))
                .w(px(36.0))
                .h(px(36.0))
                .flex()
                .items_center()
                .justify_center()
                .rounded_md()
                .border_1()
                .cursor_pointer();
            if selected {
                tile = tile.border_color(accent).bg(accent.opacity(0.12));
            } else {
                tile = tile.border_color(border);
            }
            match icon {
                Some(icon) => tile.child(icon.color().with_size(px(22.0))),
                None => tile.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("SSH.icon_auto").to_string()),
                ),
            }
            .on_click(cx.listener(move |this, _, _, cx| {
                this.manual_icon = id_string.clone();
                cx.notify();
            }))
        };

        let mut row = h_flex().gap_2().flex_wrap().items_center();
        // 自动：跟随测试连接探测到的系统图标（未探测到时为默认企鹅）
        row = row.child(tile("", self.manual_icon.is_none(), None));
        for id in SSH_ICON_IDS {
            row = row.child(tile(
                id,
                self.manual_icon.as_deref() == Some(*id),
                Some(ssh_os_icon(Some(id))),
            ));
        }
        row.child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("SSH.icon_hint").to_string()),
        )
    }

    /// 渲染基本信息标签页
    fn render_basic_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let auth_method = self.auth_method;

        v_flex()
            .gap_2()
            .child(self.render_form_row(&t!("SSH.name"), Input::new(&self.name_input)))
            .child(self.render_form_row(&t!("SSH.icon"), self.render_icon_picker(cx)))
            .child(self.render_form_row(&t!("SSH.host"), Input::new(&self.host_input)))
            .child(self.render_form_row(&t!("SSH.port"), Input::new(&self.port_input)))
            .child(self.render_form_row(&t!("SSH.username"), Input::new(&self.username_input)))
            .child(
                self.render_form_row(
                    &t!("SSH.auth_method"),
                    h_flex()
                        .gap_4()
                        .flex_wrap()
                        .child(
                            Radio::new("password")
                                .label(t!("SSH.password").to_string())
                                .checked(auth_method == AuthMethodSelection::Password)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.auth_method = AuthMethodSelection::Password;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("private-key")
                                .label(t!("SSH.private_key").to_string())
                                .checked(auth_method == AuthMethodSelection::PrivateKey)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.auth_method = AuthMethodSelection::PrivateKey;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("private-key-content")
                                .label(t!("SSH.private_key_content").to_string())
                                .checked(auth_method == AuthMethodSelection::PrivateKeyContent)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.auth_method = AuthMethodSelection::PrivateKeyContent;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("agent")
                                .label(t!("SSH.agent").to_string())
                                .checked(auth_method == AuthMethodSelection::Agent)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.auth_method = AuthMethodSelection::Agent;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Radio::new("auto-publickey")
                                .label(t!("SSH.auto_publickey").to_string())
                                .checked(auth_method == AuthMethodSelection::AutoPublicKey)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.auth_method = AuthMethodSelection::AutoPublicKey;
                                    cx.notify();
                                })),
                        ),
                ),
            )
            .when(auth_method == AuthMethodSelection::Password, |this| {
                this.child(self.render_form_row(
                    &t!("SSH.password"),
                    Input::new(&self.password_input).mask_toggle(),
                ))
            })
            .when(auth_method == AuthMethodSelection::PrivateKey, |this| {
                this.child(
                    self.render_form_row(&t!("SSH.key_path"), Input::new(&self.key_path_input)),
                )
                .child(self.render_form_row(
                    &t!("SSH.passphrase"),
                    Input::new(&self.passphrase_input).mask_toggle(),
                ))
            })
            .when(
                auth_method == AuthMethodSelection::PrivateKeyContent,
                |this| {
                    this.child(self.render_form_row(
                        &t!("SSH.private_key_content"),
                        Input::new(&self.private_key_content_input),
                    ))
                    .child(self.render_form_row(
                        &t!("SSH.passphrase"),
                        Input::new(&self.passphrase_input).mask_toggle(),
                    ))
                    .child(
                        h_flex().justify_center().child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("SSH.private_key_content_sync_hint").to_string()),
                        ),
                    )
                },
            )
            .when(auth_method == AuthMethodSelection::AutoPublicKey, |this| {
                this.child(
                    h_flex().justify_center().child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("SSH.auto_publickey_hint").to_string()),
                    ),
                )
            })
            .child(self.render_form_row(
                &t!("SSH.workspace"),
                Select::new(&self.workspace_select).w_full(),
            ))
            .when(team_management_enabled(cx), |form| {
                form.child(
                    self.render_form_row(
                        &team_label(),
                        h_flex()
                            .gap_2()
                            .child(Select::new(&self.team_select).w_full())
                            .child(
                                Button::new("sync-ssh-teams")
                                    .icon(IconName::Refresh)
                                    .ghost()
                                    .tooltip(refresh_teams_tooltip())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.request_team_sync(window, cx);
                                    })),
                            ),
                    ),
                )
            })
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

    /// 渲染初始化标签页
    fn render_init_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(
                &t!("SSH.default_directory"),
                Input::new(&self.default_directory_input),
            ))
            .child(
                self.render_form_row(&t!("SSH.init_script"), Input::new(&self.init_script_input)),
            )
            .child(
                self.render_form_row(
                    &t!("SSH.disable_shell_integration"),
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new("disable-shell-integration")
                                .checked(self.disable_shell_integration)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.disable_shell_integration =
                                        !this.disable_shell_integration;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("SSH.disable_shell_integration_desc").to_string()),
                        ),
                ),
            )
            .child(
                self.render_form_row(
                    &t!("SSH.remote_shell_integration"),
                    v_flex()
                        .gap_1()
                        .child(
                            h_flex().gap_2().child(
                                Button::new("uninstall-shell-integration")
                                    .icon(IconName::Remove)
                                    .danger()
                                    .small()
                                    .label(if self.is_uninstalling_shell_integration {
                                        t!("SSH.uninstalling_shell_integration").to_string()
                                    } else {
                                        t!("SSH.uninstall_shell_integration").to_string()
                                    })
                                    .disabled(
                                        self.is_testing || self.is_uninstalling_shell_integration,
                                    )
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.on_uninstall_shell_integration(window, cx);
                                    })),
                            ),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("SSH.uninstall_shell_integration_desc").to_string()),
                        ),
                ),
            )
    }

    /// 渲染跳板机标签页
    fn render_jump_server_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enable_jump = self.enable_jump_server;
        let jump_auth_method = self.jump_auth_method;

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("SSH.enable_jump_server"),
                    Checkbox::new("enable-jump")
                        .checked(enable_jump)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.enable_jump_server = !this.enable_jump_server;
                            if !this.enable_jump_server {
                                this.clear_jump_mfa_fields();
                            }
                            cx.notify();
                        })),
                ),
            )
            .when(enable_jump, |this| {
                this.child(
                    self.render_form_row(&t!("SSH.jump_host"), Input::new(&self.jump_host_input)),
                )
                .child(
                    self.render_form_row(&t!("SSH.jump_port"), Input::new(&self.jump_port_input)),
                )
                .child(self.render_form_row(
                    &t!("SSH.jump_username"),
                    Input::new(&self.jump_username_input),
                ))
                .child(
                    self.render_form_row(
                        &t!("SSH.jump_auth_method"),
                        h_flex()
                            .gap_4()
                            .flex_wrap()
                            .child(
                                Radio::new("jump-password")
                                    .label(t!("SSH.password").to_string())
                                    .checked(jump_auth_method == AuthMethodSelection::Password)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.jump_auth_method = AuthMethodSelection::Password;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Radio::new("jump-private-key")
                                    .label(t!("SSH.private_key").to_string())
                                    .checked(jump_auth_method == AuthMethodSelection::PrivateKey)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.jump_auth_method = AuthMethodSelection::PrivateKey;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Radio::new("jump-private-key-content")
                                    .label(t!("SSH.private_key_content").to_string())
                                    .checked(
                                        jump_auth_method == AuthMethodSelection::PrivateKeyContent,
                                    )
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.jump_auth_method =
                                            AuthMethodSelection::PrivateKeyContent;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Radio::new("jump-agent")
                                    .label(t!("SSH.agent").to_string())
                                    .checked(jump_auth_method == AuthMethodSelection::Agent)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.jump_auth_method = AuthMethodSelection::Agent;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Radio::new("jump-auto-publickey")
                                    .label(t!("SSH.auto_publickey").to_string())
                                    .checked(jump_auth_method == AuthMethodSelection::AutoPublicKey)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.jump_auth_method = AuthMethodSelection::AutoPublicKey;
                                        cx.notify();
                                    })),
                            ),
                    ),
                )
                .when(jump_auth_method == AuthMethodSelection::Password, |this| {
                    this.child(self.render_form_row(
                        &t!("SSH.jump_password"),
                        Input::new(&self.jump_password_input).mask_toggle(),
                    ))
                })
                .when(
                    jump_auth_method == AuthMethodSelection::PrivateKey,
                    |this| {
                        this.child(self.render_form_row(
                            &t!("SSH.jump_key_path"),
                            Input::new(&self.jump_key_path_input),
                        ))
                        .child(self.render_form_row(
                            &t!("SSH.jump_passphrase"),
                            Input::new(&self.jump_passphrase_input).mask_toggle(),
                        ))
                    },
                )
                .when(
                    jump_auth_method == AuthMethodSelection::PrivateKeyContent,
                    |this| {
                        this.child(self.render_form_row(
                            &t!("SSH.private_key_content"),
                            Input::new(&self.jump_private_key_content_input),
                        ))
                        .child(self.render_form_row(
                            &t!("SSH.jump_passphrase"),
                            Input::new(&self.jump_passphrase_input).mask_toggle(),
                        ))
                        .child(
                            h_flex().justify_center().child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("SSH.private_key_content_sync_hint").to_string()),
                            ),
                        )
                    },
                )
                .when(
                    jump_auth_method == AuthMethodSelection::AutoPublicKey,
                    |this| {
                        this.child(
                            h_flex().justify_center().child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("SSH.auto_publickey_hint").to_string()),
                            ),
                        )
                    },
                )
                .when_some(self.jump_mfa_request.as_ref(), |this, request| {
                    this.child(
                        v_flex()
                            .gap_2()
                            .pt_2()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("SSH.jump_mfa_required_hint").to_string()),
                            )
                            .when(!request.name.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(request.name.clone()),
                                )
                            })
                            .when(!request.instructions.is_empty(), |this| {
                                this.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(request.instructions.clone()),
                                )
                            })
                            .children(self.jump_mfa_inputs.iter().map(|input| {
                                let input_element = if input.prompt.echo {
                                    Input::new(&input.input).into_any_element()
                                } else {
                                    Input::new(&input.input).mask_toggle().into_any_element()
                                };
                                self.render_form_row(
                                    &mfa_prompt_label(&input.prompt.prompt),
                                    input_element,
                                )
                                .into_any_element()
                            })),
                    )
                })
            })
    }

    /// 渲染代理标签页
    fn render_proxy_tab(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let enable_proxy = self.enable_proxy;
        let proxy_type = self.proxy_type;

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    &t!("SSH.enable_proxy"),
                    Checkbox::new("enable-proxy")
                        .checked(enable_proxy)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.enable_proxy = !this.enable_proxy;
                            cx.notify();
                        })),
                ),
            )
            .when(enable_proxy, |this| {
                this.child(
                    self.render_form_row(
                        &t!("SSH.proxy_type"),
                        h_flex()
                            .gap_4()
                            .child(
                                Radio::new("socks5")
                                    .label("SOCKS5")
                                    .checked(proxy_type == ProxyTypeSelection::Socks5)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.proxy_type = ProxyTypeSelection::Socks5;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Radio::new("http")
                                    .label("HTTP")
                                    .checked(proxy_type == ProxyTypeSelection::Http)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.proxy_type = ProxyTypeSelection::Http;
                                        cx.notify();
                                    })),
                            ),
                    ),
                )
                .child(
                    self.render_form_row(&t!("SSH.proxy_host"), Input::new(&self.proxy_host_input)),
                )
                .child(
                    self.render_form_row(&t!("SSH.proxy_port"), Input::new(&self.proxy_port_input)),
                )
                .child(self.render_form_row(
                    &t!("SSH.proxy_username"),
                    Input::new(&self.proxy_username_input),
                ))
                .child(self.render_form_row(
                    &t!("SSH.proxy_password"),
                    Input::new(&self.proxy_password_input).mask_toggle(),
                ))
            })
    }

    /// 渲染高级设置标签页
    fn render_advanced_tab(&self) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(
                &t!("SSH.connect_timeout"),
                Input::new(&self.connect_timeout_input),
            ))
            .child(self.render_form_row(
                &t!("SSH.keepalive_interval"),
                Input::new(&self.keepalive_interval_input),
            ))
            .child(self.render_form_row(
                &t!("SSH.keepalive_max"),
                Input::new(&self.keepalive_max_input),
            ))
    }

    /// 渲染其他设置标签页
    fn render_other_tab(&self) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(&t!("SSH.remark"), Input::new(&self.remark_input)))
    }
}

fn mfa_prompt_label(prompt: &str) -> String {
    let prompt = prompt.trim();
    if prompt.is_empty() {
        t!("SSH.mfa_prompt_placeholder").to_string()
    } else {
        prompt.to_string()
    }
}

impl Focusable for SshFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SshFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.is_testing;
        let is_uninstalling_shell_integration = self.is_uninstalling_shell_integration;
        let is_busy = is_testing || is_uninstalling_shell_integration;
        let active_tab = self.active_tab;

        let test_result_element = match &self.test_result {
            Some(Ok(os_id)) => Some(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(ssh_os_icon(os_id.as_deref()).color().with_size(px(16.0)))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().success)
                            .child(t!("SSH.test_success").to_string()),
                    )
                    .into_any_element(),
            ),
            Some(Err(e)) => Some(
                div()
                    .mx_6()
                    .px_3()
                    .py_2()
                    .rounded_md()
                    .bg(cx.theme().danger.opacity(0.12))
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .max_h(px(120.0))
                    .overflow_y_scrollbar()
                    .child(e.clone())
                    .into_any_element(),
            ),
            None => None,
        };

        let uninstall_result_element = match &self.shell_integration_uninstall_result {
            Some(Ok(())) => Some(
                div()
                    .text_sm()
                    .text_color(cx.theme().success)
                    .child(t!("SSH.uninstall_shell_integration_success").to_string()),
            ),
            Some(Err(e)) => Some(div().text_sm().text_color(cx.theme().danger).child(
                t!("SSH.uninstall_shell_integration_failed", error = e.as_str()).to_string(),
            )),
            None => None,
        };

        v_flex()
            .justify_center()
            .size_full()
            // TabBar
            .child(
                div().flex().justify_center().px_3().pt_2().child(
                    TabBar::new("ssh-form-tabs")
                        .with_size(Size::Small)
                        .underline()
                        .selected_index(active_tab)
                        .on_click(cx.listener(|this, ix: &usize, _, cx| {
                            this.active_tab = *ix;
                            cx.notify();
                        }))
                        .child(Tab::new().label(t!("SSH.tab_basic").to_string()))
                        .child(Tab::new().label(t!("SSH.tab_init").to_string()))
                        .child(Tab::new().label(t!("SSH.tab_jump_server").to_string()))
                        .child(Tab::new().label(t!("SSH.tab_proxy").to_string()))
                        .child(Tab::new().label(t!("SSH.tab_advanced").to_string()))
                        .child(Tab::new().label(t!("SSH.tab_other").to_string())),
                ),
            )
            // 标签页内容
            .child(
                div()
                    .id("ssh-form-content")
                    .flex_1()
                    .p_3()
                    .overflow_y_scroll()
                    .child(match active_tab {
                        0 => self.render_basic_tab(cx).into_any_element(),
                        1 => self.render_init_tab(cx).into_any_element(),
                        2 => self.render_jump_server_tab(cx).into_any_element(),
                        3 => self.render_proxy_tab(cx).into_any_element(),
                        4 => self.render_advanced_tab().into_any_element(),
                        5 => self.render_other_tab().into_any_element(),
                        _ => div().into_any_element(),
                    }),
            )
            // 测试结果
            .when_some(test_result_element, |this, elem| {
                this.child(h_flex().justify_center().pb_2().child(elem))
            })
            .when_some(uninstall_result_element, |this, elem| {
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel(window, cx);
                            })),
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
                            .disabled(is_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .when(self.on_saved.is_some(), |this| {
                        this.child(
                            Button::new("save-continue")
                                .small()
                                .outline()
                                .label(t!("ConnectionForm.save_and_continue").to_string())
                                .disabled(is_busy)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_save_and_continue(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .disabled(is_busy)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{
        AuthMethodSelection, build_connection_test_signature, build_jump_auth_method,
        format_connection_error, format_connection_error_for_platform, parse_os_release_id,
        validate_save_state,
    };
    use anyhow::Context as _;
    use one_core::storage::{SshAuthMethod, SshParams, StoredConnection};
    use rust_i18n::t;
    use std::sync::Arc;

    fn sample_params() -> SshParams {
        SshParams {
            host: "127.0.0.1".to_string(),
            port: 22,
            username: "root".to_string(),
            auth_method: SshAuthMethod::Agent,
            connect_timeout: Some(30),
            keepalive_interval: Some(60),
            keepalive_max: Some(3),
            default_directory: Some("/tmp".to_string()),
            init_script: Some("pwd".to_string()),
            disable_shell_integration: None,
            jump_server: None,
            proxy: None,
            os_id: None,
            icon: None,
        }
    }

    #[test]
    fn ssh_form_prefill_does_not_enter_edit_mode() {
        let initial_connection =
            StoredConnection::new_ssh("imported".to_string(), sample_params(), None);
        let config = super::SshFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(initial_connection),
            on_saved: None,
            workspaces: Vec::new(),
            teams: Vec::new(),
        };

        assert!(!config.is_editing());
        assert!(config.initial_connection.is_some());
    }

    #[test]
    fn ssh_form_prefill_can_enable_save_and_continue() {
        let initial_connection =
            StoredConnection::new_ssh("imported".to_string(), sample_params(), None);
        let config = super::SshFormWindowConfig {
            editing_connection: None,
            initial_connection: Some(initial_connection),
            on_saved: Some(Arc::new(|_, _, _, _| {})),
            workspaces: Vec::new(),
            teams: Vec::new(),
        };

        assert!(config.supports_save_and_continue());
        assert!(!config.is_editing());
    }

    #[test]
    fn connection_test_signature_changes_when_auth_related_fields_change() {
        let params = sample_params();
        let original = build_connection_test_signature(&params);

        let mut changed = sample_params();
        changed.auth_method = SshAuthMethod::AutoPublicKey;
        assert_ne!(original, build_connection_test_signature(&changed));

        let mut changed_host = sample_params();
        changed_host.host = "example.com".to_string();
        assert_ne!(original, build_connection_test_signature(&changed_host));
    }

    #[test]
    fn connection_test_signature_does_not_expose_private_key_content() {
        let mut params = sample_params();
        params.auth_method = SshAuthMethod::PrivateKeyContent {
            private_key: "-----BEGIN OPENSSH PRIVATE KEY-----\nsecret\n".to_string(),
            passphrase: Some("secret-passphrase".to_string()),
        };
        let signature = build_connection_test_signature(&params);

        assert!(!signature.contains("OPENSSH PRIVATE KEY"));
        assert!(!signature.contains("secret-passphrase"));
    }

    #[test]
    fn connection_test_error_keeps_context_chain() {
        let error = Err::<(), _>(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "denied",
        ))
        .context("SSH connection failed")
        .unwrap_err();
        let message = format_connection_error(&error, "example.com");

        assert!(message.contains("SSH connection failed"));
        assert!(message.contains("denied"));
    }

    #[test]
    fn macos_private_network_route_error_includes_recovery_hint() {
        let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::HostUnreachable));
        let detail = error.to_string();
        let message = format_connection_error_for_platform(&error, "192.168.9.19", true);

        assert!(message.contains(&detail));
        assert!(message.contains(t!("SSH.local_network_route_hint").as_ref()));
    }

    #[test]
    fn public_host_route_error_keeps_original_message() {
        let error = anyhow::Error::new(std::io::Error::from(std::io::ErrorKind::HostUnreachable));
        let detail = error.to_string();
        let message = format_connection_error_for_platform(&error, "203.0.113.10", true);

        assert_eq!(detail, message);
    }

    #[test]
    fn save_gate_allows_saving_without_successful_connection_test() {
        assert_eq!(validate_save_state(false, false), Ok(()));
    }

    #[test]
    fn os_release_id_parsing_handles_common_distros() {
        assert_eq!(
            Some("ubuntu".to_string()),
            parse_os_release_id("NAME=\"Ubuntu\"\nID=ubuntu\nID_LIKE=debian\n")
        );
        assert_eq!(
            Some("rhel".to_string()),
            parse_os_release_id("NAME=\"Red Hat Enterprise Linux\"\nID=\"rhel\"\n")
        );
        assert_eq!(
            Some("centos".to_string()),
            parse_os_release_id("NAME=\"CentOS Stream\"\nID=centos\nID_LIKE=\"rhel fedora\"\n")
        );
    }

    #[test]
    fn os_release_id_parsing_ignores_id_like_and_empty_values() {
        assert_eq!(None, parse_os_release_id("ID_LIKE=debian\n"));
        assert_eq!(None, parse_os_release_id("ID=\n"));
        assert_eq!(None, parse_os_release_id("Linux\n"));
    }

    #[test]
    fn save_gate_keeps_blocking_while_connection_test_is_running() {
        assert_eq!(validate_save_state(true, false), Err("testing"));
    }

    #[test]
    fn save_gate_keeps_blocking_while_shell_integration_uninstall_is_running() {
        assert_eq!(
            validate_save_state(false, true),
            Err("uninstalling_shell_integration")
        );
    }

    #[test]
    fn jump_auth_builder_supports_private_key() {
        let auth = build_jump_auth_method(
            AuthMethodSelection::PrivateKey,
            "ignored".to_string(),
            "/home/me/.ssh/bastion".to_string(),
            "ignored-key".to_string(),
            "secret".to_string(),
        );

        assert!(matches!(
            auth,
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase: Some(passphrase),
            } if key_path == "/home/me/.ssh/bastion" && passphrase == "secret"
        ));
    }

    #[test]
    fn jump_auth_builder_omits_empty_private_key_passphrase() {
        let auth = build_jump_auth_method(
            AuthMethodSelection::PrivateKey,
            "ignored".to_string(),
            "/home/me/.ssh/bastion".to_string(),
            "ignored-key".to_string(),
            String::new(),
        );

        assert!(matches!(
            auth,
            SshAuthMethod::PrivateKey {
                key_path,
                passphrase: None,
            } if key_path == "/home/me/.ssh/bastion"
        ));
    }

    #[test]
    fn jump_auth_builder_supports_private_key_content() {
        let auth = build_jump_auth_method(
            AuthMethodSelection::PrivateKeyContent,
            "ignored".to_string(),
            "ignored-path".to_string(),
            "-----BEGIN OPENSSH PRIVATE KEY-----\nfixture\n".to_string(),
            "secret".to_string(),
        );

        assert!(matches!(
            auth,
            SshAuthMethod::PrivateKeyContent {
                private_key,
                passphrase: Some(passphrase),
            } if private_key.contains("OPENSSH PRIVATE KEY") && passphrase == "secret"
        ));
    }
}
