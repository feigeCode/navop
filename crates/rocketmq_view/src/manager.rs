//! RocketMQ 全局状态管理
//!
//! 参考 mqtt_view::manager 的结构,管理 RocketMQ 连接的生命周期:
//! 创建/测试/移除连接,以及从 StoredConnection 构建运行时参数。

use dashmap::DashMap;
use gpui::Global;
use middleware_runtime::MiddlewareAdmin;
use middleware_view::MiddlewareAdminHandle;
use rocketmq_runtime::{
    RocketmqConnection, RocketmqConnectionFactory, RocketmqError, RocketmqParams as RuntimeParams,
};
use std::sync::Arc;

/// 运行时默认连接超时(秒),与 rocketmq-runtime 保持一致
const DEFAULT_CONNECT_TIMEOUT: u64 = 5;
/// 运行时默认请求超时(毫秒),与 rocketmq-runtime 保持一致
const DEFAULT_REQUEST_TIMEOUT: u64 = 3000;

/// RocketMQ 连接存储:connection_id -> 连接
type ConnectionMap = DashMap<String, Arc<RocketmqConnection>>;

/// RocketMQ 全局状态(gpui Global)
#[derive(Clone)]
pub struct GlobalRocketmqState {
    /// 连接映射:connection_id -> connection
    connections: Arc<ConnectionMap>,
    /// 连接工厂(决定使用哪个后端实现)
    factory: RocketmqConnectionFactory,
}

impl Global for GlobalRocketmqState {}

impl GlobalRocketmqState {
    /// 使用指定工厂创建全局状态
    pub fn new(factory: RocketmqConnectionFactory) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            factory,
        }
    }

    /// 当前工厂的后端类型
    pub fn backend_kind(&self) -> rocketmq_runtime::RocketmqBackendKind {
        self.factory.backend_kind()
    }

    /// 测试连接(创建 -> 发起一次 NameServer 请求 -> 关闭,不进入连接表)
    pub async fn test_connection(&self, params: Arc<RuntimeParams>) -> Result<(), RocketmqError> {
        let connection = self.factory.create_native(params)?;
        let result = connection.test_connection().await;
        connection.close().await;
        result
    }

    /// 创建并存储新连接:先做一次连通性验证,成功后插入连接表。
    /// `connection_id` 与树节点 ID 一致(StoredConnection 数字 ID 的字符串形态)。
    pub async fn create_connection(
        &self,
        connection_id: &str,
        params: Arc<RuntimeParams>,
    ) -> Result<String, RocketmqError> {
        if connection_id.is_empty() {
            return Err(RocketmqError::Config(
                "RocketMQ connection id is required".into(),
            ));
        }

        let connection = Arc::new(self.factory.create_native(params)?);
        // 连通性验证失败直接报错,不进入连接表
        connection.test_connection().await?;

        self.connections
            .insert(connection_id.to_string(), connection);

        Ok(connection_id.to_string())
    }

    /// 获取连接(原生类型,可用 test_connection 等扩展方法)
    pub fn get_connection(&self, connection_id: &str) -> Option<Arc<RocketmqConnection>> {
        self.connections
            .get(connection_id)
            .map(|entry| entry.clone())
    }

    /// 获取标准管理接口句柄(供 middleware_view 通用四页组件使用)
    pub fn get_admin_handle(&self, connection_id: &str) -> Option<MiddlewareAdminHandle> {
        self.get_connection(connection_id)
            .map(|connection| connection as Arc<dyn MiddlewareAdmin>)
    }

    /// 移除连接:先关闭底层长连接,再从连接表删除
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), RocketmqError> {
        if let Some((_, connection)) = self.connections.remove(connection_id) {
            connection.close().await;
        }
        Ok(())
    }

    /// 检查连接是否存在
    pub fn has_connection(&self, connection_id: &str) -> bool {
        self.connections.contains_key(connection_id)
    }

    /// 获取所有连接 ID
    pub fn connection_ids(&self) -> Vec<String> {
        self.connections
            .iter()
            .map(|entry| entry.key().clone())
            .collect()
    }

    /// 获取连接数量
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// 关闭所有连接
    pub async fn close_all(&self) {
        let ids = self.connection_ids();
        for id in ids {
            let _ = self.remove_connection(&id).await;
        }
    }
}

/// RocketMQ 连接管理器辅助函数
pub struct RocketmqManager;

