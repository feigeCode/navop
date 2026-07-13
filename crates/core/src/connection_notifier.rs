use crate::storage::StoredConnection;
use gpui::{App, AppContext, Context, Entity, EventEmitter};

/// 连接数据变更事件
#[derive(Debug, Clone)]
pub enum ConnectionDataEvent {
    /// 连接被创建
    ConnectionCreated { connection: StoredConnection },
    /// 连接被更新（名称、配置等）
    ConnectionUpdated { connection: StoredConnection },
    /// 连接被删除
    ConnectionDeleted {
        connection_id: i64,
        cloud_id: Option<String>,
    },
    /// 工作区被创建
    WorkspaceCreated { workspace_id: i64 },
    /// 工作区被更新
    WorkspaceUpdated { workspace_id: i64 },
    /// 工作区被删除
    WorkspaceDeleted {
        workspace_id: i64,
        cloud_id: Option<String>,
    },
    /// Schema 结构变更（DDL 执行后触发）
    SchemaChanged {
        connection_id: String,
        database: String,
        schema: Option<String>,
    },
    /// 请求执行一次云同步（由表单等非首页入口触发）
    CloudSyncRequested,
    /// 团队缓存已刷新，打开的表单应重新读取团队选项
    TeamCacheUpdated,
}

/// 全局连接数据通知器
pub struct ConnectionDataNotifier;

impl EventEmitter<ConnectionDataEvent> for ConnectionDataNotifier {}

/// 全局包装器，存储 Entity<ConnectionDataNotifier>
#[derive(Clone)]
pub struct GlobalConnectionNotifier(pub Entity<ConnectionDataNotifier>);

impl gpui::Global for GlobalConnectionNotifier {}

/// 初始化全局通知器
pub fn init(cx: &mut App) {
    let notifier = cx.new(|_| ConnectionDataNotifier);
    cx.set_global(GlobalConnectionNotifier(notifier));
}

/// 获取全局通知器 Entity
pub fn get_notifier(cx: &App) -> Option<Entity<ConnectionDataNotifier>> {
    cx.try_global::<GlobalConnectionNotifier>()
        .map(|g| g.0.clone())
}

/// 辅助函数：发送连接事件
pub fn emit_connection_event<T>(event: ConnectionDataEvent, cx: &mut Context<T>) {
    if let Some(notifier) = cx.try_global::<GlobalConnectionNotifier>().cloned() {
        notifier.0.update(cx, |_, cx| {
            cx.emit(event);
        });
    }
}
