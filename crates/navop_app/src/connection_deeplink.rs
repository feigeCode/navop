//! 通用连接深链接：`ssh://` 与 `sftp://`。
//!
//! 链接来自浏览器、文档或其它应用，例如：
//!
//! ```text
//! ssh://alice@example.com:2222
//! sftp://alice@example.com/var/log
//! ```
//!
//! 解析结果只会变成一条**临时连接**：不写入连接仓库、不参与云同步、不保存
//! 密码；URL 里的密码直接用于本次认证，但不会出现在日志、通知或错误提示里。
//!
//! 这类 scheme 会与系统里既有的终端、文件管理器争抢默认关联，所以默认关闭，
//! 只有用户在设置里显式启用后才会注册（见 `AppSettings::enabled_deeplink_schemes`）。

use anyhow::{Context as _, Result, bail};
use gpui::{App, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use one_core::settings::AppSettings;
use one_core::storage::{
    ConnectionType, PreferredOpenMode, SshAuthMethod, SshParams, StoredConnection,
    StoredTerminalEncoding, StoredTerminalType,
};
use one_core::tab_container::TabOpenMode;
use percent_encoding::percent_decode_str;
use rust_i18n::t;
use url::{Host, Url};

use crate::navop_app::GlobalHomePage;

/// 除 `jms://` 外，Navop 能直接接管的连接类 scheme。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ConnectionScheme {
    Ssh,
    Sftp,
}

impl ConnectionScheme {
    pub(crate) fn parse(scheme: &str) -> Option<Self> {
        match scheme.to_ascii_lowercase().as_str() {
            "ssh" => Some(Self::Ssh),
            "sftp" => Some(Self::Sftp),
            _ => None,
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ssh => "ssh",
            Self::Sftp => "sftp",
        }
    }

    fn connection_type(self) -> ConnectionType {
        ConnectionType::SshSftp
    }

    fn default_port(self) -> u16 {
        22
    }
}

/// 识别 `ssh://` / `sftp://` 链接；其它内容返回 `None`。
pub(crate) fn scheme_of(raw: &str) -> Option<ConnectionScheme> {
    let trimmed = raw.trim();
    let (scheme, rest) = trimmed.split_once("://")?;
    if rest.is_empty() {
        return None;
    }
    ConnectionScheme::parse(scheme)
}

/// 解析后的连接意图（不含需要落库的状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ConnectionLink {
    pub scheme: ConnectionScheme,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: Option<String>,
    /// 仅 `sftp://` 使用；已规范化，拒绝 `..` 之类的越界路径。
    pub directory: Option<String>,
}

/// 把深链接转成一条不落库的临时连接。
pub(crate) fn connection_from_link(link: &ConnectionLink) -> StoredConnection {
    let params = SshParams {
        host: link.host.clone(),
        port: link.port,
        username: link.username.clone(),
        auth_method: SshAuthMethod::Password {
            password: link.password.clone().unwrap_or_default(),
        },
        sftp_account: None,
        sftp_default_directory: link.directory.clone(),
        credential_reference: None,
        prompt_username: Some(link.username.is_empty()),
        prompt_password: Some(link.password.is_none()),
        // 链接里带密码时认证方式固定为 password，避免密码错误后再弹键盘交互；
        // 不带密码时交给宿主按需提示。
        keyboard_interactive: link.password.as_ref().map(|_| false),
        otp: None,
        terminal_encoding: StoredTerminalEncoding::default(),
        terminal_type: StoredTerminalType::default(),
        account_expect: Default::default(),
        connect_timeout: None,
        keepalive_interval: None,
        keepalive_max: None,
        default_directory: None,
        init_script: None,
        disable_shell_integration: None,
        x11_forwarding: None,
        allow_legacy_algorithms: None,
        forward_agent: None,
        agent_forward_key: None,
        jump_server: None,
        disabled_jump_server: None,
        proxy: None,
        os_id: None,
        icon: None,
        icon_file_path: None,
        remote_file: None,
    };
    let mut connection = StoredConnection::new_ssh(String::new(), params, None);
    // 临时会话不参与云同步；id 为 None，打开后自动成为一次性标签页。
    connection.sync_enabled = false;
    if link.scheme == ConnectionScheme::Sftp {
        connection.preferred_open_mode = Some(PreferredOpenMode::DualPane);
    }
    connection
}

