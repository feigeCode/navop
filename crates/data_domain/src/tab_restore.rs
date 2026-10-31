//! Data 域的 Tab 会话恢复：数据库 / Redis / MongoDB 标签。
//!
//! 快照里的 `id` 决定了标签形态：`workspace-*-tab-{ws}` 是工作区分组标签，
//! `database-tab-{id}` / `redis-{id}` / `mongodb-{id}` 是单连接标签。连接或
//! 工作区已经被删掉时返回 `None`，该标签被静默跳过。

use db_view::database_tab::DatabaseTabView;
use domain_api::TabRestorer;
use gpui::{App, AppContext as _, Window};
use mongodb_view::MongoTabView;
use one_core::storage::traits::Repository as _;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection, Workspace,
    WorkspaceRepository,
};
use one_core::tab_container::{TabContentView, TabItemState};
use redis_view::RedisTabView;
use std::sync::Arc;

/// data 产品注册到 `app_kit::tabs` 的恢复器。
pub struct DataTabRestorer;

impl TabRestorer for DataTabRestorer {
    fn content_keys(&self) -> Vec<&'static str> {
        vec!["Database", "Redis", "MongoDB"]
    }

    fn restore(
        &self,
        state: &TabItemState,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<Arc<dyn TabContentView>> {
        match state.key.as_ref() {
            "Database" => restore_database(state, window, cx),
            "Redis" => restore_redis(state, window, cx),
            "MongoDB" => restore_mongo(state, window, cx),
            _ => None,
        }
    }
}

/// 标签对应的恢复目标：工作区分组，或单个连接。
#[derive(Debug, PartialEq)]
enum TabTarget {
    Workspace(i64),
    Connection(i64),
}

fn restore_database(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    let target = target_from(
        state.id.as_ref(),
        "workspace-database-tab-",
        "database-tab-",
    )?;
    let (workspace, connections, active) = resolve(target, ConnectionType::Database, cx)?;
    let view = cx.new(|cx| {
        DatabaseTabView::new_with_active_conn(workspace, connections, active, window, cx)
            .with_external_sidebar()
    });
    Some(Arc::new(view))
}

fn restore_redis(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    let target = target_from(state.id.as_ref(), "workspace-redis-tab-", "redis-")?;
    let (workspace, connections, active) = resolve(target, ConnectionType::Redis, cx)?;
    let view = cx.new(|cx| {
        RedisTabView::new_with_active_conn(workspace, connections, active, window, cx)
            .with_external_sidebar()
    });
    Some(Arc::new(view))
}

fn restore_mongo(
    state: &TabItemState,
    window: &mut Window,
    cx: &mut App,
) -> Option<Arc<dyn TabContentView>> {
    let target = target_from(state.id.as_ref(), "workspace-mongodb-tab-", "mongodb-")?;
    let (workspace, connections, active) = resolve(target, ConnectionType::MongoDB, cx)?;
    let view = cx.new(|cx| {
        MongoTabView::new_with_active_conn(workspace, connections, active, window, cx)
            .with_external_sidebar()
    });
    Some(Arc::new(view))
}

/// 把恢复目标翻译成视图构造参数：(工作区, 连接列表, 当前连接)。
///
/// 工作区标签不记录当时停在哪个连接上，恢复时退回该工作区的第一个连接；
/// 单连接标签按 id 精确查找，连接类型不匹配也视为不存在。
fn resolve(
    target: TabTarget,
    connection_type: ConnectionType,
    cx: &App,
) -> Option<(Option<Workspace>, Vec<StoredConnection>, Option<i64>)> {
    match target {
        TabTarget::Workspace(workspace_id) => {
            let connections = workspace_connections(connection_type, workspace_id, cx);
            let active = connections.first().and_then(|connection| connection.id);
            if connections.is_empty() {
                return None;
            }
            Some((workspace_of(workspace_id, cx), connections, active))
        }
        TabTarget::Connection(connection_id) => {
            let connection = find_connection(connection_id, connection_type, cx)?;
            let active = connection.id;
            Some((None, vec![connection], active))
        }
    }
}

