//! Redis 全局状态管理

use crate::connection::RedisConnection;
use crate::types::{RedisConnectionConfig, RedisError};
use dashmap::DashMap;
use gpui::Global;
use redis_runtime::RedisConnectionFactory;
use rust_i18n::t;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Redis 连接存储
type ConnectionMap = DashMap<String, Arc<RwLock<Box<dyn RedisConnection>>>>;

/// Redis 全局状态
#[derive(Clone)]
pub struct GlobalRedisState {
    /// 连接映射：connection_id -> connection
    connections: Arc<ConnectionMap>,
    factory: Arc<RwLock<RedisConnectionFactory>>,
}

impl Default for GlobalRedisState {
    fn default() -> Self {
        Self::new()
    }
}

impl Global for GlobalRedisState {}

impl GlobalRedisState {
    pub fn new() -> Self {
        #[cfg(feature = "builtin-redis")]
        let factory = RedisConnectionFactory::Builtin;
        #[cfg(not(feature = "builtin-redis"))]
        let factory = RedisConnectionFactory::Unavailable;
        Self {
            connections: Arc::new(DashMap::new()),
            factory: Arc::new(RwLock::new(factory)),
        }
    }

    pub fn new_with_factory(factory: RedisConnectionFactory) -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
            factory: Arc::new(RwLock::new(factory)),
        }
    }

    pub async fn set_factory(&self, factory: RedisConnectionFactory) {
        *self.factory.write().await = factory;
    }

    pub async fn test_connection(&self, config: &RedisConnectionConfig) -> Result<(), RedisError> {
        let factory = self.factory.read().await.clone();
        let mut connection = factory.create(config.clone()).await?;
        let ping_result = connection.ping().await;
        let disconnect_result = connection.disconnect().await;
        ping_result?;
        disconnect_result
    }

    /// 创建并存储新连接
    pub async fn create_connection(
        &self,
        config: RedisConnectionConfig,
    ) -> Result<String, RedisError> {
        let connection_id = config.id.clone();
        if connection_id.is_empty() {
            return Err(RedisError::Internal(
                t!("RedisConnection.connection_id_required").to_string(),
            ));
        }

        let factory = self.factory.read().await.clone();
        let conn = factory.create(config).await?;
        let conn_arc: Arc<RwLock<Box<dyn RedisConnection>>> = Arc::new(RwLock::new(conn));
        self.connections.insert(connection_id.clone(), conn_arc);

        Ok(connection_id)
    }

    /// 获取连接
    pub fn get_connection(
        &self,
        connection_id: &str,
    ) -> Option<Arc<RwLock<Box<dyn RedisConnection>>>> {
        self.connections.get(connection_id).map(|r| r.clone())
    }

    /// 移除连接
    pub async fn remove_connection(&self, connection_id: &str) -> Result<(), RedisError> {
        if let Some((_, conn)) = self.connections.remove(connection_id) {
            let mut guard = conn.write().await;
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
        self.connections.iter().map(|r| r.key().clone()).collect()
    }

    /// 获取连接数量
    pub fn connection_count(&self) -> usize {
        self.connections.len()
    }

    /// 关闭所有连接
    pub async fn close_all(&self) {
        let ids: Vec<String> = self.connection_ids();
        for id in ids {
            let _ = self.remove_connection(&id).await;
        }
    }
}

/// Redis 连接管理器辅助函数
pub struct RedisManager;

impl RedisManager {
    /// 从 StoredConnection 创建配置
    pub fn config_from_stored(
        stored: &one_core::storage::StoredConnection,
    ) -> Result<RedisConnectionConfig, RedisError> {
        let params = stored
            .to_redis_params()
            .map_err(|e| RedisError::Serialization(e.to_string()))?;

        let mode = match params.mode {
            one_core::storage::RedisMode::Standalone => {
                crate::types::RedisConnectionMode::Standalone
            }
            one_core::storage::RedisMode::Sentinel => crate::types::RedisConnectionMode::Sentinel,
            one_core::storage::RedisMode::Cluster => crate::types::RedisConnectionMode::Cluster,
        };

        Ok(RedisConnectionConfig {
            id: stored.id.map(|id| id.to_string()).unwrap_or_default(),
            name: stored.name.clone(),
            host: params.host,
            port: params.port,
            password: params.password,
            username: params.username,
            db_index: params.db_index,
            use_tls: params.use_tls,
            timeout: params.connect_timeout.unwrap_or(10),
            mode,
            ssh_tunnel: params.ssh_tunnel,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_connection_uses_the_configured_factory() {
        let state = GlobalRedisState::new_with_factory(RedisConnectionFactory::Unavailable);
        let error = state
            .test_connection(&RedisConnectionConfig::default())
            .await
            .unwrap_err();

        assert!(matches!(error, RedisError::Connection { .. }));
        assert!(error.to_string().contains("native driver is not installed"));
    }
}
