//! 数据库 / Redis / MongoDB 的连接表单装配。
//!
//! 独立产品只通过 [`domain_api::ConnectionFormProvider`] 拿到表单：种类清单由本模块声明，
//! 表单实体复用 `db_view` / `redis_view` / `mongodb_view` 的既有实现。
//! 三个表单的 `on_saved` 回调签名与契约层的 `ConnectionSavedCallback` 不一致
//! （多出「保存并继续」动作 / 不接收窗口），这里统一留空，保存后的列表刷新交给宿主。

mod labels;
#[cfg(test)]
mod tests;

use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use domain_api::{
    ConnectionFormProvider, ConnectionFormRequest, ConnectionKindDescriptor, ConnectionKindGroup,
};
use gpui::{AnyView, App, AppContext as _, SharedString, Window};
use labels::{db_kind_label, label_mongodb, label_redis};
use mongodb_view::mongo_form_window::{MongoFormWindow, MongoFormWindowConfig};
use one_core::cloud_sync::get_cached_team_options;
use one_core::storage::traits::Repository;
use one_core::storage::{
    ConnectionRepository, ConnectionType, DatabaseType, GlobalStorageState, StoredConnection,
    Workspace, WorkspaceRepository,
};
use redis_view::redis_form_window::{RedisFormWindow, RedisFormWindowConfig};

/// 内置数据库种类的 ID 前缀，完整 ID 为 `db:{DatabaseType::as_str}`。
const DB_KIND_PREFIX: &str = "db:";
const REDIS_KIND_ID: &str = "redis";
const MONGO_KIND_ID: &str = "mongodb";
/// Redis / MongoDB 排在内置数据库之后。
const REDIS_KIND_ORDER: i32 = 100;
const MONGO_KIND_ORDER: i32 = 101;

/// 数据领域的连接表单提供方。
pub struct DataConnectionFormProvider;

impl ConnectionFormProvider for DataConnectionFormProvider {
    fn kinds(&self, _cx: &App) -> Vec<ConnectionKindDescriptor> {
        kind_descriptors()
    }

    fn kind_for(&self, connection: &StoredConnection, _cx: &App) -> Option<SharedString> {
        kind_id_for_connection(connection)
    }

    fn build(
        &self,
        request: ConnectionFormRequest,
        window: &mut Window,
        cx: &mut App,
    ) -> Option<AnyView> {
        Some(match resolve_kind(request.kind_id.as_ref())? {
            KindTarget::Database(db_type) => {
                build_database_form(db_type, request, FormIo { window, cx })
            }
            KindTarget::Redis => build_redis_form(request, FormIo { window, cx }),
            KindTarget::Mongo => build_mongo_form(request, FormIo { window, cx }),
        })
    }
}

/// 组装表单需要的窗口与全局上下文；集中成一个参数，避免每个构建函数都带 window/cx。
struct FormIo<'a> {
    window: &'a mut Window,
    cx: &'a mut App,
}

/// `kind_id` 解析结果。
enum KindTarget {
    Database(DatabaseType),
    Redis,
    Mongo,
}

/// 解析种类 ID；不属于本领域（或数据库类型不可识别）时返回 `None`。
fn resolve_kind(kind_id: &str) -> Option<KindTarget> {
    if let Some(db_type) = kind_id.strip_prefix(DB_KIND_PREFIX) {
        return DatabaseType::from_str(db_type).map(KindTarget::Database);
    }
    match kind_id {
        REDIS_KIND_ID => Some(KindTarget::Redis),
        MONGO_KIND_ID => Some(KindTarget::Mongo),
        _ => None,
    }
}

