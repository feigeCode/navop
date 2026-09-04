//! MQTT 全局状态管理
//!
//! 参考 redis_view::manager 的结构,管理 MQTT 连接的生命周期:
//! 创建/测试/移除连接,以及从 StoredConnection 构建运行时配置。

use dashmap::DashMap;
use gpui::Global;
use mqtt_runtime::{
    MqttConnection, MqttConnectionConfig, MqttConnectionFactory, MqttError,
    MqttVersion as RuntimeMqttVersion,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// MQTT 连接存储:connection_id -> 连接
type ConnectionMap = DashMap<String, Arc<RwLock<Box<dyn MqttConnection>>>>;

/// MQTT 全局状态(gpui Global)
#[derive(Clone)]
pub struct GlobalMqttState {
    /// 连接映射:connection_id -> connection
    connections: Arc<ConnectionMap>,
    /// 连接工厂(决定使用哪个后端实现)
    factory: MqttConnectionFactory,
}

impl Global for GlobalMqttState {}

impl GlobalMqttState {
    /// 使用指定工厂创建全局状态
    pub fn new(factory: MqttConnectionFactory) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            factory,
        }
    }

    /// 替换连接工厂
    pub fn set_factory(&mut self, factory: MqttConnectionFactory) {
        self.factory = factory;
    }

    /// 当前工厂的后端类型
    pub fn backend_kind(&self) -> mqtt_runtime::MqttBackendKind {
        self.factory.backend_kind()
    }

    /// 测试连接(创建 -> connect -> ping -> disconnect,不进入连接表)
    pub async fn test_connection(&self, config: &MqttConnectionConfig) -> Result<(), MqttError> {
        let mut connection = self.factory.create(config.clone())?;
        connection.connect().await?;
        let ping_result = connection.ping().await;
        let disconnect_result = connection.disconnect().await;
        ping_result?;
        disconnect_result
    }

    /// 创建并存储新连接:先 connect 成功,再插入连接表。
    /// 返回连接 ID。
    pub async fn create_connection(
        &self,
        config: MqttConnectionConfig,
    ) -> Result<String, MqttError> {
        let connection_id = config.id.clone();
        if connection_id.is_empty() {
            return Err(MqttError::Config("MQTT connection id is required".into()));
        }

        let mut connection = self.factory.create(config)?;
        connection.connect().await?;

        let connection_arc: Arc<RwLock<Box<dyn MqttConnection>>> =
            Arc::new(RwLock::new(connection));
        self.connections
            .insert(connection_id.clone(), connection_arc);

        Ok(connection_id)
    }

    /// 获取连接
    pub fn get_connection(
        &self,
        connection_id: &str,
    ) -> Option<Arc<RwLock<Box<dyn MqttConnection>>>> {
        self.connections
            .get(connection_id)
            .map(|entry| entry.clone())
    }

    /// 移除连接:先 disconnect,再从连接表删除
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), MqttError> {
        if let Some((_, connection)) = self.connections.remove(connection_id) {
            let mut guard = connection.write().await;
            guard.disconnect().await?;
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

/// MQTT 连接管理器辅助函数
pub struct MqttManager;

impl MqttManager {
    /// 从 StoredConnection 创建运行时配置
    ///
    /// 映射规则:
    /// - id/name 取自存储连接
    /// - timeout = connect_timeout.unwrap_or(10)
    /// - keep_alive_secs = keep_alive.unwrap_or(30)
    /// - mqtt_version 做存储层 -> 运行时层的枚举映射
    pub fn config_from_stored(
        stored: &one_core::storage::StoredConnection,
    ) -> Result<MqttConnectionConfig, MqttError> {
        let params = stored
            .to_mqtt_params()
            .map_err(|error| MqttError::Config(error.to_string()))?;

        let mqtt_version = match params.mqtt_version {
            one_core::storage::MqttVersion::V311 => RuntimeMqttVersion::V311,
            one_core::storage::MqttVersion::V5 => RuntimeMqttVersion::V5,
        };

        Ok(MqttConnectionConfig {
            id: stored.id.map(|id| id.to_string()).unwrap_or_default(),
            name: stored.name.clone(),
            host: params.host,
            port: params.port,
            client_id: params.client_id,
            username: params.username,
            password: params.password,
            credential_reference: params.credential_reference,
            use_tls: params.use_tls,
            timeout: params.connect_timeout.unwrap_or(10),
            keep_alive_secs: params.keep_alive.unwrap_or(30),
            mqtt_version,
            clean_session: params.clean_session,
            ssh_tunnel: params.ssh_tunnel,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::{MqttParams, StoredConnection};

    /// 构造一个带完整参数的 MQTT StoredConnection
    fn stored_mqtt(id: Option<i64>, params: MqttParams) -> StoredConnection {
        let mut stored = StoredConnection::new_mqtt("测试连接".to_string(), params, None);
        stored.id = id;
        stored
    }

    #[test]
    fn config_from_stored_maps_all_fields() {
        let params = MqttParams {
            host: "broker.example.com".to_string(),
            port: 8883,
            client_id: "navop-client".to_string(),
            username: Some("user".to_string()),
            password: Some("pass".to_string()),
            credential_reference: None,
            use_tls: true,
            connect_timeout: Some(5),
            keep_alive: Some(60),
            mqtt_version: one_core::storage::MqttVersion::V311,
            clean_session: false,
            ssh_tunnel: None,
        };
        let stored = stored_mqtt(Some(42), params);

        let config = MqttManager::config_from_stored(&stored).expect("配置映射应成功");

        assert_eq!(config.id, "42");
        assert_eq!(config.name, "测试连接");
        assert_eq!(config.host, "broker.example.com");
        assert_eq!(config.port, 8883);
        assert_eq!(config.client_id, "navop-client");
        assert_eq!(config.username.as_deref(), Some("user"));
        assert_eq!(config.password.as_deref(), Some("pass"));
        assert!(config.use_tls);
        assert_eq!(config.timeout, 5);
        assert_eq!(config.keep_alive_secs, 60);
        assert_eq!(config.mqtt_version, RuntimeMqttVersion::V311);
        assert!(!config.clean_session);
        assert!(config.ssh_tunnel.is_none());
    }

    #[test]
    fn config_from_stored_applies_defaults_for_missing_optionals() {
        let params = MqttParams {
            connect_timeout: None,
            keep_alive: None,
            mqtt_version: one_core::storage::MqttVersion::V5,
            ..MqttParams::default()
        };
        let stored = stored_mqtt(None, params);

        let config = MqttManager::config_from_stored(&stored).expect("配置映射应成功");

        assert_eq!(config.timeout, 10);
        assert_eq!(config.keep_alive_secs, 30);
        assert_eq!(config.mqtt_version, RuntimeMqttVersion::V5);
        assert!(config.clean_session);
        assert_eq!(config.id, "");
    }

    #[test]
    fn config_from_stored_rejects_invalid_params_json() {
        let mut stored = stored_mqtt(None, MqttParams::default());
        stored.params = "not-json".to_string();

        let error = MqttManager::config_from_stored(&stored).unwrap_err();
        assert!(matches!(error, MqttError::Config(_)));
    }

    #[tokio::test]
    async fn create_connection_rejects_empty_id() {
        let state = GlobalMqttState::new(MqttConnectionFactory::Unavailable);
        let config = MqttConnectionConfig {
            id: String::new(),
            ..MqttConnectionConfig::default()
        };

        let error = state.create_connection(config).await.unwrap_err();
        assert!(matches!(error, MqttError::Config(_)));
    }

    #[tokio::test]
    async fn test_connection_reports_unavailable_backend() {
        let state = GlobalMqttState::new(MqttConnectionFactory::Unavailable);

        let error = state
            .test_connection(&MqttConnectionConfig::default())
            .await
            .unwrap_err();

        assert!(matches!(error, MqttError::Config(_)));
        assert!(error.to_string().contains("unavailable"));
    }
}
