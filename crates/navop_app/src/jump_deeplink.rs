//! JumpServer `jms://` 深链接：把网页端签发的一次性 connection token 变成
//! 一条临时 SSH / RDP 会话。
//!
//! 链接形如 `jms://<base64url(JSON)>`，payload 由 JumpServer 前端生成：
//!
//! ```json
//! {
//!   "protocol": "ssh",
//!   "name": "account@asset",
//!   "token": { "id": "token-id", "value": "token-secret" },
//!   "asset": { "name": "Production Server" },
//!   "endpoint": { "host": "gateway.example.com", "port": 2222 }
//! }
//! ```
//!
//! SSH 打开方式与官方客户端一致：直接以 `JMS-<token-id>` / `<token-value>`
//! SSH 到 `endpoint`，由网关上的 Koko 用 token 桥接到目标资产。
//!
//! RDP 走原生客户端（Razor）时 payload 里没有 `endpoint`，网关地址和登录用户名
//! 都在 `.rdp` 文件内容里：
//!
//! ```json
//! {
//!   "protocol": "rdp",
//!   "token": { "id": "token-id", "value": "token-secret" },
//!   "file": { "name": "user-Asset.rdp", "content": "full address:s:gw:3389\nusername:s:user|token-id\n" }
//! }
//! ```
//!
//! 两种链接都不写入连接仓库，也不保存密码；链接本身不会出现在日志或错误提示里。

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use gpui::{App, AppContext as _, Entity, ParentElement as _, Styled as _, Window, div, px};
use gpui_component::{
    ActiveTheme as _, WindowExt as _,
    dialog::DialogButtonProps,
    input::{Input, InputState},
    notification::Notification,
    v_flex,
};
use one_core::settings::AppSettings;
use one_core::storage::{
    PreferredOpenMode, RemoteDesktopBackendPreference, RemoteDesktopParams, RemoteDesktopProtocol,
    SshAuthMethod, SshParams, StoredConnection, StoredTerminalEncoding, StoredTerminalType,
};
use one_core::tab_container::TabOpenMode;
use rust_i18n::t;
use serde_json::Value;

use crate::home_tab::HomePage;
use crate::navop_app::GlobalHomePage;

/// JumpServer 深链接协议前缀（大小写不敏感）。
pub(crate) const JUMP_DEEPLINK_PREFIX: &str = "jms://";

/// RDP 默认端口：`.rdp` 的 `full address` 没写端口时使用。
const DEFAULT_RDP_PORT: u16 = 3389;

/// 深链接里声明的协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JumpProtocol {
    Ssh,
    Sftp,
    Telnet,
    /// Windows 资产的 RDP 会话（JumpServer 原生客户端走 Razor 网关）。
    Rdp,
}

impl JumpProtocol {
    fn from_payload(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ssh" => Ok(Self::Ssh),
            "sftp" => Ok(Self::Sftp),
            "telnet" => Ok(Self::Telnet),
            "rdp" => Ok(Self::Rdp),
            other => bail!("不支持的 JumpServer 协议: {other}"),
        }
    }

    /// RDP 需要在打开前向用户索要密码（JumpServer 的 `.rdp` 里不含密码）。
    fn needs_password_prompt(self) -> bool {
        self == Self::Rdp
    }
}

/// 解析后的深链接内容（不含任何需要落库的状态）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JumpLink {
    pub protocol: JumpProtocol,
    pub label: String,
    pub host: String,
    pub port: u16,
    pub token_id: String,
    pub token_value: String,
    /// RDP 链接的 `username:s`，JumpServer 写的是 `<登录用户名>|<token-id>`。
    pub rdp_username: Option<String>,
}

/// 判断字符串是否是 JumpServer 深链接。
pub(crate) fn is_jump_deeplink(raw: &str) -> bool {
    let trimmed = raw.trim();
    trimmed
        .get(..JUMP_DEEPLINK_PREFIX.len())
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case(JUMP_DEEPLINK_PREFIX))
}

