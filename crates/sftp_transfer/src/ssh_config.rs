//! 已保存连接 → SSH 连接配置的解析。
//!
//! 终端侧边栏的远端文件面板需要在会话中途把浏览目标切到另一台已保存主机，
//! 因此必须能在 `terminal_view` 侧独立构造 [`SshConnectConfig`]；SFTP 双栏视图
//! 也需要同一套规则（SFTP 专用账号覆盖、跳板机、代理、初始目录）。
//!
//! 放在 `sftp_transfer` 而不是各自的视图 crate：`terminal_view` 与 `sftp_view`
//! 都依赖本 crate，而 `ssh` 不应反向依赖 `one_core::storage`，`sftp_view`
//! 对 `terminal_view` 也不可见。

use std::time::Duration;

use anyhow::{Context as _, Result};
use one_core::storage::models::{
    BastionUserFormat as StorageBastionUserFormat, JumpMode as StorageJumpMode,
    JumpServerConfig as StorageJumpServerConfig, ProxyConfig as StorageProxyConfig,
    ProxyType as StorageProxyType, SshAuthMethod, SshParams, StoredConnection,
};
use ssh::{
    AgentIdentity, BastionUserFormat, HostKeyVerifier, JumpMode, JumpServerConnectConfig,
    ProxyConnectConfig, ProxyType, SshAuth, SshConnectConfig,
};

/// 已保存连接解析出的 SSH 目标。
#[derive(Clone)]
pub struct ResolvedSshTarget {
    /// 用于建立独立会话的连接参数。
    pub config: SshConnectConfig,
    /// 连接成功后进入的初始目录；`None` 时由服务器登录目录决定。
    pub sftp_initial_directory: Option<String>,
}

/// 解析连接的 SSH 目标；非 SSH 连接或参数无效时返回错误。
///
/// SFTP 专用账号（`sftp_account`）非空时覆盖 SSH 顶层凭据，与双栏视图
/// 的既有行为保持一致。
pub fn resolve_ssh_target(connection: &StoredConnection) -> Result<ResolvedSshTarget> {
    let params = connection
        .to_ssh_params()
        .context("connection does not contain valid SSH parameters")?;
    let initial_directory = sftp_initial_directory(&params);
    let uses_sftp_account = params
        .sftp_account
        .as_ref()
        .is_some_and(|account| !account.username.trim().is_empty() || !account.password.is_empty());
    let agent_identities = if uses_sftp_account || !params.forward_agent.unwrap_or(false) {
        Vec::new()
    } else {
        agent_identities_from_auth(&params.auth_method)
    };
    let (username, auth) = if uses_sftp_account {
        let account = params.sftp_account.as_ref().expect("guarded by flag");
        (
            account.username.clone(),
            SshAuth::Password(account.password.clone()),
        )
    } else {
        (params.username.clone(), ssh_auth(params.auth_method))
    };
    let config = SshConnectConfig {
        host: params.host,
        port: params.port,
        username,
        auth,
        timeout: params.connect_timeout.map(Duration::from_secs),
        keepalive_interval: params.keepalive_interval.map(Duration::from_secs),
        keepalive_max: params.keepalive_max,
        jump_server: params.jump_server.as_ref().map(jump_connect_config),
        proxy: params.proxy.as_ref().map(proxy_connect_config),
        proxy_command: params.proxy_command.clone(),
        keyboard_interactive_responder: None,
        host_key_verifier: HostKeyVerifier::default(),
        x11_forwarding: false,
        forward_agent: params.forward_agent.unwrap_or(false),
        agent_identities,
        allow_legacy_algorithms: params.allow_legacy_algorithms.unwrap_or(false),
    };
    Ok(ResolvedSshTarget {
        config,
        sftp_initial_directory: initial_directory,
    })
}

/// 解析连接的 SSH 连接参数。
pub fn ssh_config_for(connection: &StoredConnection) -> Result<SshConnectConfig> {
    Ok(resolve_ssh_target(connection)?.config)
}

