//! Redis 全局状态管理

use crate::connection::RedisConnection;
use crate::types::{RedisConnectionConfig, RedisError};
use dashmap::DashMap;
use gpui::Global;
use redis_runtime::RedisConnectionImpl;
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
}

impl Default for GlobalRedisState {
    fn default() -> Self {
        Self::new()
    }
}

impl Global for GlobalRedisState {}

impl GlobalRedisState {
    pub fn new() -> Self {
        Self {
            connections: Arc::new(DashMap::new()),
        }
    }

    pub async fn test_connection(&self, config: &RedisConnectionConfig) -> Result<(), RedisError> {
        let mut connection = RedisConnectionImpl::new(config.clone());
        connection.connect().await?;
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

        let mut conn = RedisConnectionImpl::new(config);
        conn.connect().await?;
        let conn_arc: Arc<RwLock<Box<dyn RedisConnection>>> = Arc::new(RwLock::new(Box::new(conn)));
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
            cluster_nodes: params.cluster.map(|c| c.nodes).unwrap_or_default(),
            ssh_tunnel: params.ssh_tunnel,
        })
    }
}

#[cfg(test)]
pub(crate) mod empty_db_switch_test_support {
    use super::*;
    use one_core::storage::StoredConnection;

    /// 空库切换复现（redis_tree_view 集成测试）用的已连接状态。
    ///
    /// 依赖本机 redis-server（127.0.0.1:16379，db2 有一个种子键）；
    /// 没有服务时返回 Err，调用方跳过测试。
    pub(crate) fn connected_state_for_empty_db_switch() -> Result<GlobalRedisState, anyhow::Error> {
        let config = empty_db_switch_config();
        let state = GlobalRedisState::new();
        state.blocking_create_connection_for_test(config)?;
        Ok(state)
    }

    pub(crate) fn empty_db_switch_stored_connection(name: &str) -> StoredConnection {
        StoredConnection::new_redis(name.to_string(), empty_db_switch_params(), None)
    }

    fn empty_db_switch_params() -> one_core::storage::RedisParams {
        one_core::storage::RedisParams {
            host: "127.0.0.1".to_string(),
            port: 16379,
            password: None,
            username: None,
            credential_reference: None,
            db_index: 0,
            mode: one_core::storage::RedisMode::Standalone,
            use_tls: false,
            connect_timeout: Some(3),
            sentinel: None,
            cluster: None,
            ssh_tunnel: None,
        }
    }

    fn empty_db_switch_config() -> RedisConnectionConfig {
        let mut stored = StoredConnection::new_redis(
            "empty-db-switch".to_string(),
            empty_db_switch_params(),
            None,
        );
        stored.id = Some(4201);
        RedisManager::config_from_stored(&stored).expect("config")
    }

    impl GlobalRedisState {
        /// 测试专用：同步建连（独立 tokio runtime，不依赖 GPUI 全局 Tokio）。
        pub(crate) fn blocking_create_connection_for_test(
            &self,
            config: RedisConnectionConfig,
        ) -> anyhow::Result<()> {
            let state = self.clone();
            let join_result = std::thread::spawn(move || {
                // 自建 current-thread runtime + 超时，避免 redis-server 不在时挂死。
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("test tokio runtime");
                let timeout = tokio::time::Duration::from_secs(3);
                runtime.block_on(async move {
                    tokio::time::timeout(timeout, state.create_connection(config))
                        .await
                        .map_err(|_| anyhow::anyhow!("connect redis 16379 timed out"))?
                        .map_err(|e| anyhow::anyhow!("{e}"))?;
                    Ok::<(), anyhow::Error>(())
                })
            })
            .join()
            .map_err(|_| anyhow::anyhow!("test thread panicked"))?;
            join_result
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_connection_requires_a_connection_id() {
        let state = GlobalRedisState::new();
        let error = state
            .create_connection(RedisConnectionConfig::default())
            .await
            .unwrap_err();

        assert!(matches!(error, RedisError::Internal(_)));
    }
}
