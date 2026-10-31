//! 连接导入/导出与从全量产品迁移。
//!
//! 三个产品共用同一份迁移包格式（见 [`bundle`]），全量产品还可以直接指定
//! `one-hub.db` 只读迁移（见 [`sqlite`]）。导入先做纯函数规划
//! （判重、重命名、工作区匹配），再逐条落库并广播事件，保证列表即时刷新。

pub(crate) mod bundle;
mod dialog;
mod kinds;
mod sqlite;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::{Context as _, Result, anyhow, bail};
use chrono::Utc;
use gpui::App;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event_from_app};
use one_core::storage::traits::Repository as _;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StorageManager, StoredConnection,
    Workspace, WorkspaceRepository,
};

use self::bundle::plan::{
    ImportPlan, PlanInput, PlannedConnection, PlannedWorkspace, normalize_name, plan_import,
};

pub use self::bundle::{
    BUNDLE_VERSION, BundleConnection, BundleWorkspace, ConnectionBundle, ImportReport,
};
pub use self::dialog::open_import_dialog;
pub use self::sqlite::{
    detect_full_product, full_product_candidates, is_sqlite_database, read_bundle_from_sqlite,
};

pub(crate) use self::kinds::supported_connection_types;

/// 从全量产品的数据库文件一键迁移（自动探测源库）。
pub fn import_from_full_product(cx: &mut App) -> Result<ImportReport> {
    let path = detect_full_product().ok_or_else(|| {
        anyhow!(
            "未自动找到全量产品的 one-hub.db，请点「选择文件…」手动指定（常见位置：~/.config/navop/one-hub.db）"
        )
    })?;
    import_from_database(&path, cx)
}

/// 从用户指定的数据库文件迁移（只读打开，不改动源库）。
pub fn import_from_database(path: &Path, cx: &mut App) -> Result<ImportReport> {
    import_parsed_bundle(self::sqlite::read_bundle_from_sqlite(path)?, cx)
}

/// 从用户指定的源文件迁移：SQLite 库按全量产品库读取，其余按迁移包 JSON 读取。
///
/// 独立产品没有「全量产品目录」可探测时（换机、便携版、全量版已卸载），这是唯一
/// 的迁移入口，因此不能只看扩展名：`one-hub.db` 常被重命名或没有扩展名。
pub fn import_from_source_file(path: &Path, cx: &mut App) -> Result<ImportReport> {
    if is_sqlite_database(path) {
        return import_from_database(path, cx);
    }
    let text =
        std::fs::read_to_string(path).with_context(|| format!("读取「{}」失败", path.display()))?;
    import_bundle(&text, cx)
}

/// 展开用户手填/粘贴的路径：去空白与引号、去 `file://` 前缀、展开 `~` 与 `%VAR%`。
///
/// 弹窗会优先让用户点「选择文件…」，但输入框仍允许手写，而手写时常见的
/// `"~/Desktop/one-hub.db"`（从 Finder 复制/拖入带引号）与 Windows 的
/// `%USERPROFILE%\...` 必须能读通。
pub fn expand_source_path(input: &str) -> PathBuf {
    let trimmed = input.trim().trim_matches(|ch| ch == '"' || ch == '\'');
    let without_scheme = trimmed.strip_prefix("file://").unwrap_or(trimmed);
    let expanded = expand_env_vars(without_scheme);
    if expanded == "~" {
        return dirs::home_dir().unwrap_or_else(|| PathBuf::from(expanded));
    }
    if let Some(rest) = expanded
        .strip_prefix("~/")
        .or_else(|| expanded.strip_prefix("~\\"))
        && let Some(home) = dirs::home_dir()
    {
        return home.join(rest);
    }
    PathBuf::from(expanded)
}

/// 展开 `%VAR%`（Windows 手写路径的常见写法）；不认识的变量原样保留。
fn expand_env_vars(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut rest = input;
    while let Some(start) = rest.find('%') {
        let Some(end_offset) = rest[start + 1..].find('%') else {
            break;
        };
        let end = start + 1 + end_offset;
        let name = &rest[start + 1..end];
        out.push_str(&rest[..start]);
        match std::env::var(name) {
            Ok(value) if !name.is_empty() => out.push_str(&value),
            _ => out.push_str(&rest[start..=end]),
        }
        rest = &rest[end + 1..];
    }
    out.push_str(rest);
    out
}

