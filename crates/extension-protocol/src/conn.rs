//! 连接管理 (`conn/*`)。
//!
//! 一个扩展进程内可以同时持有多个连接,每个用 `conn_id` 标识。
//! 凭证不直接传明文——通过 [`SecretRef`] 走宿主的 secret store。
//!
//! 详见 [`docs/design/extensions/api-database.md`] §5。

use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// 宿主分配的连接 id。
///
/// u64 足够(单个扩展进程几乎不会跑出 2^32 个并发连接);也避免被解释为时间戳。
pub type ConnId = u64;

/// 一次申请到的凭证引用。
///
/// 字符串前缀语义参考 `docs/design/extensions/security.md`:
///
/// - `kss://onetcli/sec/<id>` - secret store reference(默认)
/// - `env://<NAME>` - 环境变量(开发模式)
/// - `inline:base64(...)` - 测试用,生产禁用
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SecretRef {
    pub secret_ref: String,
}

impl SecretRef {
    pub fn new(s: impl Into<String>) -> Self {
        Self {
            secret_ref: s.into(),
        }
    }
}

/// 凭证容器——key 是凭证名(`password` / `keyfile` / `token` / ...),value 是
/// 一个 [`SecretRef`]。扩展按需读取。
pub type Credentials = HashMap<String, SecretRef>;

/// `conn/test` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnTestParams {
    pub driver_id: String,
    pub config: Value,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub credentials: Credentials,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ConnOptions>,
}

impl ConnTestParams {
    pub fn new(driver_id: impl Into<String>, config: Value) -> Self {
        Self {
            driver_id: driver_id.into(),
            config,
            credentials: HashMap::new(),
            options: None,
        }
    }

    pub fn with_credential(mut self, name: impl Into<String>, secret: SecretRef) -> Self {
        self.credentials.insert(name.into(), secret);
        self
    }

    pub fn with_options(mut self, options: ConnOptions) -> Self {
        self.options = Some(options);
        self
    }
}

/// `conn/test` 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnTestResult {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_version: Option<String>,
    #[serde(default)]
    pub warnings: Vec<String>,
    /// 整个测试耗时(毫秒)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latency_ms: Option<u32>,
}

/// `conn/open` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnOpenParams {
    pub driver_id: String,
    /// 驱动私有的连接配置(host/port/database/...)。结构由驱动自描述。
    pub config: Value,
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub credentials: Credentials,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<ConnOptions>,
}

impl ConnOpenParams {
    pub fn new(driver_id: impl Into<String>, config: Value) -> Self {
        Self {
            driver_id: driver_id.into(),
            config,
            credentials: HashMap::new(),
            options: None,
        }
    }

    pub fn with_credential(mut self, name: impl Into<String>, secret: SecretRef) -> Self {
        self.credentials.insert(name.into(), secret);
        self
    }

    pub fn with_options(mut self, options: ConnOptions) -> Self {
        self.options = Some(options);
        self
    }
}

/// 通用连接选项,跨 driver 共享。Driver 私有项放 `extra`。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connect_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub read_timeout_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keepalive_ms: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssl: Option<SslOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_tunnel: Option<SshTunnelOptions>,
    /// 驱动自有选项(charset / app_name / pool_size 等)。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// TLS / SSL 配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SslOptions {
    /// `disable` / `prefer` / `require` / `verify_ca` / `verify_full`。
    pub mode: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ca_ref: Option<SecretRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cert_ref: Option<SecretRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key_ref: Option<SecretRef>,
    /// 不校验证书域名(自签场景)。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub insecure_skip_verify: Option<bool>,
}

impl SslOptions {
    pub fn new(mode: impl Into<String>) -> Self {
        Self {
            mode: mode.into(),
            ca_ref: None,
            cert_ref: None,
            key_ref: None,
            insecure_skip_verify: None,
        }
    }
}

/// SSH 隧道配置——由宿主预先开好后下发 session_ref。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshTunnelOptions {
    /// 宿主已建立的 SSH session 引用(`sht://onetcli/ssh/<id>`)。
    pub session_ref: String,
}

/// `conn/open` 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnOpenResult {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_info: Option<ServerInfo>,
}

/// 服务端版本信息。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ServerInfo {
    pub version: String,
    #[serde(default)]
    pub features: Vec<String>,
    /// 任意驱动自描述字段。
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub extra: Value,
}

/// `conn/close` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnCloseParams {
    pub conn_id: ConnId,
}

/// `conn/ping` 请求参数。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnPingParams {
    pub conn_id: ConnId,
}

/// `conn/ping` 响应。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConnPingResult {
    pub latency_ms: u32,
}