/// 反解 tab id；工作区前缀必须先于单连接前缀尝试。
fn target_from(tab_id: &str, workspace_prefix: &str, connection_prefix: &str) -> Option<TabTarget> {
    if let Some(rest) = tab_id.strip_prefix(workspace_prefix) {
        return rest.parse().ok().map(TabTarget::Workspace);
    }
    let connection_id = tab_id
        .strip_prefix(connection_prefix)?
        .split('-')
        .next()?
        .parse()
        .ok()?;
    Some(TabTarget::Connection(connection_id))
}

fn connection_repository(cx: &App) -> Option<Arc<ConnectionRepository>> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    storage.get::<ConnectionRepository>()
}

fn find_connection(
    connection_id: i64,
    connection_type: ConnectionType,
    cx: &App,
) -> Option<StoredConnection> {
    let repository = connection_repository(cx)?;
    let connection = repository.get(connection_id).ok().flatten()?;
    (connection.connection_type == connection_type).then_some(connection)
}

/// 工作区标签里的兄弟连接；顺序沿用仓库顺序（恢复时不再重排）。
fn workspace_connections(
    connection_type: ConnectionType,
    workspace_id: i64,
    cx: &App,
) -> Vec<StoredConnection> {
    let Some(repository) = connection_repository(cx) else {
        return Vec::new();
    };
    repository
        .list()
        .unwrap_or_default()
        .into_iter()
        .filter(|candidate| {
            candidate.connection_type == connection_type
                && candidate.workspace_id == Some(workspace_id)
        })
        .collect()
}

fn workspace_of(workspace_id: i64, cx: &App) -> Option<Workspace> {
    let storage = cx.global::<GlobalStorageState>().storage.clone();
    let repository = storage.get::<WorkspaceRepository>()?;
    repository.get(workspace_id).ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_restorer_covers_every_data_tab_key() {
        let keys = DataTabRestorer.content_keys();
        assert_eq!(vec!["Database", "Redis", "MongoDB"], keys);
    }

    #[test]
    fn workspace_tab_ids_win_over_connection_prefix() {
        assert_eq!(
            target_from(
                "workspace-database-tab-7",
                "workspace-database-tab-",
                "database-tab-"
            ),
            Some(TabTarget::Workspace(7))
        );
        assert_eq!(
            target_from("workspace-redis-tab-3", "workspace-redis-tab-", "redis-"),
            Some(TabTarget::Workspace(3))
        );
        assert_eq!(
            target_from(
                "workspace-mongodb-tab-9",
                "workspace-mongodb-tab-",
                "mongodb-"
            ),
            Some(TabTarget::Workspace(9))
        );
    }

    #[test]
    fn single_connection_tab_ids_keep_their_connection() {
        assert_eq!(
            target_from(
                "database-tab-42",
                "workspace-database-tab-",
                "database-tab-"
            ),
            Some(TabTarget::Connection(42))
        );
        assert_eq!(
            target_from("redis-42", "workspace-redis-tab-", "redis-"),
            Some(TabTarget::Connection(42))
        );
        assert_eq!(
            target_from("mongodb-42", "workspace-mongodb-tab-", "mongodb-"),
            Some(TabTarget::Connection(42))
        );
    }

    #[test]
    fn foreign_or_broken_tab_ids_are_rejected() {
        assert_eq!(
            target_from("database-tab-", "workspace-database-tab-", "database-tab-"),
            None
        );
        assert_eq!(
            target_from(
                "database-tab-abc",
                "workspace-database-tab-",
                "database-tab-"
            ),
            None
        );
        assert_eq!(
            target_from(
                "workspace-database-tab-",
                "workspace-database-tab-",
                "database-tab-"
            ),
            None
        );
        assert_eq!(
            target_from(
                "ssh-terminal-1-2",
                "workspace-database-tab-",
                "database-tab-"
            ),
            None
        );
    }
}