/// 解析 `ssh://user[:password]@host[:port]` 与
/// `sftp://user[:password]@host[:port][/path]`。
///
/// 错误信息只描述问题，不回显原始 URL（链接里可能带密码）。
pub(crate) fn parse_connection_deeplink(
    raw: &str,
    scheme: ConnectionScheme,
) -> Result<ConnectionLink> {
    let url = Url::parse(raw.trim()).context("链接格式不合法")?;
    if url.scheme() != scheme.as_str() {
        bail!("链接协议与预期不一致");
    }
    let host = match url.host() {
        Some(Host::Ipv4(address)) => address.to_string(),
        Some(Host::Ipv6(address)) => address.to_string(),
        Some(Host::Domain(domain)) => domain.to_string(),
        None => bail!("链接缺少主机名"),
    };
    if host.trim().is_empty() {
        bail!("链接缺少主机名");
    }
    let port = match url.port() {
        Some(0) => bail!("端口不合法"),
        Some(port) => port,
        None => scheme.default_port(),
    };

    let username = decode(url.username()).trim().to_string();
    let password = url
        .password()
        .map(decode)
        .map(|password| password.to_string())
        .filter(|password| !password.is_empty());

    let directory = match scheme {
        ConnectionScheme::Sftp => sanitize_directory(url.path())?,
        ConnectionScheme::Ssh => None,
    };

    Ok(ConnectionLink {
        scheme,
        host,
        port,
        username,
        password,
        directory,
    })
}

fn decode(raw: &str) -> String {
    percent_decode_str(raw).decode_utf8_lossy().to_string()
}

