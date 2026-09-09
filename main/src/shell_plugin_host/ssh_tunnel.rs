//! 扩展连接(composite 扩展,MQTT/RocketMQ)SSH 隧道共享 helper。
//!
//! 职责分三层,供表单与打开/测试链路复用:
//!
//! 1. **存储形状 IO**:`SshTunnelFormValue` ↔ `connection_tunnel::SshTunnelConfig` ↔
//!    `config["ssh_tunnel"]` JSON。敏感字段(password/private_key_content/private_key_passphrase)
//!    落盘前幂等加密、读取时解密,与迁移 SQL 保留的惰性字段形状完全一致
//!    (见 crates/core/migrations/20260828000001 与 models.rs `insert_lazy_ssh_tunnel`)。
//! 2. **引用模式解析**:按 `connection_id` 从 `ConnectionRepository` 取 SSH 连接,
//!    补全隧道缺省字段(对齐 one-core `runtime_resolver` 的既有语义)。
//! 3. **宿主侧地址改写**:启用隧道时逐地址建立本地端口转发并改写 config——
//!    单地址字段(host/port,MQTT)直接改写;`namesrv_addrs`(分号串,RocketMQ)
//!    逐地址建隧道后重组为 `127.0.0.1:本地端口;...`。

use anyhow::{Context as _, Result, anyhow};
use connection_form::SshTunnelFormValue;
use connection_tunnel::{SshTunnelConfig, TunnelGuard, resolve_connection_target};
use one_core::crypto;
use one_core::storage::traits::Repository;
use one_core::storage::{ConnectionRepository, ConnectionType, SshAuthMethod, StoredConnection};
use serde_json::Value;

/// 改写结果:改写后的 config 与随资源生命周期持有的隧道守卫集合。
pub(crate) struct TunnelRewrite {
    pub(crate) config: serde_json::Map<String, Value>,
    pub(crate) guards: Vec<TunnelGuard>,
}

/// 宿主侧入口:读 `config.ssh_tunnel`,启用则解析引用、逐地址建隧道并改写地址字段。
///
/// 未配置/未启用/无地址字段时原样返回(guards 为空),不视为错误。
pub(crate) async fn rewrite_config_with_tunnel(
    mut config: serde_json::Map<String, Value>,
    repository: Option<&ConnectionRepository>,
) -> Result<TunnelRewrite> {
    let Some(raw) = config.get("ssh_tunnel").cloned() else {
        return Ok(TunnelRewrite {
            config,
            guards: Vec::new(),
        });
    };
    let mut tunnel: SshTunnelConfig = serde_json::from_value(raw)
        .context("config.ssh_tunnel is not a valid SSH tunnel configuration")?;
    if !tunnel.enabled {
        return Ok(TunnelRewrite {
            config,
            guards: Vec::new(),
        });
    }
    decrypt_stored_secrets(&mut tunnel);
    if tunnel.connection_id.is_some() {
        resolve_referenced_tunnel(&mut tunnel, repository).await?;
    }

    let mut guards = Vec::new();
    if let Some(namesrv) = config
        .get("namesrv_addrs")
        .and_then(Value::as_str)
        .map(str::to_string)
    {
        // RocketMQ:分号串逐地址建隧道改写后重组
        let rewritten = rewrite_namesrv_addrs(&namesrv, &tunnel, &mut guards).await?;
        config.insert("namesrv_addrs".into(), Value::String(rewritten));
    } else if let Some((host, port)) = single_address(&config) {
        // MQTT 等单地址扩展:直接改写 host/port
        let target = resolve_connection_target(&host, port, Some(&tunnel))
            .await
            .map_err(|error| anyhow!("SSH tunnel for {host}:{port} failed: {error}"))?;
        config.insert("host".into(), Value::String(target.host));
        config.insert("port".into(), Value::from(target.port));
        guards.extend(target.tunnel);
    } else {
        return Err(anyhow!(
            "SSH tunnel enabled but the extension config has no tunnelable address fields (host/port or namesrv_addrs)"
        ));
    }
    Ok(TunnelRewrite { config, guards })
}

