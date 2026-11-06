//! 连接型工具的 StoredConnection 装载。

use one_core::storage::StoredConnection;

/// 连接仓库句柄：由注册方（navop_app）注入，避免 extension_tools 直接
/// 依赖 GPUI 全局状态。
pub type ConnectionStore = std::sync::Arc<dyn ConnectionSource + Send + Sync>;

/// 连接读取源：生产实现包 `ConnectionRepository`，测试给内存 map。
pub trait ConnectionSource: Send + Sync {
    fn load(&self, connection_id: i64) -> Option<StoredConnection>;

    /// 按名称查找（MCP 侧 `connection` 参数接受名称；Agent 侧不用）。
    fn find_by_name(&self, _name: &str) -> Option<StoredConnection> {
        None
    }
}

/// 生产实现：同步读 SQLite（repository 的 get 是同步 API）。
pub struct RepositoryConnectionSource {
    repository: std::sync::Arc<one_core::storage::ConnectionRepository>,
}

impl RepositoryConnectionSource {
    pub fn new(repository: std::sync::Arc<one_core::storage::ConnectionRepository>) -> Self {
        Self { repository }
    }
}

impl ConnectionSource for RepositoryConnectionSource {
    fn load(&self, connection_id: i64) -> Option<StoredConnection> {
        use one_core::storage::traits::Repository;
        self.repository.get(connection_id).ok().flatten()
    }

    fn find_by_name(&self, name: &str) -> Option<StoredConnection> {
        use one_core::storage::traits::Repository;
        self.repository
            .list()
            .ok()?
            .into_iter()
            .find(|candidate| candidate.name == name)
    }
}

/// 按资源引用里的连接 id（StoredConnection.id 字符串）取回连接记录。
pub(crate) fn load_stored_connection(
    store: &ConnectionStore,
    connection_id: &str,
) -> Result<StoredConnection, String> {
    let Ok(id) = connection_id.parse::<i64>() else {
        return Err(format!("连接资源 id `{connection_id}` 不是有效的存储 id"));
    };
    store
        .load(id)
        .ok_or_else(|| format!("找不到 id 为 {id} 的连接"))
}
