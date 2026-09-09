//! RocketMQ 领域类型:统一错误与连接参数。

use connection_tunnel::SshTunnelConfig;
use middleware_runtime::MiddlewareError;
use one_core::storage::CredentialReference;
use serde::{Deserialize, Serialize};

/// RocketMQ 统一错误:直接复用中间件契约错误,保证全链路错误类型一致
pub type RocketmqError = MiddlewareError;

/// RocketMQ 连接参数(运行时;持久化兼容 one_core StorageParams 的 JSON 形态)
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RocketmqParams {
    /// NameServer 地址列表(支持 `host:port`;缺省端口补 9876)
    #[serde(default)]
    pub namesrv_addrs: Vec<String>,
    /// ACL AccessKey(未配置 ACL 时留空)
    #[serde(default)]
    pub access_key: Option<String>,
    /// ACL SecretKey
    #[serde(default)]
    pub secret_key: Option<String>,
    /// 凭据本引用(仅持久层使用,运行时透传)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential_reference: Option<CredentialReference>,
    /// 业务域(预留字段,协议层不使用)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub domain: Option<String>,
    /// 连接超时(秒)
    #[serde(default = "default_connect_timeout")]
    pub connect_timeout: u64,
    /// 请求超时(毫秒)
    #[serde(default = "default_request_timeout")]
    pub request_timeout: u64,
    /// SSH 隧道配置
    #[serde(default)]
    pub ssh_tunnel: Option<SshTunnelConfig>,
}

fn default_connect_timeout() -> u64 {
    5
}

fn default_request_timeout() -> u64 {
    3000
}

/// NameServer 默认端口
pub const DEFAULT_NAMESRV_PORT: u16 = 9876;

impl Default for RocketmqParams {
    fn default() -> Self {
        Self {
            namesrv_addrs: vec![format!("127.0.0.1:{DEFAULT_NAMESRV_PORT}")],
            access_key: None,
            secret_key: None,
            credential_reference: None,
            domain: None,
            connect_timeout: default_connect_timeout(),
            request_timeout: default_request_timeout(),
            ssh_tunnel: None,
        }
    }
}

impl RocketmqParams {
    /// 规范化后的 NameServer 地址列表(补默认端口、去空白)
    pub fn normalized_namesrv_addrs(&self) -> Vec<String> {
        self.namesrv_addrs
            .iter()
            .map(|addr| normalize_addr(addr, DEFAULT_NAMESRV_PORT))
            .collect()
    }

    /// ACL 是否启用(AccessKey 与 SecretKey 均非空)
    pub fn acl_enabled(&self) -> bool {
        self.access_key
            .as_deref()
            .is_some_and(|v| !v.trim().is_empty())
            && self
                .secret_key
                .as_deref()
                .is_some_and(|v| !v.trim().is_empty())
    }

    /// 服务器信息显示
    pub fn server_info(&self) -> String {
        self.normalized_namesrv_addrs().join(";")
    }
}