/// 解析 `jms://` 深链接。
pub(crate) fn parse_jump_deeplink(raw: &str) -> Result<JumpLink> {
    let trimmed = raw.trim();
    if !is_jump_deeplink(trimmed) {
        bail!("不是 jms:// 深链接");
    }
    // 去掉协议前缀与 JumpServer 偶尔追加的尾部 `/`。
    let encoded = trimmed[JUMP_DEEPLINK_PREFIX.len()..].trim_end_matches('/');
    if encoded.is_empty() {
        bail!("jms:// 链接缺少 payload");
    }
    let payload = decode_payload(encoded)?;

    let protocol = payload
        .get("protocol")
        .and_then(string_value)
        .context("jms:// payload 缺少 protocol")?;
    let protocol = JumpProtocol::from_payload(&protocol)?;

    // 新版 payload 把凭据放在 token 里，旧版（v1）直接放在顶层。
    let token = payload.get("token");
    let token_id = token
        .and_then(|token| token.get("id"))
        .and_then(string_value)
        .or_else(|| payload.get("id").and_then(string_value))
        .context("jms:// payload 缺少 token id")?;
    let token_value = token
        .and_then(|token| token.get("value"))
        .and_then(string_value)
        .or_else(|| payload.get("value").and_then(string_value))
        .context("jms:// payload 缺少 token value")?;

    let asset_name = payload
        .pointer("/asset/name")
        .and_then(string_value)
        .filter(|name| !name.is_empty());
    let name = payload
        .get("name")
        .and_then(string_value)
        .filter(|name| !name.is_empty());
    let file_name = payload
        .pointer("/file/name")
        .and_then(string_value)
        .filter(|name| !name.is_empty());

    let (host, port, rdp_username) = if protocol == JumpProtocol::Rdp {
        // 原生客户端（Razor）的 RDP payload 只有 `.rdp` 文件内容：网关地址写在
        // `full address:s`，登录用户名写在 `username:s`（`<用户>|<token-id>`）。
        // 少数部署也会直接给 `endpoint`，此时退化到与 SSH 相同的取法。
        let content = payload.pointer("/file/content").and_then(string_value);
        let rdp_username = content
            .as_deref()
            .and_then(|content| rdp_option(content, "username"));
        let address = content
            .as_deref()
            .and_then(|content| rdp_option(content, "full address"));
        let (host, port) = match address {
            Some(address) => split_rdp_address(&address)
                .with_context(|| format!("jms:// RDP 链接的网关地址无效: {address}"))?,
            None => {
                let host = payload
                    .pointer("/endpoint/host")
                    .and_then(string_value)
                    .context("jms:// RDP 链接缺少 .rdp 文件内容和 endpoint")?;
                let port = payload
                    .pointer("/endpoint/port")
                    .and_then(parse_port)
                    .unwrap_or(DEFAULT_RDP_PORT);
                (host, port)
            }
        };
        (host, port, rdp_username)
    } else {
        let host = payload
            .pointer("/endpoint/host")
            .and_then(string_value)
            .context("jms:// payload 缺少 endpoint host")?;
        let port = payload
            .pointer("/endpoint/port")
            .and_then(parse_port)
            .context("jms:// payload 的 endpoint port 无效")?;
        (host, port, None)
    };

    let label = asset_name
        .or(name)
        .or_else(|| {
            file_name.map(|name| {
                name.strip_suffix(".rdp")
                    .or_else(|| name.strip_suffix(".RDP"))
                    .unwrap_or(&name)
                    .to_string()
            })
        })
        .unwrap_or_else(|| host.clone());

    Ok(JumpLink {
        protocol,
        label,
        host,
        port,
        token_id,
        token_value,
        rdp_username,
    })
}

/// 从 `.rdp` 文本里取字符串键：JumpServer 逐行写成 `key:s:value`。
fn rdp_option(content: &str, key: &str) -> Option<String> {
    content.lines().find_map(|line| {
        let value = line.trim().strip_prefix(key)?.strip_prefix(":s:")?.trim();
        (!value.is_empty()).then(|| value.to_string())
    })
}

/// 解析 `.rdp` 的 `full address`：`host:port`，没写端口时按 RDP 默认端口算。
fn split_rdp_address(address: &str) -> Option<(String, u16)> {
    let address = address.trim();
    match address.rsplit_once(':') {
        Some((host, port)) => {
            let port = port.trim().parse::<u16>().ok().filter(|port| *port != 0)?;
            let host = host.trim();
            (!host.is_empty()).then(|| (host.to_string(), port))
        }
        None => (!address.is_empty()).then(|| (address.to_string(), DEFAULT_RDP_PORT)),
    }
}

