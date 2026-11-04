//! 数据库 / Redis / MongoDB 的连接表单装配。
//!
//! 独立产品只通过 [`domain_api::ConnectionFormProvider`] 拿到表单：种类清单由本模块声明，
//! 表单实体复用 `db_view` / `redis_view` / `mongodb_view` 的既有实现。
//! 三个表单的 `on_saved` 回调签名与契约层的 `ConnectionSavedCallback` 不一致
//! （多出「保存并继续」动作 / 不接收窗口），这里统一留空，保存后的列表刷新交给宿主。

mod labels;
#[cfg(test)]
mod tests;

use db::ipc::{IpcDriverManifest, IpcDriverRegistry, local_icon_asset_path};
use db_view::connection_form_window::{ConnectionFormWindow, ConnectionFormWindowConfig};
use domain_api::{
    ConnectionFormProvider, ConnectionFormRequest, ConnectionKindDescriptor, ConnectionKindGroup,
    ConnectionKindIcon,
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
/// 外部数据库驱动种类的 ID 前缀，完整 ID 为 `db:external:{driver_id}`。
const DB_EXTERNAL_KIND_PREFIX: &str = "db:external:";
const REDIS_KIND_ID: &str = "redis";
const MONGO_KIND_ID: &str = "mongodb";
/// Redis / MongoDB 排在内置数据库之后。
const REDIS_KIND_ORDER: i32 = 100;
const MONGO_KIND_ORDER: i32 = 101;
/// 外部驱动排在内置数据库之后，顺序按驱动清单的排序结果。
const EXTERNAL_KIND_ORDER_BASE: i32 = 200;
/// 这些外部驱动由内置类型代表，不重复出现在新建入口。
///
/// DuckDB / Oracle Go 走内置表单；TDengine / ClickHouse 已有内置类型且表单
/// 会按 `external_driver_id` 路由到驱动，再列一次会出现重名卡片。
const BUILTIN_EXTERNAL_DRIVER_IDS: &[&str] = &["duckdb", "oracle-go", "tdengine", "clickhouse"];

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
    if let Some(driver_id) = kind_id.strip_prefix(DB_EXTERNAL_KIND_PREFIX) {
        if driver_id.is_empty() {
            return None;
        }
        return Some(KindTarget::Database(DatabaseType::external(driver_id)));
    }
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
    kinds.extend(external_kind_descriptors(&IpcDriverRegistry::load_default()));
    kinds
}

/// 已安装的外部数据库驱动 → 新建入口。
///
/// 只收驱动清单声明 `ui.show_in_new_connection` 的驱动；由内置类型代表的
/// 驱动（DuckDB / Oracle Go）跳过，避免同一种类出现两次。
fn external_kind_descriptors(registry: &IpcDriverRegistry) -> Vec<ConnectionKindDescriptor> {
    registry
        .drivers()
        .iter()
        .filter(|driver| driver.ui.show_in_new_connection)
        .filter(|driver| !BUILTIN_EXTERNAL_DRIVER_IDS.contains(&driver.id.as_str()))
        .enumerate()
        .map(|(index, driver)| external_kind_descriptor(driver, index))
        .collect()
}

fn external_kind_descriptor(driver: &IpcDriverManifest, index: usize) -> ConnectionKindDescriptor {
    let mut kind = ConnectionKindDescriptor::new(
        format!("{DB_EXTERNAL_KIND_PREFIX}{}", driver.id),
        label_external_database,
        ConnectionKindGroup::Database,
        EXTERNAL_KIND_ORDER_BASE + index as i32,
    )
    .with_label(driver.name.clone());
    if !driver.description.trim().is_empty() {
        kind = kind.with_description(driver.description.clone());
    }
    if let Some(category) = driver.category.as_deref() {
        kind = kind.with_category(category);
    }
    if let Some(icon) = external_driver_icon(driver) {
        kind = kind.with_icon(icon);
    }
    kind
}

/// 驱动清单的图标：优先品牌色资产，其次磁盘文件。
fn external_driver_icon(driver: &IpcDriverManifest) -> Option<ConnectionKindIcon> {
    if let Some(asset_path) = driver.preferred_icon_asset_path() {
        return Some(ConnectionKindIcon::Asset(asset_path.into()));
    }
    driver
        .preferred_icon_file_path()
        .map(|path| ConnectionKindIcon::Asset(local_icon_asset_path(&path).into()))
}

/// 外部驱动种类的兜底标签；实际展示用清单里的名字（`with_label`）。
fn label_external_database(_: &App) -> SharedString {
    "Database".into()
}

/// 已有连接对应的种类 ID；只识别数据库 / Redis / MongoDB。
fn kind_id_for_connection(connection: &StoredConnection) -> Option<SharedString> {
    match connection.connection_type {
        ConnectionType::Database => connection.to_db_connection().ok().map(|config| {
            let database_type = &config.database_type;
            SharedString::from(match database_type {
                DatabaseType::External { driver_id } => {
                    format!("{DB_EXTERNAL_KIND_PREFIX}{driver_id}")
                }
                _ => format!("{DB_KIND_PREFIX}{}", database_type.as_str()),
            })
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
    let external_driver_id = match &db_type {
        DatabaseType::External { driver_id } => Some(driver_id.clone()),
        _ => None,
    };
    let config = ConnectionFormWindowConfig {
        db_type,
        external_driver_id,
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