/// 迁移包里的 JSON 字符串必须是合法 JSON，否则说明包已被破坏。
fn validate_params(connection: &BundleConnection) -> Result<()> {
    if connection.params.trim().is_empty() {
        return Ok(());
    }
    serde_json::from_str::<serde_json::Value>(&connection.params)
        .map(|_| ())
        .with_context(|| format!("连接「{}」的 params 不是合法 JSON", connection.name))
}

/// 把当前产品的连接与工作区导出成迁移包 JSON。
pub fn export_bundle(cx: &App) -> Result<String> {
    let connections = connection_repository(cx)?
        .list()
        .map_err(|error| anyhow!("读取连接失败：{error}"))?;
    let workspaces = workspace_repository(cx)?
        .list()
        .map_err(|error| anyhow!("读取工作区失败：{error}"))?;
    let workspace_names: HashMap<i64, String> = workspaces
        .iter()
        .filter_map(|workspace| Some((workspace.id?, workspace.name.clone())))
        .collect();
    let bundle = ConnectionBundle {
        version: BUNDLE_VERSION,
        exported_at: Utc::now().timestamp(),
        app_id: crate::identity().map(|identity| identity.app_id.to_string()),
        workspaces: workspaces.iter().map(to_bundle_workspace).collect(),
        connections: connections
            .iter()
            .map(|connection| to_bundle_connection(connection, &workspace_names))
            .collect(),
    };
    serde_json::to_string_pretty(&bundle).context("序列化迁移包失败")
}

fn to_bundle_workspace(workspace: &Workspace) -> BundleWorkspace {
    BundleWorkspace {
        name: workspace.name.clone(),
        color: workspace.color.clone(),
        icon: workspace.icon.clone(),
    }
}

fn to_bundle_connection(
    connection: &StoredConnection,
    workspace_names: &HashMap<i64, String>,
) -> BundleConnection {
    BundleConnection {
        name: connection.name.clone(),
        connection_type: connection.connection_type.to_string(),
        params: connection.params.clone(),
        workspace: connection
            .workspace_id
            .and_then(|id| workspace_names.get(&id).cloned()),
        remark: connection.remark.clone(),
        selected_databases: connection.selected_databases.clone(),
        sync_enabled: connection.sync_enabled,
    }
}

/// 从迁移包 JSON 导入连接，返回统计结果。
pub fn import_bundle(json: &str, cx: &mut App) -> Result<ImportReport> {
    let bundle: ConnectionBundle =
        serde_json::from_str(json).context("迁移包 JSON 解析失败，请确认文件内容")?;
    import_parsed_bundle(bundle, cx)
}

/// 迁移包版本不能高于当前产品支持的版本。
fn ensure_supported_version(bundle: &ConnectionBundle) -> Result<()> {
    if bundle.version > BUNDLE_VERSION {
        let (found, supported) = (bundle.version, BUNDLE_VERSION);
        bail!("迁移包版本 {found} 高于当前支持的 {supported}，请升级后再导入");
    }
    Ok(())
}

/// 校验版本并落库。
fn import_parsed_bundle(bundle: ConnectionBundle, cx: &mut App) -> Result<ImportReport> {
    ensure_supported_version(&bundle)?;
    for connection in &bundle.connections {
        validate_params(connection)?;
    }
    let connections_repo = connection_repository(cx)?;
    let workspaces_repo = workspace_repository(cx)?;
    let existing_connections = connections_repo
        .list()
        .map_err(|error| anyhow!("读取已有连接失败：{error}"))?;
    let existing_workspaces = workspaces_repo
        .list()
        .map_err(|error| anyhow!("读取已有工作区失败：{error}"))?;

    let existing_names: Vec<String> = existing_connections
        .iter()
        .map(|c| c.name.clone())
        .collect();
    let existing_identities: Vec<(String, String)> = existing_connections
        .iter()
        .map(|c| (normalize_name(&c.name), c.connection_type.to_string()))
        .collect();
    let existing_workspace_names: Vec<String> =
        existing_workspaces.iter().map(|w| w.name.clone()).collect();
    let plan = plan_import(
        &bundle,
        PlanInput {
            existing_names: &existing_names,
            existing_identities: &existing_identities,
            existing_workspaces: &existing_workspace_names,
        },
        &supported_connection_types(),
    );
    let mut workspace_ids: HashMap<String, i64> = existing_workspaces
        .iter()
        .filter_map(|workspace| Some((normalize_name(&workspace.name), workspace.id?)))
        .collect();
    apply_plan(plan, &mut workspace_ids, cx)
}