impl RocketmqManager {
    /// 从 StoredConnection 创建运行时参数。
    ///
    /// 持久层与运行时层的 JSON 形态一致,这里做显式字段映射
    /// (Option 数值字段缺失时回退运行时默认值)。
    pub fn params_from_stored(
        stored: &one_core::storage::StoredConnection,
    ) -> Result<Arc<RuntimeParams>, RocketmqError> {
        let params = stored
            .to_rocketmq_params()
            .map_err(|error| RocketmqError::Config(error.to_string()))?;

        Ok(Arc::new(RuntimeParams {
            namesrv_addrs: params.namesrv_addrs,
            access_key: params.access_key,
            secret_key: params.secret_key,
            credential_reference: params.credential_reference,
            domain: params.domain,
            connect_timeout: params.connect_timeout.unwrap_or(DEFAULT_CONNECT_TIMEOUT),
            request_timeout: params.request_timeout.unwrap_or(DEFAULT_REQUEST_TIMEOUT),
            ssh_tunnel: params.ssh_tunnel,
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{RocketmqParams, RocketmqSshTunnelConfig, StoredConnection};

    /// 构造一个带完整参数的 RocketMQ StoredConnection
    fn stored_rocketmq(id: Option<i64>, params: RocketmqParams) -> StoredConnection {
        let mut stored = StoredConnection::new_rocketmq("测试连接".to_string(), params, None);
        stored.id = id;
        stored
    }

    #[test]
    fn params_from_stored_maps_all_fields() {
        let params = RocketmqParams {
            namesrv_addrs: vec!["10.0.0.1:9876".to_string(), "10.0.0.2:9876".to_string()],
            access_key: Some("rocketmq".to_string()),
            secret_key: Some("12345678".to_string()),
            credential_reference: None,
            domain: Some("prod".to_string()),
            connect_timeout: Some(8),
            request_timeout: Some(5000),
            ssh_tunnel: None,
        };
        let stored = stored_rocketmq(Some(42), params);

        let runtime = RocketmqManager::params_from_stored(&stored).expect("参数映射应成功");

        assert_eq!(runtime.namesrv_addrs.len(), 2);
        assert_eq!(runtime.access_key.as_deref(), Some("rocketmq"));
        assert_eq!(runtime.secret_key.as_deref(), Some("12345678"));
        assert_eq!(runtime.domain.as_deref(), Some("prod"));
        assert_eq!(runtime.connect_timeout, 8);
        assert_eq!(runtime.request_timeout, 5000);
        assert!(runtime.acl_enabled());
    }

    #[test]
    fn params_from_stored_applies_runtime_defaults() {
        let stored = stored_rocketmq(None, RocketmqParams::default());

        let runtime = RocketmqManager::params_from_stored(&stored).expect("参数映射应成功");

        // 数值字段缺失时回退运行时默认值
        assert_eq!(runtime.connect_timeout, DEFAULT_CONNECT_TIMEOUT);
        assert_eq!(runtime.request_timeout, DEFAULT_REQUEST_TIMEOUT);
        assert!(!runtime.acl_enabled());
        assert_eq!(runtime.namesrv_addrs, vec!["127.0.0.1:9876".to_string()]);
    }

    #[test]
    fn params_from_stored_rejects_invalid_params_json() {
        let mut stored = stored_rocketmq(None, RocketmqParams::default());
        stored.params = "not-json".to_string();

        let error = RocketmqManager::params_from_stored(&stored).unwrap_err();
        assert!(matches!(error, RocketmqError::Config(_)));
    }

    #[test]
    fn params_from_stored_preserves_ssh_tunnel() {
        let mut params = RocketmqParams::default();
        params.ssh_tunnel = Some(RocketmqSshTunnelConfig {
            enabled: true,
            connection_id: Some(7),
            ..RocketmqSshTunnelConfig::default()
        });
        let stored = stored_rocketmq(None, params);

        let runtime = RocketmqManager::params_from_stored(&stored).unwrap();
        let tunnel = runtime.ssh_tunnel.as_ref().expect("隧道应保留");
        assert!(tunnel.enabled);
        assert_eq!(tunnel.connection_id, Some(7));
    }

    #[tokio::test]
    async fn create_connection_reports_unavailable_backend() {
        let state = GlobalRocketmqState::new(RocketmqConnectionFactory::Unavailable);

        let error = state
            .create_connection("1", Arc::new(RuntimeParams::default()))
            .await
            .unwrap_err();

        assert!(matches!(error, RocketmqError::Config(_)));
    }

    #[tokio::test]
    async fn create_connection_rejects_empty_id() {
        let state = GlobalRocketmqState::new(RocketmqConnectionFactory::Unavailable);

        let error = state
            .create_connection("", Arc::new(RuntimeParams::default()))
            .await
            .unwrap_err();

        assert!(matches!(error, RocketmqError::Config(_)));
    }
}
