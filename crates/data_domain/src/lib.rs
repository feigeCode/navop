//! `navopData` 的领域实现（骨架）。
//!
//! 首个迁入的是 Redis 单连接打开路径；工作区分组仍留在 `main`，
//! 等 `HomeContext` 暴露当前工作区后再迁。Database/Mongo 随后加入。

use domain_api::{ConnectionOpener, Domain, DomainId, DomainRegistry, HomeContext};
use gpui::{App, AppContext, Window};
use one_core::storage::{ConnectionType, StoredConnection};
use one_core::tab_container::{TabItem, TabOpenMode};
use redis_view::RedisTabView;
use std::sync::Arc;

pub struct DataDomain;

impl Domain for DataDomain {
    fn id(&self) -> DomainId {
        DomainId::DATABASE
    }

    fn register(&self, registry: &mut DomainRegistry) {
        registry.register_connection_opener(ConnectionType::Redis, Arc::new(RedisConnectionOpener));
    }

    fn init(&self, _cx: &mut App) -> anyhow::Result<()> {
        Ok(())
    }
}

struct RedisConnectionOpener;

impl RedisConnectionOpener {
    fn supports_type(connection_type: ConnectionType) -> bool {
        connection_type == ConnectionType::Redis
    }
}

impl ConnectionOpener for RedisConnectionOpener {
    fn open(
        &self,
        connection: &StoredConnection,
        mode: TabOpenMode,
        home: &mut dyn HomeContext,
        window: &mut Window,
        cx: &mut App,
    ) -> bool {
        if !Self::supports_type(connection.connection_type) {
            return false;
        }
        let connection = connection.clone();
        let connection_id = connection.id;
        let (tab_id, connections, workspace) = match connection.workspace_id {
            Some(workspace_id) => {
                let mut connections: Vec<StoredConnection> = home
                    .connections()
                    .iter()
                    .filter(|candidate| {
                        candidate.connection_type == ConnectionType::Redis
                            && candidate.workspace_id == Some(workspace_id)
                    })
                    .cloned()
                    .collect();
                if connections.is_empty() {
                    connections.push(connection);
                }
                home.sort_connections(&mut connections, cx);
                (
                    format!("workspace-redis-tab-{workspace_id}"),
                    connections,
                    home.workspace(workspace_id),
                )
            }
            None => (format!("redis-{}", connection_id.unwrap_or(0)), vec![connection], None),
        };
        let view = cx.new(|cx| {
            RedisTabView::new_with_active_conn(workspace, connections, connection_id, window, cx)
                .with_external_sidebar()
        });
        home.open_tab(
            TabItem::from_dyn(tab_id, "redis", Arc::new(view)),
            mode,
            window,
            cx,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_domain_has_stable_id() {
        assert_eq!(DataDomain.id(), DomainId::DATABASE);
    }

    #[test]
    fn redis_opener_accepts_only_redis_type() {
        assert!(RedisConnectionOpener::supports_type(ConnectionType::Redis));
        assert!(!RedisConnectionOpener::supports_type(ConnectionType::Database));
    }
}