/// 本领域声明的全部连接种类：内置数据库 + Redis + MongoDB。
fn kind_descriptors() -> Vec<ConnectionKindDescriptor> {
    let mut kinds: Vec<ConnectionKindDescriptor> = DatabaseType::builtin_all()
        .iter()
        .enumerate()
        .map(|(order, db_type)| {
            ConnectionKindDescriptor::new(
                format!("{DB_KIND_PREFIX}{}", db_type.as_str()),
                db_kind_label(db_type),
                ConnectionKindGroup::Database,
                order as i32,
            )
        })
        .collect();
    kinds.push(ConnectionKindDescriptor::new(
        REDIS_KIND_ID,
        label_redis,
        ConnectionKindGroup::Database,
        REDIS_KIND_ORDER,
    ));
    kinds.push(ConnectionKindDescriptor::new(
        MONGO_KIND_ID,
        label_mongodb,
        ConnectionKindGroup::Database,
        MONGO_KIND_ORDER,
    ));
    kinds
}

/// 已有连接对应的种类 ID；只识别数据库 / Redis / MongoDB。
fn kind_id_for_connection(connection: &StoredConnection) -> Option<SharedString> {
    match connection.connection_type {
        ConnectionType::Database => connection.to_db_connection().ok().map(|config| {
            SharedString::from(format!("{DB_KIND_PREFIX}{}", config.database_type.as_str()))
        }),
        ConnectionType::Redis => Some(REDIS_KIND_ID.into()),
        ConnectionType::MongoDB => Some(MONGO_KIND_ID.into()),
        _ => None,
    }
}

/// 数据库表单：外部驱动统一走默认注册表，表单内部按类型再分派。
fn build_database_form(
    db_type: DatabaseType,
    request: ConnectionFormRequest,
    io: FormIo<'_>,
) -> AnyView {
    let FormIo { window, cx } = io;
    let ConnectionFormRequest {
        editing_connection,
        initial_connection,
        initial_workspace_id,
        ..
    } = request;
    let config = ConnectionFormWindowConfig {
        db_type,
        external_driver_id: None,
        external_driver_registry: db::ipc::IpcDriverRegistry::load_default(),
        editing_connection,
        initial_connection,
        initial_workspace_id,
        on_saved: None,
        workspaces: load_workspaces(cx),
        teams: get_cached_team_options(cx),
        ssh_connections: load_ssh_connections(cx),
    };
    cx.new(|cx| ConnectionFormWindow::new(config, window, cx))
        .into()
}

/// Redis 表单。
fn build_redis_form(request: ConnectionFormRequest, io: FormIo<'_>) -> AnyView {
    let FormIo { window, cx } = io;
    let ConnectionFormRequest {
        editing_connection,
        initial_connection,
        initial_workspace_id,
        ..
    } = request;
    let config = RedisFormWindowConfig {
        editing_connection,
        initial_connection,
        initial_workspace_id,
        on_saved: None,
        workspaces: load_workspaces(cx),
        teams: get_cached_team_options(cx),
        ssh_connections: load_ssh_connections(cx),
    };
    cx.new(|cx| RedisFormWindow::new(config, window, cx)).into()
}

/// MongoDB 表单。
fn build_mongo_form(request: ConnectionFormRequest, io: FormIo<'_>) -> AnyView {
    let FormIo { window, cx } = io;
    let ConnectionFormRequest {
        editing_connection,
        initial_connection,
        initial_workspace_id,
        ..
    } = request;
    let config = MongoFormWindowConfig {
        editing_connection,
        initial_connection,
        initial_workspace_id,
        on_saved: None,
        workspaces: load_workspaces(cx),
        teams: get_cached_team_options(cx),
        ssh_connections: load_ssh_connections(cx),
    };
    cx.new(|cx| MongoFormWindow::new(config, window, cx)).into()
}

/// 工作区分组：存储未就绪时退回空列表（表单按「不分组」展示）。
fn load_workspaces(cx: &App) -> Vec<Workspace> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<WorkspaceRepository>())
        .and_then(|repo| repo.list().ok())
        .unwrap_or_default()
}

/// SSH 连接候选：表单里「通过 SSH 隧道连接」下拉框使用。
fn load_ssh_connections(cx: &App) -> Vec<StoredConnection> {
    cx.try_global::<GlobalStorageState>()
        .and_then(|state| state.storage.get::<ConnectionRepository>())
        .and_then(|repo| repo.list().ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
        .collect()
}
