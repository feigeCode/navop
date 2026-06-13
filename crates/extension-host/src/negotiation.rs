//! init/shutdown 握手与 capability 协商。
//!
//! 在拿到 `JsonRpcClientHandle` 之后:
//!
//! 1. 调用 [`negotiate`] 发送 `init`,等扩展回 `InitResult`。
//! 2. 把 `InitResult.features` 与本地需要的 capability 取交集——缺关键 capability
//!    时返回 [`HostError::Incompatible`]。
//! 3. 返回 [`ExtensionSession`],包含已确认的 capability 集合 + ready drivers。
//!
//! 关闭时调 [`shutdown`] 给扩展 `shutdown.grace_ms` 毫秒优雅退出窗口。

use std::collections::HashSet;
use std::time::Duration;

use extension_protocol::lifecycle::{
    Capability, InitConfig, InitParams, InitResult, ShutdownParams,
};
use extension_protocol::method;
use serde_json::Value;

use crate::client::{JsonRpcClientHandle, RequestOptions};
use crate::error::{HostError, HostResult};

/// 握手配置。
#[derive(Debug, Clone)]
pub struct NegotiationConfig {
    /// 宿主版本(语义版本)。
    pub host_version: String,
    /// 宿主可提供的 API 版本(`name -> version`)。
    pub api_offered: Vec<(String, String)>,
    /// 本次扩展实例 id(UUID 字符串)。
    pub instance_id: String,
    /// 初始化配置(传给扩展的)。
    pub config: InitConfig,
    /// 本地必需的 capability;缺失则视为不兼容。
    pub required_capabilities: Vec<String>,
    /// 握手超时。
    pub handshake_timeout: Duration,
}

impl NegotiationConfig {
    pub fn new(host_version: impl Into<String>, instance_id: impl Into<String>) -> Self {
        Self {
            host_version: host_version.into(),
            api_offered: Vec::new(),
            instance_id: instance_id.into(),
            config: InitConfig::default(),
            required_capabilities: Vec::new(),
            handshake_timeout: Duration::from_millis(crate::DEFAULT_HANDSHAKE_TIMEOUT_MS),
        }
    }

    pub fn offer_api(mut self, name: impl Into<String>, version: impl Into<String>) -> Self {
        self.api_offered.push((name.into(), version.into()));
        self
    }

    pub fn require_capability(mut self, cap: impl Into<String>) -> Self {
        self.required_capabilities.push(cap.into());
        self
    }

    pub fn with_log_level(mut self, level: impl Into<String>) -> Self {
        self.config.log_level = Some(level.into());
        self
    }

    pub fn with_workspace(mut self, dir: impl Into<String>) -> Self {
        self.config.workspace = Some(dir.into());
        self
    }

    pub fn with_locale(mut self, locale: impl Into<String>) -> Self {
        self.config.locale = Some(locale.into());
        self
    }

    pub fn with_handshake_timeout(mut self, d: Duration) -> Self {
        self.handshake_timeout = d;
        self
    }
}

/// 协商成功后返回的会话信息。
#[derive(Debug, Clone)]
pub struct ExtensionSession {
    pub extension_version: String,
    pub api_used: Vec<(String, String)>,
    pub features: HashSet<String>,
    /// 扩展声明实际实现的 wire method 全名集合(`InitResult.methods`)。
    /// 空表示未声明(legacy):宿主照常调用,靠 `METHOD_NOT_FOUND` 回退。
    pub methods: HashSet<String>,
    pub drivers_ready: Vec<String>,
}

impl ExtensionSession {
    pub fn has_feature(&self, cap: &str) -> bool {
        self.features.contains(cap)
    }

    /// 扩展是否声明实现了某个 wire method。
    pub fn declares_method(&self, method: &str) -> bool {
        self.methods.contains(method)
    }

    /// 是否带了任何 method 声明(用于区分 legacy 与显式声明)。
    pub fn has_method_declarations(&self) -> bool {
        !self.methods.is_empty()
    }

    pub fn supports_streaming(&self) -> bool {
        self.has_feature(Capability::STREAMING)
    }

    pub fn supports_cancel(&self) -> bool {
        self.has_feature(Capability::CANCEL_REQUEST)
    }

    pub fn supports_transactions(&self) -> bool {
        self.has_feature(Capability::TRANSACTIONS)
    }
}

/// 执行 init 握手。
pub async fn negotiate(
    client: &JsonRpcClientHandle,
    config: NegotiationConfig,
) -> HostResult<ExtensionSession> {
    let mut params = InitParams::new(config.host_version.clone(), config.instance_id.clone());
    for (k, v) in config.api_offered.iter() {
        params = params.with_api(k.clone(), v.clone());
    }
    params.config = config.config.clone();

    let opts = RequestOptions::default().with_timeout(config.handshake_timeout);
    let raw: Value = client
        .call_raw(method::INIT, serde_json::to_value(&params)?, opts)
        .await?;

    let result: InitResult = serde_json::from_value(raw)?;
    validate_method_declarations(&result.methods)?;

    let features: HashSet<String> = result.features.iter().cloned().collect();
    for cap in &config.required_capabilities {
        if !features.contains(cap) {
            return Err(HostError::Incompatible(format!(
                "extension lacks required capability `{cap}`"
            )));
        }
    }

    Ok(ExtensionSession {
        extension_version: result.extension_version,
        api_used: result.api_used.into_iter().collect(),
        features,
        methods: result.methods.into_iter().collect(),
        drivers_ready: result.drivers_ready,
    })
}