/// `sftp://host/var/log` 的 path 只能作为初始目录；拒绝任何 `..` 越界。
fn sanitize_directory(path: &str) -> Result<Option<String>> {
    if path.is_empty() || path == "/" {
        return Ok(None);
    }
    let decoded = decode(path);
    let mut segments = Vec::new();
    for segment in decoded.split('/') {
        match segment {
            "" | "." => continue,
            ".." => bail!("路径不合法"),
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return Ok(None);
    }
    Ok(Some(format!("/{}", segments.join("/"))))
}

struct ConnectionDeepLinkNotification;

/// 打开 `ssh://` / `sftp://` 深链接；失败只提示「无法打开」，不回显链接内容。
pub(crate) fn open_connection_deeplink(raw: &str, window: &mut Window, cx: &mut App) {
    let Some(scheme) = scheme_of(raw) else {
        return;
    };
    if !AppSettings::global(cx).deeplink_scheme_enabled(scheme.as_str()) {
        window.push_notification(
            Notification::warning(
                t!("Common.deeplink_scheme_disabled", scheme = scheme.as_str()).to_string(),
            )
            .id::<ConnectionDeepLinkNotification>()
            .autohide(true),
            cx,
        );
        return;
    }
    if let Err(error) = try_open_connection_deeplink(raw, scheme, window, cx) {
        tracing::warn!(error = %error, scheme = scheme.as_str(), "failed to open deep link");
        window.push_notification(
            Notification::error(
                t!(
                    "Common.connection_deeplink_failed",
                    error = format!("{error:#}")
                )
                .to_string(),
            )
            .id::<ConnectionDeepLinkNotification>()
            .autohide(true),
            cx,
        );
    }
}

fn try_open_connection_deeplink(
    raw: &str,
    scheme: ConnectionScheme,
    window: &mut Window,
    cx: &mut App,
) -> Result<()> {
    let link = parse_connection_deeplink(raw, scheme)?;
    let connection = connection_from_link(&link);
    tracing::info!(
        host = %link.host,
        port = link.port,
        scheme = scheme.as_str(),
        "opening connection deep link"
    );
    let home = cx
        .try_global::<GlobalHomePage>()
        .map(|global| global.home_page.clone())
        .context("home page is not ready")?;
    window.activate_window();
    window.defer(cx, move |window, cx| {
        home.update(cx, |home, cx| {
            if !app_kit::registry().open_connection(
                &connection,
                TabOpenMode::Activate,
                home,
                window,
                cx,
            ) {
                window.push_notification(
                    Notification::warning(
                        t!(
                            "Common.connection_type_unsupported",
                            name = connection.name.clone()
                        )
                        .to_string(),
                    )
                    .autohide(true),
                    cx,
                );
            }
        });
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<ConnectionLink> {
        let scheme = scheme_of(raw).expect("应识别 scheme");
        parse_connection_deeplink(raw, scheme)
    }

    #[test]
    fn recognizes_ssh_and_sftp_links_case_insensitively() {
        assert_eq!(Some(ConnectionScheme::Ssh), scheme_of("ssh://host"));
        assert_eq!(Some(ConnectionScheme::Sftp), scheme_of("SFTP://host"));
        assert_eq!(Some(ConnectionScheme::Ssh), scheme_of("  Ssh://host  "));
        assert_eq!(None, scheme_of("jms://abc"));
        assert_eq!(None, scheme_of("http://host"));
        assert_eq!(None, scheme_of("ssh://"));
        assert_eq!(None, scheme_of(""));
    }

    #[test]
    fn parses_ssh_userinfo_and_default_port() {
        let link = parse("ssh://alice@example.com").expect("应解析成功");
        assert_eq!(link.scheme, ConnectionScheme::Ssh);
        assert_eq!(link.host, "example.com");
        assert_eq!(link.port, 22);
        assert_eq!(link.username, "alice");
        assert_eq!(link.password, None);
        assert_eq!(link.directory, None);
    }

    #[test]
    fn parses_explicit_port_and_percent_decoded_password() {
        let link = parse("ssh://alice:p%40ss%20word@example.com:2222").expect("应解析成功");
        assert_eq!(link.port, 2222);
        assert_eq!(link.password.as_deref(), Some("p@ss word"));
    }

    #[test]
    fn sftp_uses_path_as_initial_directory() {
        let link = parse("sftp://alice@example.com:22/var/log").expect("应解析成功");
        assert_eq!(link.directory.as_deref(), Some("/var/log"));

        let root = parse("sftp://example.com/").expect("应解析成功");
        assert_eq!(root.directory, None);
    }

    #[test]
    fn sftp_directory_traversal_is_normalized_before_use() {
        // url crate 在解析阶段就把 `..` 与 `%2e%2e` 规范化掉，不会把越界路径交给
        // 会话；解析结果至少不会含 `..` 段。
        for raw in [
            "sftp://example.com/var/../etc",
            "sftp://example.com/var/%2e%2e/etc",
        ] {
            let directory = parse(raw)
                .expect("规范化后的路径仍可解析")
                .directory
                .expect("应得到初始目录");
            assert!(!directory.split('/').any(|segment| segment == ".."));
            assert_eq!(directory, "/etc");
        }
    }

    #[test]
    fn rejects_missing_host_and_mismatched_scheme() {
        assert!(parse_connection_deeplink("ssh:///var/log", ConnectionScheme::Ssh).is_err());
        assert!(parse_connection_deeplink("sftp://example.com", ConnectionScheme::Ssh).is_err());
    }

    #[test]
    fn error_messages_do_not_echo_credentials() {
        let error = parse("ssh://alice:secret@").expect_err("应拒绝");
        let rendered = format!("{error:#}");
        assert!(!rendered.contains("secret"));
    }

    #[test]
    fn builds_ephemeral_connection_with_prompt_for_missing_credentials() {
        let link = parse("ssh://alice@example.com:2222").expect("应解析成功");
        let connection = connection_from_link(&link);
        assert_eq!(connection.id, None);
        assert!(!connection.sync_enabled);
        assert_eq!(connection.connection_type, ConnectionType::SshSftp);
        assert_eq!(connection.name, "alice@example.com:2222");

        let params: SshParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 SshParams");
        assert_eq!(params.host, "example.com");
        assert_eq!(params.port, 2222);
        assert_eq!(params.username, "alice");
        assert_eq!(params.prompt_password, Some(true));
        assert!(params.prompts_for_password());
        assert!(!params.prompts_for_username());
        assert_eq!(params.keyboard_interactive, None);
    }

    #[test]
    fn password_links_disable_interactive_fallback() {
        let link = parse("ssh://alice:secret@example.com").expect("应解析成功");
        let connection = connection_from_link(&link);
        let params: SshParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 SshParams");
        assert_eq!(params.prompt_password, Some(false));
        assert_eq!(params.keyboard_interactive, Some(false));
        match &params.auth_method {
            SshAuthMethod::Password { password } => assert_eq!(password, "secret"),
            other => panic!("应使用密码认证: {other:?}"),
        }
    }

    #[test]
    fn anonymous_links_prompt_for_username() {
        let link = parse("ssh://example.com").expect("应解析成功");
        let connection = connection_from_link(&link);
        let params: SshParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 SshParams");
        assert!(params.prompts_for_username());
        assert!(connection.name.starts_with("example.com"));
    }

    #[test]
    fn sftp_links_open_dual_pane_with_directory() {
        let link = parse("sftp://alice@example.com/var/log").expect("应解析成功");
        let connection = connection_from_link(&link);
        assert_eq!(
            connection.preferred_open_mode,
            Some(PreferredOpenMode::DualPane)
        );
        let params: SshParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 SshParams");
        assert_eq!(params.sftp_default_directory.as_deref(), Some("/var/log"));
    }

    #[test]
    fn parses_ipv6_hosts() {
        let link = parse("ssh://alice@[::1]:2222").expect("应解析成功");
        assert_eq!(link.host, "::1");
        assert_eq!(link.port, 2222);
    }
}