/// 从 SSH 参数中提取 SFTP 初始目录，空白值视为未配置。
pub fn sftp_initial_directory(params: &SshParams) -> Option<String> {
    params
        .sftp_default_directory
        .as_ref()
        .map(|dir| dir.trim().to_string())
        .filter(|dir| !dir.is_empty())
}

/// 从存储连接中提取 SFTP 初始目录；非 SSH 连接返回 `None`。
pub fn sftp_initial_directory_of(connection: &StoredConnection) -> Option<String> {
    connection
        .to_ssh_params()
        .ok()
        .and_then(|params| sftp_initial_directory(&params))
}

/// 存储的认证方式 → 运行时认证方式。
pub fn ssh_auth(method: SshAuthMethod) -> SshAuth {
    match method {
        SshAuthMethod::Password { password } => SshAuth::Password(password),
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => SshAuth::PrivateKey {
            key_path,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => SshAuth::PrivateKeyContent {
            private_key,
            passphrase,
            certificate_path: None,
        },
        SshAuthMethod::Agent => SshAuth::Agent,
        SshAuthMethod::Pageant => SshAuth::Pageant,
        SshAuthMethod::AutoPublicKey => SshAuth::AutoPublicKey,
        SshAuthMethod::None => SshAuth::None,
        SshAuthMethod::Chain(steps) => SshAuth::Chain(steps.into_iter().map(ssh_auth).collect()),
    }
}

/// 存储的跳板机配置 → 连接层跳板机配置（统一映射入口）。
///
/// 历史上 terminal / runtime_sftp / port_forwarding / ssh_form 各自维护一份
/// 等价映射，bastion 字段加入后必须五处同步；统一到本函数后新增字段只需
/// 改这里，各构造点显式传入 MFA 响应器即可。
pub fn jump_connect_config(jump: &StorageJumpServerConfig) -> JumpServerConnectConfig {
    JumpServerConnectConfig {
        host: jump.host.clone(),
        port: jump.port,
        username: jump.username.clone(),
        auth: ssh_auth(jump.auth_method.clone()),
        mode: match jump.mode {
            StorageJumpMode::ProxyJump => JumpMode::ProxyJump,
            StorageJumpMode::BastionDirect => JumpMode::BastionDirect,
        },
        bastion_asset: jump
            .bastion
            .as_ref()
            .map(|target| target.asset.clone())
            .unwrap_or_default(),
        bastion_account: jump
            .bastion
            .as_ref()
            .map(|target| target.account.clone())
            .unwrap_or_default(),
        bastion_user_format: jump
            .bastion
            .as_ref()
            .map(|target| match target.user_format {
                StorageBastionUserFormat::JumpServer => BastionUserFormat::JumpServer,
            })
            .unwrap_or_default(),
    }
}

/// 存储的代理配置 → 连接层代理配置。
pub fn proxy_connect_config(proxy: &StorageProxyConfig) -> ProxyConnectConfig {
    ProxyConnectConfig {
        proxy_type: match proxy.proxy_type {
            StorageProxyType::Socks5 => ProxyType::Socks5,
            StorageProxyType::Http => ProxyType::Http,
        },
        host: proxy.host.clone(),
        port: proxy.port,
        username: proxy.username.clone(),
        password: proxy.password.clone(),
    }
}

/// 从存储 SSH 认证方式推导要加入本地 ssh-agent 的私钥身份。
///
/// 仅当连接开启了 ForwardAgent 时才会被加载进 agent 并转发给远端。
fn agent_identities_from_auth(auth: &SshAuthMethod) -> Vec<AgentIdentity> {
    match auth {
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => vec![AgentIdentity::PrivateKeyPath {
            key_path: key_path.clone(),
            passphrase: passphrase.clone(),
        }],
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => vec![AgentIdentity::PrivateKeyContent {
            private_key: private_key.clone(),
            passphrase: passphrase.clone(),
        }],
        SshAuthMethod::Chain(steps) => steps.iter().flat_map(agent_identities_from_auth).collect(),
        SshAuthMethod::Password { .. }
        | SshAuthMethod::Agent
        | SshAuthMethod::Pageant
        | SshAuthMethod::AutoPublicKey
        | SshAuthMethod::None => Vec::new(),
    }
}