fn decode_payload(encoded: &str) -> Result<Value> {
    // JumpServer 有时发 URL-safe 字符集、省略 padding，统一归一化后再解码。
    let mut normalized = encoded.replace('-', "+").replace('_', "/");
    while normalized.len() % 4 != 0 {
        normalized.push('=');
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(normalized.as_bytes())
        .context("jms:// payload 不是合法的 base64url")?;
    let text = String::from_utf8(bytes).context("jms:// payload 不是 UTF-8 文本")?;
    serde_json::from_str(&text).context("jms:// payload 不是合法的 JSON")
}

fn string_value(value: &Value) -> Option<String> {
    let raw = value.as_str()?.trim();
    if raw.is_empty() {
        None
    } else {
        Some(raw.to_string())
    }
}

fn parse_port(value: &Value) -> Option<u16> {
    let port = match value {
        Value::Number(number) => number.as_u64()?,
        Value::String(text) => text.trim().parse::<u64>().ok()?,
        _ => return None,
    };
    u16::try_from(port).ok().filter(|port| *port != 0)
}

/// 把深链接转成一条不落库的连接。
///
/// SSH/SFTP/Telnet 用 `JMS-<token-id>` / `<token-value>` 交给网关上的 Koko 认证；
/// RDP 连的是 JumpServer 网关本身，`password` 由调用方从对话框取（链接里没有）。
pub(crate) fn connection_from_link(link: &JumpLink, password: Option<String>) -> StoredConnection {
    if link.protocol == JumpProtocol::Rdp {
        rdp_connection_from_link(link, password)
    } else {
        ssh_connection_from_link(link)
    }
}

/// RDP：网关地址 + `.rdp` 里的登录用户名，密码留空时按空密码走 CredSSP。
fn rdp_connection_from_link(link: &JumpLink, password: Option<String>) -> StoredConnection {
    let params = RemoteDesktopParams {
        protocol: RemoteDesktopProtocol::Rdp,
        host: link.host.clone(),
        port: link.port,
        // JumpServer 的 RDP 用户名是 `<登录用户>|<token-id>`；payload 里没有时
        // 退化到官方客户端的 `JMS-<token-id>` 写法，让网关自己给出提示。
        username: link
            .rdp_username
            .clone()
            .or_else(|| Some(format!("JMS-{}", link.token_id))),
        password,
        credential_reference: None,
        domain: None,
        read_only: false,
        audio_playback: false,
        proxy: None,
        backend_preference: RemoteDesktopBackendPreference::default(),
        rdp: None,
    };
    let mut connection = StoredConnection::new_remote_desktop(link.label.clone(), params, None);
    // 临时会话不参与云同步；id 为 None，打开后自动成为一次性标签页。
    connection.sync_enabled = false;
    connection
}

/// SSH 系：用户名 `JMS-<token-id>`、密码为 token 值，认证由 Koko 完成。
fn ssh_connection_from_link(link: &JumpLink) -> StoredConnection {
    let params = SshParams {
        host: link.host.clone(),
        port: link.port,
        username: format!("JMS-{}", link.token_id),
        auth_method: SshAuthMethod::Password {
            password: link.token_value.clone(),
        },
        sftp_account: None,
        sftp_default_directory: None,
        credential_reference: None,
        prompt_username: Some(false),
        prompt_password: Some(false),
        // token 就是密码，认证方式固定为 password，避免网关拒绝后再弹键盘交互。
        keyboard_interactive: Some(false),
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
        proxy_command: None,
    };
    let mut connection = StoredConnection::new_ssh(link.label.clone(), params, None);
    // 临时会话不参与云同步；id 为 None，打开后自动成为一次性标签页。
    connection.sync_enabled = false;
    if link.protocol == JumpProtocol::Sftp {
        connection.preferred_open_mode = Some(PreferredOpenMode::DualPane);
    }
    connection
}

struct JumpDeepLinkNotification;

/// 打开 `jms://` 深链接；任何失败都只提示「无法打开」，不回显链接内容。
pub(crate) fn open_jump_deeplink(raw: &str, window: &mut Window, cx: &mut App) {
    // 开关关掉后既不再注册系统关联，也在这里拒绝已被系统送进来的链接。
    if !AppSettings::global(cx).deeplink_scheme_enabled(AppSettings::PRIVATE_DEEPLINK_SCHEME) {
        window.push_notification(
            Notification::warning(
                t!(
                    "Common.deeplink_scheme_disabled",
                    scheme = AppSettings::PRIVATE_DEEPLINK_SCHEME
                )
                .to_string(),
            )
            .id::<JumpDeepLinkNotification>()
            .autohide(true),
            cx,
        );
        return;
    }
    if let Err(error) = try_open_jump_deeplink(raw, window, cx) {
        tracing::warn!(error = %error, "failed to open JumpServer deep link");
        window.push_notification(
            Notification::error(
                t!("Common.jump_deeplink_failed", error = format!("{error:#}")).to_string(),
            )
            .id::<JumpDeepLinkNotification>()
            .autohide(true),
            cx,
        );
    }
}

fn try_open_jump_deeplink(raw: &str, window: &mut Window, cx: &mut App) -> Result<()> {
    let link = parse_jump_deeplink(raw)?;
    tracing::info!(
        host = %link.host,
        port = link.port,
        protocol = ?link.protocol,
        "opening JumpServer deep link"
    );
    let home = cx
        .try_global::<GlobalHomePage>()
        .map(|global| global.home_page.clone())
        .context("home page is not ready")?;
    window.activate_window();
    if link.protocol.needs_password_prompt() {
        // JumpServer 的 RDP 链接里没有密码（官方客户端同样由用户自己填），
        // 先问一次再开会话；留空按空密码连接，交给网关去校验。
        prompt_rdp_password(link, home, window, cx);
        return Ok(());
    }
    open_link_connection(connection_from_link(&link, None), home, window, cx);
    Ok(())
}

/// 弹「RDP 密码」对话框，确认后打开会话。
fn prompt_rdp_password(link: JumpLink, home: Entity<HomePage>, window: &mut Window, cx: &mut App) {
    let name = link.label.clone();
    let address = format!("{}:{}", link.host, link.port);
    tracing::info!(
        host = %link.host,
        port = link.port,
        "asking for the JumpServer RDP password"
    );
    let password_input = cx.new(|cx| InputState::new(window, cx).masked(true));
    let input_for_ok = password_input.clone();
    window.open_alert_dialog(cx, move |alert, _window, _cx| {
        alert
            .title(t!("Common.jump_rdp_password_title", name = name.clone()).to_string())
            .description(
                t!(
                    "Common.jump_rdp_password_description",
                    address = address.clone()
                )
                .to_string(),
            )
            .confirm()
            .width(px(420.))
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Common.jump_rdp_connect").to_string())
                    .cancel_text(t!("Common.cancel").to_string())
                    .show_cancel(true),
            )
            .content({
                let password_input = password_input.clone();
                move |content, _window, cx| {
                    content.child(
                        v_flex().gap_2().child(Input::new(&password_input)).child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Common.jump_rdp_password_hint").to_string()),
                        ),
                    )
                }
            })
            .on_ok({
                // 对话框的 build 闭包是 `Fn`（每次重绘都会调用），所以这里每次
                // 都重新克隆一份，不能把捕获的值移出去。
                let link = link.clone();
                let home = home.clone();
                let input_for_ok = input_for_ok.clone();
                move |_, window, cx| {
                    let password = input_for_ok.read(cx).value().to_string();
                    let password = (!password.is_empty()).then_some(password);
                    open_link_connection(
                        connection_from_link(&link, password),
                        home.clone(),
                        window,
                        cx,
                    );
                    true
                }
            })
    });
}