fn validate_method_declarations(methods: &[String]) -> HostResult<()> {
    for method_name in methods {
        if !is_allowed_method_declaration(method_name) {
            return Err(HostError::Incompatible(format!(
                "extension declares unknown IPC method `{method_name}`"
            )));
        }
    }
    Ok(())
}

fn is_allowed_method_declaration(method_name: &str) -> bool {
    method::is_allowed_declaration(method_name)
}

/// 发送 shutdown 通知(非阻塞——扩展可能在 shutdown 期间还在处理 pending 请求)。
///
/// 与 init 不同,shutdown 走的是 *request*(允许扩展返回错误,host 据此判断
/// 是否需要强 kill)。caller 应当在请求返回(或 grace 超时)后:
///
/// 1. 调用 [`JsonRpcClient::shutdown`] 停 reader task。
/// 2. drop [`ProcessHandle`],kill_on_drop 兜底回收 OS 资源。
pub async fn shutdown(client: &JsonRpcClientHandle, grace_ms: u32) -> HostResult<()> {
    let params = ShutdownParams { grace_ms };
    let opts =
        RequestOptions::default().with_timeout(Duration::from_millis(grace_ms as u64 + 1_000));
    // shutdown 返回 null;不在乎具体值
    let _: Value = client
        .call_raw(method::SHUTDOWN, serde_json::to_value(&params)?, opts)
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::JsonRpcClient;
    use crate::transport::{FramedTransport, recv_async, send_async};
    use extension_protocol::envelope::{Response, RpcMessage};
    use extension_protocol::error::{ProtocolError, error_codes};
    use tokio::io::duplex;

    async fn fake_extension_for_init(
        mut reader: tokio::io::ReadHalf<tokio::io::DuplexStream>,
        mut writer: tokio::io::WriteHalf<tokio::io::DuplexStream>,
        ext_version: &str,
        features: &[&str],
        drivers: &[&str],
        methods: &[&str],
    ) {
        let ext_version = ext_version.to_string();
        let features: Vec<String> = features.iter().map(|s| s.to_string()).collect();
        let drivers: Vec<String> = drivers.iter().map(|s| s.to_string()).collect();
        let methods: Vec<String> = methods.iter().map(|s| s.to_string()).collect();
        loop {
            let msg: Result<RpcMessage, _> = recv_async(&mut reader).await;
            match msg {
                Ok(RpcMessage::Request(req)) if req.method == "init" => {
                    let result = serde_json::json!({
                        "extension_version": ext_version.clone(),
                        "api_used": {"database": "1.2"},
                        "features": features.clone(),
                        "drivers_ready": drivers.clone(),
                        "methods": methods.clone(),
                    });
                    send_async(
                        &mut writer,
                        &RpcMessage::Response(Response::ok(req.id, result)),
                    )
                    .await
                    .unwrap();
                }
                Ok(RpcMessage::Request(req)) if req.method == "shutdown" => {
                    send_async(
                        &mut writer,
                        &RpcMessage::Response(Response::ok(req.id, serde_json::json!(null))),
                    )
                    .await
                    .unwrap();
                    break;
                }
                Ok(RpcMessage::Request(req)) => {
                    let pe = ProtocolError::new(error_codes::METHOD_NOT_FOUND, "unknown");
                    send_async(
                        &mut writer,
                        &RpcMessage::Response(Response::err(req.id, pe)),
                    )
                    .await
                    .unwrap();
                }
                Ok(_) => {}
                Err(_) => break,
            }
        }
    }

    fn build_test_client(
        ext_version: &str,
        features: Vec<&'static str>,
        drivers: Vec<&'static str>,
    ) -> (JsonRpcClient, tokio::task::JoinHandle<()>) {
        build_test_client_with_methods(ext_version, features, drivers, Vec::new())
    }

    fn build_test_client_with_methods(
        ext_version: &str,
        features: Vec<&'static str>,
        drivers: Vec<&'static str>,
        methods: Vec<&'static str>,
    ) -> (JsonRpcClient, tokio::task::JoinHandle<()>) {
        let (client_side, server_side) = duplex(8192);
        let (cr, cw) = tokio::io::split(client_side);
        let (sr, sw) = tokio::io::split(server_side);
        let ext_version = ext_version.to_string();
        let server = tokio::spawn(async move {
            fake_extension_for_init(sr, sw, &ext_version, &features, &drivers, &methods).await
        });
        let client = JsonRpcClient::start(FramedTransport::new(cr, cw));
        (client, server)
    }

    #[tokio::test]
    async fn negotiate_returns_session_with_features() {
        let (client, _server) = build_test_client(
            "1.2.0",
            vec!["streaming", "cancel_request"],
            vec!["cassandra"],
        );
        let h = client.handle();
        let cfg = NegotiationConfig::new("1.4.2", "test-instance").offer_api("database", "1.2");
        let session = negotiate(&h, cfg).await.unwrap();
        assert_eq!(session.extension_version, "1.2.0");
        assert!(session.supports_streaming());
        assert!(session.supports_cancel());
        assert!(!session.supports_transactions());
        assert_eq!(session.drivers_ready, vec!["cassandra"]);
    }

    #[tokio::test]
    async fn negotiate_fails_if_required_capability_missing() {
        let (client, _server) = build_test_client("1.0.0", vec!["streaming"], vec!["d1"]);
        let h = client.handle();
        let cfg =
            NegotiationConfig::new("1.4.2", "i1").require_capability(Capability::TRANSACTIONS);
        let err = negotiate(&h, cfg).await.unwrap_err();
        assert!(matches!(err, HostError::Incompatible(_)));
    }

    #[tokio::test]
    async fn negotiate_passes_when_all_required_present() {
        let (client, _server) =
            build_test_client("1.0.0", vec!["streaming", "transactions"], vec!["d1"]);
        let h = client.handle();
        let cfg = NegotiationConfig::new("1.4.2", "i1")
            .require_capability(Capability::STREAMING)
            .require_capability(Capability::TRANSACTIONS);
        let session = negotiate(&h, cfg).await.unwrap();
        assert!(session.supports_streaming());
        assert!(session.supports_transactions());
    }

    #[tokio::test]
    async fn negotiate_rejects_unknown_protocol_method_declarations() {
        let (client, _server) =
            build_test_client_with_methods("1.0.0", vec![], vec!["d1"], vec!["schema/colums"]);
        let h = client.handle();
        let cfg = NegotiationConfig::new("1.4.2", "i1");

        let err = negotiate(&h, cfg).await.unwrap_err();

        assert!(matches!(
            err,
            HostError::Incompatible(message) if message.contains("schema/colums")
        ));
    }

    #[tokio::test]
    async fn negotiate_allows_private_extension_method_declarations() {
        let (client, _server) = build_test_client_with_methods(
            "1.0.0",
            vec![],
            vec!["d1"],
            vec![method::SCHEMA_COLUMNS, "x/demo/profile"],
        );
        let h = client.handle();
        let cfg = NegotiationConfig::new("1.4.2", "i1");

        let session = negotiate(&h, cfg).await.unwrap();

        assert!(session.declares_method(method::SCHEMA_COLUMNS));
        assert!(session.declares_method("x/demo/profile"));
    }

    #[tokio::test]
    async fn shutdown_completes_successfully() {
        let (client, server) = build_test_client("1.0.0", vec![], vec![]);
        let h = client.handle();
        // 先 init,再 shutdown
        let cfg = NegotiationConfig::new("1.4.2", "i1");
        let _ = negotiate(&h, cfg).await.unwrap();
        shutdown(&h, 1_000).await.unwrap();
        // 等待 server 退出
        let _ = tokio::time::timeout(Duration::from_secs(1), server).await;
    }

    #[tokio::test]
    async fn config_builder_sets_log_level_workspace_locale() {
        let cfg = NegotiationConfig::new("1.0.0", "id")
            .with_log_level("debug")
            .with_workspace("/tmp/x")
            .with_locale("zh-CN");
        assert_eq!(cfg.config.log_level.as_deref(), Some("debug"));
        assert_eq!(cfg.config.workspace.as_deref(), Some("/tmp/x"));
        assert_eq!(cfg.config.locale.as_deref(), Some("zh-CN"));
    }

    #[tokio::test]
    async fn config_builder_offers_api_versions() {
        let cfg = NegotiationConfig::new("1.0.0", "id")
            .offer_api("database", "1.2")
            .offer_api("ui", "1.0");
        assert_eq!(cfg.api_offered.len(), 2);
        assert_eq!(
            cfg.api_offered[0],
            ("database".to_string(), "1.2".to_string())
        );
    }

    #[tokio::test]
    async fn extension_session_helpers() {
        let mut features = HashSet::new();
        features.insert(Capability::STREAMING.to_string());
        let mut methods = HashSet::new();
        methods.insert(method::SQL_EXPLAIN.to_string());
        let s = ExtensionSession {
            extension_version: "1.0.0".into(),
            api_used: vec![],
            features,
            methods,
            drivers_ready: vec![],
        };
        assert!(s.supports_streaming());
        assert!(!s.supports_cancel());
        assert!(s.has_method_declarations());
        assert!(s.declares_method(method::SQL_EXPLAIN));
        assert!(!s.declares_method(method::SQL_FORMAT));
    }
}