/// `conn/use` 请求参数。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ConnUseParams {
    pub conn_id: ConnId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schema: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_ref_round_trip() {
        let s = SecretRef::new("kss://onetcli/sec/abc");
        let j = serde_json::to_string(&s).unwrap();
        assert_eq!(j, r#"{"secret_ref":"kss://onetcli/sec/abc"}"#);
        let p: SecretRef = serde_json::from_str(&j).unwrap();
        assert_eq!(p.secret_ref, "kss://onetcli/sec/abc");
    }

    #[test]
    fn conn_test_params_builder() {
        let p = ConnTestParams::new("cassandra", serde_json::json!({"host": "127.0.0.1"}))
            .with_credential("password", SecretRef::new("kss://onetcli/sec/abc"));
        assert_eq!(p.driver_id, "cassandra");
        assert!(p.credentials.contains_key("password"));
    }

    #[test]
    fn conn_test_params_round_trip() {
        let p = ConnTestParams::new("postgres", serde_json::json!({"host": "h", "port": 5432}))
            .with_credential("password", SecretRef::new("kss://x"))
            .with_options(ConnOptions {
                connect_timeout_ms: Some(5_000),
                ..Default::default()
            });
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ConnTestParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.driver_id, "postgres");
        assert_eq!(
            parsed.config,
            serde_json::json!({"host": "h", "port": 5432})
        );
        assert_eq!(parsed.credentials.len(), 1);
        assert_eq!(parsed.options.unwrap().connect_timeout_ms, Some(5_000));
    }

    #[test]
    fn conn_test_result_with_warnings() {
        let r = ConnTestResult {
            ok: true,
            server_version: Some("4.1.3".into()),
            warnings: vec!["weak cipher".into()],
            latency_ms: Some(12),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ConnTestResult = serde_json::from_str(&j).unwrap();
        assert!(parsed.ok);
        assert_eq!(parsed.server_version.as_deref(), Some("4.1.3"));
        assert_eq!(parsed.warnings, vec!["weak cipher"]);
        assert_eq!(parsed.latency_ms, Some(12));
    }

    #[test]
    fn conn_open_params_round_trip() {
        let p = ConnOpenParams::new("cassandra", serde_json::json!({"host": "h"}));
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ConnOpenParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.driver_id, "cassandra");
        assert!(parsed.credentials.is_empty());
        assert!(parsed.options.is_none());
    }

    #[test]
    fn conn_open_result_round_trip() {
        let r = ConnOpenResult {
            conn_id: 17,
            server_info: Some(ServerInfo {
                version: "4.1.3".into(),
                features: vec!["lwt".into(), "udt".into()],
                ..Default::default()
            }),
        };
        let j = serde_json::to_string(&r).unwrap();
        let parsed: ConnOpenResult = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 17);
        let info = parsed.server_info.unwrap();
        assert_eq!(info.version, "4.1.3");
        assert_eq!(info.features.len(), 2);
    }

    #[test]
    fn ssl_options_with_certs() {
        let s = SslOptions::new("require");
        let j = serde_json::to_string(&s).unwrap();
        // skip_serializing_if 必须生效
        assert!(!j.contains("ca_ref"));
        assert!(!j.contains("cert_ref"));
        assert!(!j.contains("insecure_skip_verify"));
        assert!(j.contains(r#""mode":"require""#));
    }

    #[test]
    fn ssh_tunnel_options_round_trip() {
        let t = SshTunnelOptions {
            session_ref: "sht://onetcli/ssh/abc".into(),
        };
        let j = serde_json::to_string(&t).unwrap();
        let parsed: SshTunnelOptions = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.session_ref, "sht://onetcli/ssh/abc");
    }

    #[test]
    fn conn_close_params_round_trip() {
        let p = ConnCloseParams { conn_id: 42 };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ConnCloseParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 42);
    }

    #[test]
    fn conn_ping_round_trip() {
        let p = ConnPingParams { conn_id: 9 };
        let r = ConnPingResult { latency_ms: 3 };
        let j = serde_json::to_string(&(p, r)).unwrap();
        // 简单 smoke,确保 serde 正常
        assert!(j.contains(r#""conn_id":9"#));
        assert!(j.contains(r#""latency_ms":3"#));
    }

    #[test]
    fn conn_use_params_skip_none() {
        let p = ConnUseParams {
            conn_id: 5,
            database: Some("db1".into()),
            schema: None,
            role: None,
        };
        let j = serde_json::to_string(&p).unwrap();
        assert!(j.contains(r#""database":"db1""#));
        assert!(!j.contains("schema"));
        assert!(!j.contains("role"));
    }

    #[test]
    fn conn_use_params_round_trip() {
        let p = ConnUseParams {
            conn_id: 5,
            database: Some("db".into()),
            schema: Some("public".into()),
            role: Some("admin".into()),
        };
        let j = serde_json::to_string(&p).unwrap();
        let parsed: ConnUseParams = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.conn_id, 5);
        assert_eq!(parsed.database.as_deref(), Some("db"));
        assert_eq!(parsed.schema.as_deref(), Some("public"));
        assert_eq!(parsed.role.as_deref(), Some("admin"));
    }

    #[test]
    fn conn_options_full_round_trip() {
        let o = ConnOptions {
            connect_timeout_ms: Some(5_000),
            read_timeout_ms: Some(30_000),
            keepalive_ms: Some(60_000),
            ssl: Some(SslOptions {
                mode: "verify_full".into(),
                ca_ref: Some(SecretRef::new("kss://ca")),
                cert_ref: None,
                key_ref: None,
                insecure_skip_verify: None,
            }),
            ssh_tunnel: Some(SshTunnelOptions {
                session_ref: "sht://sess".into(),
            }),
            extra: serde_json::json!({"app_name": "onetcli"}),
        };
        let j = serde_json::to_string(&o).unwrap();
        let parsed: ConnOptions = serde_json::from_str(&j).unwrap();
        assert_eq!(parsed.connect_timeout_ms, Some(5_000));
        assert_eq!(parsed.read_timeout_ms, Some(30_000));
        assert_eq!(parsed.keepalive_ms, Some(60_000));
        assert_eq!(parsed.ssl.as_ref().unwrap().mode, "verify_full");
        assert_eq!(
            parsed.ssh_tunnel.as_ref().unwrap().session_ref,
            "sht://sess"
        );
        assert_eq!(parsed.extra, serde_json::json!({"app_name": "onetcli"}));
    }
}