fn open_link_connection(
    connection: StoredConnection,
    home: Entity<HomePage>,
    window: &mut Window,
    cx: &mut App,
) {
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use one_core::storage::ConnectionType;

    fn encode(payload: &str) -> String {
        URL_SAFE_NO_PAD.encode(payload)
    }

    const SAMPLE: &str = r#"{
        "version": 2,
        "protocol": "ssh",
        "name": "Account@Production Server",
        "token": { "id": "token-id", "value": "token-secret" },
        "asset": { "id": "asset-id", "name": "Production Server", "address": "10.0.0.1" },
        "endpoint": { "host": "gw.example.com", "port": 2222 }
    }"#;

    #[test]
    fn recognizes_jms_prefix_case_insensitively() {
        assert!(is_jump_deeplink("jms://abc"));
        assert!(is_jump_deeplink("JMS://abc"));
        assert!(is_jump_deeplink("  Jms://abc  "));
        assert!(!is_jump_deeplink("jms:abc"));
        assert!(!is_jump_deeplink("ssh://host"));
        assert!(!is_jump_deeplink(""));
        // 多字节前缀不能 panic
        assert!(!is_jump_deeplink("跳板机://abc"));
    }

    #[test]
    fn parses_jump_server_payload() {
        let link = parse_jump_deeplink(&format!("jms://{}", encode(SAMPLE))).expect("应解析成功");
        assert_eq!(link.protocol, JumpProtocol::Ssh);
        assert_eq!(link.label, "Production Server");
        assert_eq!(link.host, "gw.example.com");
        assert_eq!(link.port, 2222);
        assert_eq!(link.token_id, "token-id");
        assert_eq!(link.token_value, "token-secret");
    }

    #[test]
    fn tolerates_url_safe_alphabet_missing_padding_and_trailing_slash() {
        let encoded = encode(SAMPLE);
        assert!(!encoded.contains('='));
        let link = parse_jump_deeplink(&format!("jms://{encoded}//")).expect("应解析成功");
        assert_eq!(link.host, "gw.example.com");
    }

    #[test]
    fn falls_back_to_legacy_token_fields() {
        let payload = r#"{
            "protocol": "sftp",
            "name": "legacy",
            "id": "legacy-id",
            "value": "legacy-secret",
            "endpoint": { "host": "gw.example.com", "port": "2222" }
        }"#;
        let link = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect("应解析成功");
        assert_eq!(link.protocol, JumpProtocol::Sftp);
        assert_eq!(link.label, "legacy");
        assert_eq!(link.port, 2222);
        assert_eq!(link.token_id, "legacy-id");
        assert_eq!(link.token_value, "legacy-secret");
    }

    #[test]
    fn rejects_payload_without_credentials() {
        let payload = r#"{
            "protocol": "ssh",
            "endpoint": { "host": "gw.example.com", "port": 2222 }
        }"#;
        let error = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect_err("应拒绝");
        assert!(format!("{error:#}").contains("token"));
    }

    #[test]
    fn rejects_invalid_endpoint() {
        let missing_host = r#"{
            "protocol": "ssh",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "port": 2222 }
        }"#;
        assert!(parse_jump_deeplink(&format!("jms://{}", encode(missing_host))).is_err());

        let bad_port = r#"{
            "protocol": "ssh",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "host": "gw.example.com", "port": 70000 }
        }"#;
        assert!(parse_jump_deeplink(&format!("jms://{}", encode(bad_port))).is_err());
    }

    #[test]
    fn rejects_unsupported_protocol() {
        let payload = r#"{
            "protocol": "vnc",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "host": "gw.example.com", "port": 2222 }
        }"#;
        let error = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect_err("应拒绝");
        assert!(format!("{error:#}").contains("vnc"));
    }

    #[test]
    fn rejects_malformed_payload_without_echoing_it() {
        let error = parse_jump_deeplink("jms://not-base64-$$$").expect_err("应拒绝");
        assert!(!format!("{error:#}").contains("not-base64"));
    }

    #[test]
    fn builds_ephemeral_ssh_connection() {
        let link = parse_jump_deeplink(&format!("jms://{}", encode(SAMPLE))).expect("应解析成功");
        let connection = connection_from_link(&link, None);
        assert_eq!(connection.id, None);
        assert!(!connection.sync_enabled);
        assert_eq!(connection.name, "Production Server");
        assert_eq!(connection.connection_type, ConnectionType::SshSftp);

        let params: SshParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 SshParams");
        assert_eq!(params.host, "gw.example.com");
        assert_eq!(params.port, 2222);
        assert_eq!(params.username, "JMS-token-id");
        match &params.auth_method {
            SshAuthMethod::Password { password } => assert_eq!(password, "token-secret"),
            other => panic!("应使用 token 作为密码认证: {other:?}"),
        }
        // 认证方式固定为密码：不要在 token 失效时退化成交互式键盘认证。
        assert_eq!(params.keyboard_interactive, Some(false));
        assert_eq!(params.prompt_password, Some(false));
        assert!(!params.prompts_for_password());
        assert!(!params.prompts_for_username());
    }

    #[test]
    fn sftp_link_opens_dual_pane() {
        let payload = r#"{
            "protocol": "sftp",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "host": "gw.example.com", "port": 2222 }
        }"#;
        let link = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect("应解析成功");
        let connection = connection_from_link(&link, None);
        assert_eq!(
            connection.preferred_open_mode,
            Some(PreferredOpenMode::DualPane)
        );
    }

    /// RDP 链接（JumpServer 原生客户端 / Razor）：网关地址和用户名都在 `.rdp`
    /// 文件内容里，payload 里没有 endpoint。
    const RDP_SAMPLE: &str = r#"{
        "version": 2,
        "protocol": "rdp",
        "name": "admin@WinServer[2026-11-27]",
        "token": { "id": "token-id", "value": "token-secret" },
        "file": {
            "name": "admin-WinServer.rdp",
            "content": "full address:s:rdp-gw.example.com:3389\nusername:s:admin|token-id\nsession bpp:i:24\n"
        }
    }"#;

    #[test]
    fn parses_rdp_payload_from_the_rdp_file() {
        let link =
            parse_jump_deeplink(&format!("jms://{}", encode(RDP_SAMPLE))).expect("应解析成功");
        assert_eq!(link.protocol, JumpProtocol::Rdp);
        assert!(link.protocol.needs_password_prompt());
        assert_eq!(link.host, "rdp-gw.example.com");
        assert_eq!(link.port, 3389);
        assert_eq!(link.token_id, "token-id");
        assert_eq!(link.rdp_username.as_deref(), Some("admin|token-id"));
        assert_eq!(link.label, "admin@WinServer[2026-11-27]");
    }

    #[test]
    fn rdp_file_name_labels_the_link_when_the_payload_has_no_name() {
        let payload = r#"{
            "protocol": "rdp",
            "token": { "id": "token-id", "value": "token-secret" },
            "file": {
                "name": "admin-WinServer.rdp",
                "content": "full address:s:rdp-gw.example.com:3389\nusername:s:admin|token-id\n"
            }
        }"#;
        let link = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect("应解析成功");
        assert_eq!(link.label, "admin-WinServer");
    }

    #[test]
    fn rdp_address_without_a_port_uses_the_default_port() {
        let payload = RDP_SAMPLE.replace("rdp-gw.example.com:3389", "rdp-gw.example.com");
        let link = parse_jump_deeplink(&format!("jms://{}", encode(&payload))).expect("应解析成功");
        assert_eq!(link.host, "rdp-gw.example.com");
        assert_eq!(link.port, 3389);
    }

    #[test]
    fn rdp_payload_without_the_rdp_file_falls_back_to_the_endpoint() {
        let payload = r#"{
            "protocol": "rdp",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "host": "gw.example.com", "port": 3389 }
        }"#;
        let link = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect("应解析成功");
        assert_eq!(link.host, "gw.example.com");
        assert_eq!(link.port, 3389);
        assert_eq!(link.rdp_username, None);
        let connection = connection_from_link(&link, None);
        let params: RemoteDesktopParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 RemoteDesktopParams");
        assert_eq!(params.username.as_deref(), Some("JMS-id"));
    }

    #[test]
    fn rejects_rdp_payload_without_any_gateway_address() {
        let payload = r#"{
            "protocol": "rdp",
            "token": { "id": "id", "value": "value" }
        }"#;
        let error = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect_err("应拒绝");
        assert!(format!("{error:#}").contains("endpoint"));
    }

    #[test]
    fn builds_ephemeral_rdp_connection() {
        let link =
            parse_jump_deeplink(&format!("jms://{}", encode(RDP_SAMPLE))).expect("应解析成功");
        let connection = connection_from_link(&link, Some("asset-password".to_string()));
        assert_eq!(connection.id, None);
        assert!(!connection.sync_enabled);
        assert_eq!(connection.connection_type, ConnectionType::Rdp);

        let params: RemoteDesktopParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 RemoteDesktopParams");
        assert_eq!(params.protocol, RemoteDesktopProtocol::Rdp);
        assert_eq!(params.host, "rdp-gw.example.com");
        assert_eq!(params.port, 3389);
        assert_eq!(params.username.as_deref(), Some("admin|token-id"));
        assert_eq!(params.password.as_deref(), Some("asset-password"));
    }

    #[test]
    fn rdp_connection_without_a_password_keeps_it_empty() {
        let link =
            parse_jump_deeplink(&format!("jms://{}", encode(RDP_SAMPLE))).expect("应解析成功");
        let connection = connection_from_link(&link, None);
        let params: RemoteDesktopParams =
            serde_json::from_str(&connection.params).expect("params 应是合法 RemoteDesktopParams");
        assert_eq!(params.password, None);
    }
}