/// 逐条写入连接与新建工作区，并广播事件。
fn apply_plan(
    plan: ImportPlan,
    workspace_ids: &mut HashMap<String, i64>,
    cx: &mut App,
) -> Result<ImportReport> {
    let report = plan.report.clone();
    create_workspaces(plan.workspaces, workspace_ids, cx)?;
    create_connections(plan.connections, workspace_ids, cx)?;
    Ok(report)
}

fn create_workspaces(
    workspaces: Vec<PlannedWorkspace>,
    workspace_ids: &mut HashMap<String, i64>,
    cx: &mut App,
) -> Result<()> {
    let repo = workspace_repository(cx)?;
    for planned in workspaces {
        let mut workspace = Workspace {
            id: None,
            name: planned.name,
            color: planned.color,
            icon: planned.icon,
            parent_id: None,
            created_at: None,
            updated_at: None,
            cloud_id: None,
            last_synced_at: None,
            sort_order: None,
            sidebar_collapsed: false,
        };
        let name = workspace.name.clone();
        let id = repo
            .insert(&mut workspace)
            .map_err(|error| anyhow!("创建工作区「{name}」失败：{error}"))?;
        workspace_ids.insert(normalize_name(&name), id);
        emit_connection_event_from_app(
            ConnectionDataEvent::WorkspaceCreated { workspace_id: id },
            cx,
        );
    }
    Ok(())
}

fn create_connections(
    connections: Vec<PlannedConnection>,
    workspace_ids: &HashMap<String, i64>,
    cx: &mut App,
) -> Result<()> {
    let repo = connection_repository(cx)?;
    for planned in connections {
        let workspace_id = planned
            .workspace_key
            .as_ref()
            .and_then(|key| workspace_ids.get(key).copied());
        let mut connection = StoredConnection {
            id: None,
            credential_revision: None,
            name: planned.name,
            connection_type: ConnectionType::from_str(&planned.connection_type),
            params: planned.params,
            workspace_id,
            selected_databases: planned.selected_databases,
            remark: planned.remark,
            sync_enabled: planned.sync_enabled,
            cloud_id: None,
            last_synced_at: None,
            last_used_at: None,
            sort_order: None,
            created_at: None,
            updated_at: None,
            team_id: None,
            owner_id: None,
            preferred_open_mode: None,
            ai_access_level: Default::default(),
        };
        let name = connection.name.clone();
        repo.insert(&mut connection)
            .map_err(|error| anyhow!("导入连接「{name}」失败：{error}"))?;
        emit_connection_event_from_app(ConnectionDataEvent::ConnectionCreated { connection }, cx);
    }
    Ok(())
}

fn connection_repository(cx: &App) -> Result<Arc<ConnectionRepository>> {
    storage(cx)?
        .get::<ConnectionRepository>()
        .ok_or_else(|| anyhow!("连接仓库未初始化"))
}

fn workspace_repository(cx: &App) -> Result<Arc<WorkspaceRepository>> {
    storage(cx)?
        .get::<WorkspaceRepository>()
        .ok_or_else(|| anyhow!("工作区仓库未初始化"))
}

fn storage(cx: &App) -> Result<&StorageManager> {
    cx.try_global::<GlobalStorageState>()
        .map(|state| &state.storage)
        .ok_or_else(|| anyhow!("本地存储尚未初始化"))
}

#[cfg(test)]
mod tests;