/// 单地址字段提取:host 为字符串、port 为数字(MQTT 清单声明 Text+Number)。
fn single_address(config: &serde_json::Map<String, Value>) -> Option<(String, u16)> {
    let host = config.get("host")?.as_str()?.trim().to_string();
    if host.is_empty() {
        return None;
    }
    let port = config
        .get("port")
        .and_then(Value::as_u64)
        .and_then(|port| u16::try_from(port).ok())?;
    Some((host, port))
}

/// 分号串拆分为非空地址列表(容忍空白与结尾分隔符)。
pub(crate) fn split_namesrv_addrs(raw: &str) -> Vec<String> {
    raw.split(';')
        .map(str::trim)
        .filter(|addr| !addr.is_empty())
        .map(str::to_string)
        .collect()
}

/// 解析 `host:port` 地址(兼容 `[::1]:9876` 形式的 IPv6 字面量)。
pub(crate) fn parse_host_port(addr: &str) -> Result<(String, u16)> {
    let (host, port) = addr
        .rsplit_once(':')
        .ok_or_else(|| anyhow!("RocketMQ name server address is missing a port: {addr}"))?;
    let port: u16 = port
        .parse()
        .with_context(|| format!("RocketMQ name server address has an invalid port: {addr}"))?;
    Ok((
        host.trim_start_matches('[')
            .trim_end_matches(']')
            .to_string(),
        port,
    ))
}

async fn rewrite_namesrv_addrs(
    raw: &str,
    tunnel: &SshTunnelConfig,
    guards: &mut Vec<TunnelGuard>,
) -> Result<String> {
    let addresses = split_namesrv_addrs(raw);
    anyhow::ensure!(!addresses.is_empty(), "namesrv_addrs is empty");
    let mut rewritten = Vec::with_capacity(addresses.len());
    for address in &addresses {
        let (host, port) = parse_host_port(address)?;
        let target = resolve_connection_target(&host, port, Some(tunnel))
            .await
            .map_err(|error| anyhow!("SSH tunnel for {address} failed: {error}"))?;
        rewritten.push(format!("{}:{}", target.host, target.port));
        guards.extend(target.tunnel);
    }
    Ok(rewritten.join(";"))
}

/// 引用模式:按 connection_id 取 SSH 连接(含凭据/钥匙串解析)并补全隧道缺省字段。
///
/// 语义对齐 one-core `runtime_resolver::resolve_runtime_connection` 的 Mqtt/Rocketmq
/// 旧分支:引用连接整体覆盖 host/port/username/认证与超时;target_host/target_port
/// 不在此处兜底,由 `resolve_connection_target` 按每个直连地址回退。
async fn resolve_referenced_tunnel(
    tunnel: &mut SshTunnelConfig,
    repository: Option<&ConnectionRepository>,
) -> Result<()> {
    let repository = repository.ok_or_else(|| {
        anyhow!("SSH tunnel references a saved SSH connection but the connection repository is unavailable")
    })?;
    let id = tunnel
        .connection_id
        .ok_or_else(|| anyhow!("SSH tunnel reference is missing connection_id"))?;
    let connection = repository
        .get(id)?
        .ok_or_else(|| anyhow!("referenced SSH connection not found: {id}"))?;
    let resolved = repository
        .resolve_runtime_connection(&connection)
        .context("failed to resolve referenced SSH connection credentials")?;
    apply_referenced_ssh_connection(tunnel, &resolved)
}

