//! JumpServer `jms://` 深链接：把网页端签发的一次性 connection token 变成
//! 一条临时 SSH 会话。
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
//! 打开方式与官方客户端一致：直接以 `JMS-<token-id>` / `<token-value>` SSH 到
//! `endpoint`，由网关上的 Koko 用 token 桥接到目标资产；不写入连接仓库，也不
//! 保存密码，链接本身也不会出现在日志或错误提示里。

use anyhow::{Context as _, Result, bail};
use base64::Engine as _;
use gpui::{App, Window};
use gpui_component::{WindowExt as _, notification::Notification};
use one_core::storage::{
    PreferredOpenMode, SshAuthMethod, SshParams, StoredConnection, StoredTerminalEncoding,
    StoredTerminalType,
};
use one_core::tab_container::TabOpenMode;
use rust_i18n::t;
use serde_json::Value;

use crate::navop_app::GlobalHomePage;

/// JumpServer 深链接协议前缀（大小写不敏感）。
pub(crate) const JUMP_DEEPLINK_PREFIX: &str = "jms://";

/// 深链接里声明的协议。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum JumpProtocol {
    Ssh,
    Sftp,
    Telnet,
}

impl JumpProtocol {
    fn from_payload(raw: &str) -> Result<Self> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "ssh" => Ok(Self::Ssh),
            "sftp" => Ok(Self::Sftp),
            "telnet" => Ok(Self::Telnet),
            other => bail!("不支持的 JumpServer 协议: {other}"),
        }
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

    let host = payload
        .pointer("/endpoint/host")
        .and_then(string_value)
        .context("jms:// payload 缺少 endpoint host")?;
    let port = payload
        .pointer("/endpoint/port")
        .and_then(parse_port)
        .context("jms:// payload 的 endpoint port 无效")?;

    let asset_name = payload
        .pointer("/asset/name")
        .and_then(string_value)
        .filter(|name| !name.is_empty());
    let name = payload
        .get("name")
        .and_then(string_value)
        .filter(|name| !name.is_empty());
    let label = asset_name.or(name).unwrap_or_else(|| host.clone());

    Ok(JumpLink {
        protocol,
        label,
        host,
        port,
        token_id,
        token_value,
    })
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

/// 把深链接转成一条不落库的 SSH 连接：用户名 `JMS-<token-id>`、密码为
/// token 值，认证由网关上的 Koko 完成。
pub(crate) fn connection_from_link(link: &JumpLink) -> StoredConnection {
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
    let connection = connection_from_link(&link);
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
            "protocol": "rdp",
            "token": { "id": "id", "value": "value" },
            "endpoint": { "host": "gw.example.com", "port": 2222 }
        }"#;
        let error = parse_jump_deeplink(&format!("jms://{}", encode(payload))).expect_err("应拒绝");
        assert!(format!("{error:#}").contains("rdp"));
    }

    #[test]
    fn rejects_malformed_payload_without_echoing_it() {
        let error = parse_jump_deeplink("jms://not-base64-$$$").expect_err("应拒绝");
        assert!(!format!("{error:#}").contains("not-base64"));
    }

    #[test]
    fn builds_ephemeral_ssh_connection() {
        let link = parse_jump_deeplink(&format!("jms://{}", encode(SAMPLE))).expect("应解析成功");
        let connection = connection_from_link(&link);
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
        let connection = connection_from_link(&link);
        assert_eq!(
            connection.preferred_open_mode,
            Some(PreferredOpenMode::DualPane)
        );
    }
}