/// 单个地址规范化:补默认端口、localhost 归一化为 127.0.0.1
pub fn normalize_addr(addr: &str, default_port: u16) -> String {
    let addr = addr.trim();
    if addr.is_empty() {
        return format!("127.0.0.1:{default_port}");
    }
    // 最后一个冒号后为纯数字才视为已含端口(兼容 [IPv6]:port 与裸 IPv6 字面量)
    let has_port = addr
        .rsplit_once(':')
        .is_some_and(|(_, port)| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    if has_port {
        return addr.replace("localhost", "127.0.0.1");
    }
    let host = addr.replace("localhost", "127.0.0.1");
    format!("{host}:{default_port}")
}

impl TryFrom<&serde_json::Map<String, serde_json::Value>> for RocketmqParams {
    type Error = RocketmqError;

    /// 从持久层参数 map 构造(兼容数组与分号/逗号分隔字符串两种形态)。
    ///
    /// 兼容键名:`namesrv_addrs`(推荐)/`namesrv_addr`/`namesrv`。
    fn try_from(map: &serde_json::Map<String, serde_json::Value>) -> Result<Self, Self::Error> {
        let mut addrs: Vec<String> = Vec::new();
        if let Some(value) = map.get("namesrv_addrs").or_else(|| map.get("namesrv_addr")) {
            match value {
                serde_json::Value::Array(items) => {
                    for item in items {
                        if let Some(addr) = item.as_str() {
                            addrs.push(addr.to_string());
                        }
                    }
                }
                serde_json::Value::String(text) => {
                    addrs.extend(
                        text.split([';', ','])
                            .map(str::trim)
                            .filter(|item| !item.is_empty())
                            .map(str::to_string),
                    );
                }
                _ => {
                    return Err(RocketmqError::Config(
                        "namesrv_addrs 应为字符串数组或分隔字符串".into(),
                    ));
                }
            }
        }
        if addrs.is_empty() {
            return Err(RocketmqError::Config(
                "RocketMQ 连接缺少 namesrv_addrs".into(),
            ));
        }

        let params = RocketmqParams {
            namesrv_addrs: addrs,
            access_key: map
                .get("access_key")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            secret_key: map
                .get("secret_key")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            credential_reference: map
                .get("credential_reference")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
            domain: map
                .get("domain")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            connect_timeout: map
                .get("connect_timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(default_connect_timeout()),
            request_timeout: map
                .get("request_timeout")
                .and_then(|v| v.as_u64())
                .unwrap_or(default_request_timeout()),
            ssh_tunnel: map
                .get("ssh_tunnel")
                .and_then(|v| serde_json::from_value(v.clone()).ok()),
        };
        Ok(params)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn default_params() {
        let params = RocketmqParams::default();
        assert_eq!(
            params.normalized_namesrv_addrs(),
            vec!["127.0.0.1:9876".to_string()]
        );
        assert!(!params.acl_enabled());
        assert_eq!(params.connect_timeout, 5);
        assert_eq!(params.request_timeout, 3000);
        assert_eq!(params.server_info(), "127.0.0.1:9876");
    }

    #[test]
    fn normalize_addr_variants() {
        assert_eq!(normalize_addr("10.0.0.1", 9876), "10.0.0.1:9876");
        assert_eq!(normalize_addr("10.0.0.1:9877", 9876), "10.0.0.1:9877");
        assert_eq!(normalize_addr("localhost", 9876), "127.0.0.1:9876");
        assert_eq!(normalize_addr("localhost:9876", 9876), "127.0.0.1:9876");
        assert_eq!(normalize_addr(" 127.0.0.1 ", 9876), "127.0.0.1:9876");
    }

    #[test]
    fn from_params_map_array_and_string() {
        // 数组形态
        let map = json!({
            "namesrv_addrs": ["10.0.0.1:9876", "10.0.0.2:9876"],
            "access_key": "rocketmq",
            "secret_key": "12345678",
            "connect_timeout": 8,
            "request_timeout": 5000
        });
        let params = RocketmqParams::try_from(map.as_object().unwrap()).unwrap();
        assert_eq!(params.namesrv_addrs.len(), 2);
        assert!(params.acl_enabled());
        assert_eq!(params.connect_timeout, 8);
        assert_eq!(params.request_timeout, 5000);

        // 字符串形态(分号分隔,缺端口补默认)
        let map = json!({"namesrv_addr": "10.0.0.1;10.0.0.2:19876"});
        let params = RocketmqParams::try_from(map.as_object().unwrap()).unwrap();
        assert_eq!(
            params.normalized_namesrv_addrs(),
            vec!["10.0.0.1:9876".to_string(), "10.0.0.2:19876".to_string()]
        );

        // 缺少地址报配置错误
        let map = json!({"access_key": "x"});
        assert!(RocketmqParams::try_from(map.as_object().unwrap()).is_err());
    }

    #[test]
    fn params_serde_roundtrip() {
        let params = RocketmqParams::default();
        let json = serde_json::to_string(&params).unwrap();
        let back: RocketmqParams = serde_json::from_str(&json).unwrap();
        assert_eq!(back.namesrv_addrs, params.namesrv_addrs);
        assert_eq!(back.request_timeout, params.request_timeout);
    }
}