/// 纯函数:用已解析的 SSH 连接参数覆盖隧道缺省字段(供单测直接覆盖)。
pub(crate) fn apply_referenced_ssh_connection(
    tunnel: &mut SshTunnelConfig,
    ssh_connection: &StoredConnection,
) -> Result<()> {
    let Some(id) = tunnel.connection_id else {
        return Ok(());
    };
    anyhow::ensure!(
        ssh_connection.id == Some(id),
        "referenced SSH connection id mismatch"
    );
    anyhow::ensure!(
        ssh_connection.connection_type == ConnectionType::SshSftp,
        "referenced connection {id} is not an SSH/SFTP connection"
    );
    let params = ssh_connection.to_ssh_params()?;
    tunnel.host = params.host;
    tunnel.port = params.port;
    tunnel.username = params.username;
    tunnel.timeout = params.connect_timeout;
    match params.auth_method {
        SshAuthMethod::Password { password } => {
            tunnel.auth_type = "password".to_string();
            tunnel.password = Some(password);
            tunnel.private_key_path = None;
            tunnel.private_key_content = None;
            tunnel.private_key_passphrase = None;
        }
        SshAuthMethod::PrivateKey {
            key_path,
            passphrase,
        } => {
            tunnel.auth_type = "private_key".to_string();
            tunnel.password = None;
            tunnel.private_key_path = Some(key_path);
            tunnel.private_key_content = None;
            tunnel.private_key_passphrase = passphrase;
        }
        SshAuthMethod::PrivateKeyContent {
            private_key,
            passphrase,
        } => {
            tunnel.auth_type = "private_key_content".to_string();
            tunnel.password = None;
            tunnel.private_key_path = None;
            tunnel.private_key_content = Some(private_key);
            tunnel.private_key_passphrase = passphrase;
        }
        SshAuthMethod::Agent => {
            tunnel.auth_type = "agent".to_string();
            tunnel.password = None;
            tunnel.private_key_path = None;
            tunnel.private_key_content = None;
            tunnel.private_key_passphrase = None;
        }
        SshAuthMethod::Pageant => {
            tunnel.auth_type = "pageant".to_string();
            tunnel.password = None;
            tunnel.private_key_path = None;
            tunnel.private_key_content = None;
            tunnel.private_key_passphrase = None;
        }
        SshAuthMethod::AutoPublicKey => {
            tunnel.auth_type = "auto_publickey".to_string();
            tunnel.password = None;
            tunnel.private_key_path = None;
            tunnel.private_key_content = None;
            tunnel.private_key_passphrase = None;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// 存储形状 IO:表单值 ↔ SshTunnelConfig ↔ config["ssh_tunnel"] JSON
// ---------------------------------------------------------------------------

/// 表单值 → 存储用隧道配置(字段一一对应,语义对齐 Redis 表单 `build_ssh_tunnel_config`)。
pub(crate) fn tunnel_from_form_value(value: &SshTunnelFormValue) -> Option<SshTunnelConfig> {
    if !value.enabled {
        return None;
    }
    Some(SshTunnelConfig {
        enabled: true,
        connection_id: value.connection_id,
        host: value.host.clone(),
        port: value.port,
        username: value.username.clone(),
        auth_type: value.auth_type.clone(),
        password: value.password.clone(),
        private_key_path: value.private_key_path.clone(),
        private_key_content: value.private_key_content.clone(),
        private_key_passphrase: value.private_key_passphrase.clone(),
        target_host: value.target_host.clone(),
        target_port: value.target_port,
        timeout: value.timeout,
    })
}

/// 隧道配置序列化为存储 JSON(敏感字段幂等加密,与迁移惰性形状一致)。
pub(crate) fn tunnel_to_stored_json(tunnel: &SshTunnelConfig) -> Value {
    let mut stored = tunnel.clone();
    encrypt_stored_secrets(&mut stored);
    serde_json::to_value(&stored).expect("SshTunnelConfig 序列化不应失败")
}

/// 从 `config["ssh_tunnel"]` JSON 反序列化为表单回填值(解密 ENC: 敏感字段)。
pub(crate) fn form_value_from_stored_json(raw: &Value) -> Option<SshTunnelFormValue> {
    let mut tunnel: SshTunnelConfig = serde_json::from_value(raw.clone()).ok()?;
    decrypt_stored_secrets(&mut tunnel);
    Some(SshTunnelFormValue {
        enabled: tunnel.enabled,
        connection_id: tunnel.connection_id,
        host: tunnel.host,
        port: tunnel.port,
        username: tunnel.username,
        auth_type: tunnel.auth_type,
        password: tunnel.password,
        private_key_path: tunnel.private_key_path,
        private_key_content: tunnel.private_key_content,
        private_key_passphrase: tunnel.private_key_passphrase,
        target_host: tunnel.target_host,
        target_port: tunnel.target_port,
        timeout: tunnel.timeout,
    })
}

/// 敏感字段落盘前幂等加密(已是 ENC: 密文则原样,与 `insert_lazy_ssh_tunnel` 一致)。
fn encrypt_stored_secrets(tunnel: &mut SshTunnelConfig) {
    if let Some(password) = tunnel.password.take() {
        tunnel.password = Some(crypto::encrypt_password(&password));
    }
    if let Some(content) = tunnel.private_key_content.take() {
        tunnel.private_key_content = Some(crypto::encrypt_password(&content));
    }
    if let Some(passphrase) = tunnel.private_key_passphrase.take() {
        tunnel.private_key_passphrase = Some(crypto::encrypt_password(&passphrase));
    }
}

/// 读取时解密 ENC: 敏感字段(明文原样保留,解密失败置空与 `decrypt_password` 一致)。
fn decrypt_stored_secrets(tunnel: &mut SshTunnelConfig) {
    if let Some(password) = tunnel.password.take() {
        tunnel.password = Some(crypto::decrypt_password(&password));
    }
    if let Some(content) = tunnel.private_key_content.take() {
        tunnel.private_key_content = Some(crypto::decrypt_password(&content));
    }
    if let Some(passphrase) = tunnel.private_key_passphrase.take() {
        tunnel.private_key_passphrase = Some(crypto::decrypt_password(&passphrase));
    }
}

/// 判断扩展清单声明的表单是否支持 SSH 隧道:
/// 声明 `namesrv_addrs`(RocketMQ)或 `host`+`port`(MQTT 等 TCP 单地址)字段。
pub(crate) fn form_supports_tunnel(
    form: &extension_runtime::extension::manifest::ResourceConnectionForm,
) -> bool {
    let field_ids = form
        .tabs
        .iter()
        .flat_map(|tab| tab.fields.iter().map(|field| field.id.as_str()))
        .collect::<Vec<_>>();
    field_ids.contains(&"namesrv_addrs")
        || (field_ids.contains(&"host") && field_ids.contains(&"port"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn form_value() -> SshTunnelFormValue {
        SshTunnelFormValue {
            enabled: true,
            connection_id: Some(7),
            host: "jump.example.com".into(),
            port: 2222,
            username: "deploy".into(),
            auth_type: "private_key".into(),
            password: None,
            private_key_path: Some("~/.ssh/id_ed25519".into()),
            private_key_content: None,
            private_key_passphrase: Some("passphrase-1".into()),
            target_host: Some("mq.internal".into()),
            target_port: Some(1883),
            timeout: Some(45),
        }
    }

    #[test]
    fn form_value_roundtrips_through_stored_json() {
        let value = form_value();
        let tunnel = tunnel_from_form_value(&value).unwrap();
        let json = tunnel_to_stored_json(&tunnel);
        let restored = form_value_from_stored_json(&json).unwrap();

        assert_eq!(value, restored);
    }

    #[test]
    fn disabled_form_value_maps_to_none() {
        let mut value = form_value();
        value.enabled = false;
        assert!(tunnel_from_form_value(&value).is_none());
    }

    /// 保存形状必须与迁移惰性字段(insert_lazy_ssh_tunnel)完全一致,防回归。
    #[test]
    fn stored_shape_matches_lazy_migration_shape() {
        let value = form_value();
        let tunnel = tunnel_from_form_value(&value).unwrap();

        // 宿主保存形状
        let saved = tunnel_to_stored_json(&tunnel);
        // 迁移惰性形状:同一 SshTunnelConfig + 幂等加密后序列化
        let mut lazy = tunnel.clone();
        if let Some(password) = lazy.password.take() {
            lazy.password = Some(crypto::encrypt_password(&password));
        }
        if let Some(content) = lazy.private_key_content.take() {
            lazy.private_key_content = Some(crypto::encrypt_password(&content));
        }
        if let Some(passphrase) = lazy.private_key_passphrase.take() {
            lazy.private_key_passphrase = Some(crypto::encrypt_password(&passphrase));
        }
        let lazy = serde_json::to_value(&lazy).unwrap();

        // 加密引入随机 nonce,密文字段按解密结果比较,其余字段逐一相等
        for (key, expected) in lazy.as_object().unwrap() {
            let actual = saved.get(key).unwrap();
            match key.as_str() {
                "password" | "private_key_content" | "private_key_passphrase" => {
                    let expected_plain = expected.as_str().map(crypto::decrypt_password);
                    let actual_plain = actual.as_str().map(crypto::decrypt_password);
                    assert_eq!(expected_plain, actual_plain, "secret field {key} mismatch");
                }
                _ => assert_eq!(expected, actual, "field {key} mismatch"),
            }
        }
        assert_eq!(
            saved.as_object().unwrap().len(),
            lazy.as_object().unwrap().len()
        );
    }

    #[test]
    fn namesrv_split_and_parse() {
        assert_eq!(
            split_namesrv_addrs(" 10.0.0.1:9876 ; 10.0.0.2:9876 ;"),
            vec!["10.0.0.1:9876".to_string(), "10.0.0.2:9876".to_string()]
        );
        assert_eq!(
            parse_host_port("10.0.0.2:10911").unwrap(),
            ("10.0.0.2".into(), 10911)
        );
        assert_eq!(parse_host_port("[::1]:9876").unwrap(), ("::1".into(), 9876));
        assert!(parse_host_port("10.0.0.2").is_err());
        assert!(parse_host_port("10.0.0.2:notaport").is_err());
    }

    #[test]
    fn single_address_requires_string_host_and_numeric_port() {
        let mut config = serde_json::Map::new();
        config.insert("host".into(), Value::String("mq.internal".into()));
        config.insert("port".into(), Value::from(1883));
        assert_eq!(
            single_address(&config),
            Some(("mq.internal".to_string(), 1883))
        );

        // 端口为字符串或缺失时不改写
        config.insert("port".into(), Value::String("1883".into()));
        assert_eq!(single_address(&config), None);
        config.remove("port");
        assert_eq!(single_address(&config), None);
    }

    fn ssh_connection(id: i64, auth_method: SshAuthMethod) -> StoredConnection {
        use one_core::storage::SshParams;
        let mut connection = StoredConnection::new_ssh(
            "prod-bastion".into(),
            SshParams {
                disabled_jump_server: None,
                sftp_default_directory: None,
                sftp_account: None,
                host: "bastion.example.com".into(),
                port: 2222,
                username: "deploy".into(),
                auth_method,
                credential_reference: None,
                prompt_username: None,
                prompt_password: None,
                keyboard_interactive: None,
                terminal_encoding: Default::default(),
                terminal_type: Default::default(),
                connect_timeout: Some(15),
                keepalive_interval: None,
                keepalive_max: None,
                default_directory: None,
                init_script: None,
                disable_shell_integration: None,
                x11_forwarding: None,
                allow_legacy_algorithms: None,
                jump_server: None,
                proxy: None,
                os_id: None,
                icon: None,
                icon_file_path: None,
                account_expect: Default::default(),
            },
            None,
        );
        connection.id = Some(id);
        connection
    }

    #[test]
    fn referenced_ssh_connection_fills_tunnel_defaults() {
        let mut tunnel = SshTunnelConfig {
            enabled: true,
            connection_id: Some(42),
            host: String::new(),
            port: 22,
            username: String::new(),
            auth_type: "password".into(),
            password: None,
            private_key_path: None,
            private_key_content: None,
            private_key_passphrase: None,
            target_host: None,
            target_port: None,
            timeout: None,
        };
        apply_referenced_ssh_connection(
            &mut tunnel,
            &ssh_connection(
                42,
                SshAuthMethod::PrivateKey {
                    key_path: "~/.ssh/bastion".into(),
                    passphrase: Some("secret".into()),
                },
            ),
        )
        .unwrap();

        assert_eq!("bastion.example.com", tunnel.host);
        assert_eq!(2222, tunnel.port);
        assert_eq!("deploy", tunnel.username);
        assert_eq!(Some(15), tunnel.timeout);
        assert_eq!("private_key", tunnel.auth_type);
        assert_eq!(Some("~/.ssh/bastion".to_string()), tunnel.private_key_path);
        assert_eq!(Some("secret".to_string()), tunnel.private_key_passphrase);
        assert!(tunnel.password.is_none());
        // target 字段不在此处兜底,由逐地址解析回退
        assert!(tunnel.target_host.is_none() && tunnel.target_port.is_none());
    }

    #[test]
    fn referenced_ssh_connection_rejects_mismatched_or_non_ssh() {
        let mut tunnel = SshTunnelConfig {
            connection_id: Some(42),
            ..Default::default()
        };
        // id 不匹配
        assert!(
            apply_referenced_ssh_connection(&mut tunnel, &ssh_connection(43, SshAuthMethod::Agent))
                .is_err()
        );
        // 非 SSH/SFTP 连接
        let mut redis = StoredConnection::new_redis(
            "cache".into(),
            one_core::storage::RedisParams {
                host: "127.0.0.1".into(),
                port: 6379,
                password: None,
                username: None,
                credential_reference: None,
                db_index: 0,
                mode: Default::default(),
                use_tls: false,
                connect_timeout: None,
                sentinel: None,
                cluster: None,
                ssh_tunnel: None,
            },
            None,
        );
        redis.id = Some(42);
        assert!(apply_referenced_ssh_connection(&mut tunnel, &redis).is_err());
    }

    #[test]
    fn manifest_fields_gate_tunnel_support() {
        let mqtt_like = serde_json::from_str(
            r#"{"tabs":[{"id":"general","label":"常规","fields":[
                {"id":"host","label":"主机","fieldType":"Text"},
                {"id":"port","label":"端口","fieldType":"Number"}
            ]}]}"#,
        )
        .unwrap();
        let rocketmq_like = serde_json::from_str(
            r#"{"tabs":[{"id":"general","label":"常规","fields":[
                {"id":"namesrv_addrs","label":"地址","fieldType":"Text"}
            ]}]}"#,
        )
        .unwrap();
        let search_like =
            serde_json::from_str::<extension_runtime::extension::manifest::ResourceConnectionForm>(
                r#"{"tabs":[{"id":"general","label":"常规","fields":[
                {"id":"url","label":"URL","fieldType":"Text"}
            ]}]}"#,
            )
            .unwrap();

        assert!(form_supports_tunnel(&mqtt_like));
        assert!(form_supports_tunnel(&rocketmq_like));
        assert!(!form_supports_tunnel(&search_like));
    }

    #[tokio::test]
    async fn rewrite_passes_through_when_tunnel_missing_or_disabled() {
        // 无 ssh_tunnel 键:原样直通
        let mut config = serde_json::Map::new();
        config.insert("host".into(), Value::String("mq.internal".into()));
        config.insert("port".into(), Value::from(1883));
        let outcome = rewrite_config_with_tunnel(config.clone(), None)
            .await
            .unwrap();
        assert!(outcome.guards.is_empty());
        assert_eq!(config, outcome.config);

        // 禁用:保留键但直通
        let mut disabled = config.clone();
        disabled.insert(
            "ssh_tunnel".into(),
            serde_json::json!({"enabled": false, "host": "jump.example.com"}),
        );
        let outcome = rewrite_config_with_tunnel(disabled.clone(), None)
            .await
            .unwrap();
        assert!(outcome.guards.is_empty());
        assert_eq!(disabled, outcome.config);
    }

    #[tokio::test]
    async fn rewrite_rejects_enabled_tunnel_without_address_fields() {
        let mut config = serde_json::Map::new();
        config.insert("url".into(), Value::String("https://example.test".into()));
        config.insert(
            "ssh_tunnel".into(),
            serde_json::json!({"enabled": true, "host": "jump.example.com", "username": "deploy", "auth_type": "agent"}),
        );
        assert!(rewrite_config_with_tunnel(config, None).await.is_err());
    }
}
