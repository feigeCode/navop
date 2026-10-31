//! 导入规划的纯函数：判重、重命名、工作区匹配与统计。

use std::collections::HashSet;

use one_core::storage::ConnectionType;

use super::{BundleConnection, ConnectionBundle};

/// 一次导入的统计结果。
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportReport {
    /// 实际写入的连接数。
    pub added: usize,
    /// 因同名同类型重复或数据无效而跳过的连接数。
    pub skipped: usize,
    /// 与已有名称冲突、追加了后缀的连接数。
    pub renamed: usize,
    /// 新建的工作区数。
    pub workspaces_created: usize,
    /// 当前产品不支持的连接类型数。
    pub unsupported: usize,
    /// 端口转发连接数：`params` 里引用了其它连接的本地 id，需要重映射，本轮先跳过。
    pub skipped_port_forwarding: usize,
    /// 引用了不存在的工作区、已落到「未分组」的连接数。
    pub ungrouped: usize,
    /// `params` 中仍有其它产品密文、需要用户重新输入密码的连接数。
    pub needs_repair: usize,
}

impl ImportReport {
    /// 弹窗与通知使用的一句话摘要。
    pub fn summary(&self) -> String {
        let mut parts = vec![format!("新增 {} 个连接", self.added)];
        let extras = [
            (self.workspaces_created, "新建 {n} 个工作区"),
            (self.renamed, "{n} 个重名连接已追加后缀"),
            (self.skipped, "跳过 {n} 个重复连接"),
            (self.unsupported, "{n} 个连接类型当前产品不支持"),
            (
                self.skipped_port_forwarding,
                "{n} 个端口转发暂未导入（连接引用未重映射）",
            ),
            (self.ungrouped, "{n} 个连接已放入未分组"),
            (self.needs_repair, "{n} 个连接需要重新输入密码"),
        ];
        for (count, template) in extras {
            if count > 0 {
                parts.push(template.replace("{n}", &count.to_string()));
            }
        }
        parts.join("，")
    }
}

/// 导入规划依赖的已有数据快照。
pub struct PlanInput<'a> {
    /// 已有连接名（保留原始大小写，仅用于判重）。
    pub existing_names: &'a [String],
    /// 已有连接的 (规范化名称, 连接类型) 组合。
    pub existing_identities: &'a [(String, String)],
    /// 已有工作区名。
    pub existing_workspaces: &'a [String],
}

/// 待新建的工作区。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedWorkspace {
    pub name: String,
    pub color: Option<String>,
    pub icon: Option<String>,
}

/// 待写入的连接（id、云端字段等本地元数据全部清空）。
#[derive(Debug, Clone, PartialEq)]
pub struct PlannedConnection {
    pub name: String,
    pub connection_type: String,
    pub params: String,
    /// 规范化后的工作区名，已确认存在；`None` 表示未分组。
    pub workspace_key: Option<String>,
    pub remark: Option<String>,
    pub selected_databases: Option<String>,
    pub sync_enabled: bool,
    pub renamed: bool,
    pub ungrouped: bool,
    pub needs_repair: bool,
}

/// 导入计划：先全部规划，再逐条落库，避免半途失败留下半份数据。
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ImportPlan {
    pub workspaces: Vec<PlannedWorkspace>,
    pub connections: Vec<PlannedConnection>,
    pub report: ImportReport,
}

enum SkipReason {
    /// 同名同类型已存在（含包内重复）。
    Duplicate,
    /// 当前产品不支持该连接类型。
    Unsupported,
    /// 端口转发引用未重映射，先跳过。
    PortForwarding,
}

/// 计算导入计划；纯函数，不触碰数据库。
pub fn plan_import(
    bundle: &ConnectionBundle,
    input: PlanInput<'_>,
    supported: &[String],
) -> ImportPlan {
    let mut workspace_keys: HashSet<String> = input
        .existing_workspaces
        .iter()
        .map(|name| normalize_name(name))
        .collect();
    let mut used_identities: HashSet<(String, String)> =
        input.existing_identities.iter().cloned().collect();
    let mut used_names: HashSet<String> = input
        .existing_names
        .iter()
        .map(|name| normalize_name(name))
        .collect();
    let mut plan = ImportPlan::default();

    for workspace in &bundle.workspaces {
        let key = normalize_name(&workspace.name);
        if key.is_empty() || !workspace_keys.insert(key) {
            continue;
        }
        plan.workspaces.push(PlannedWorkspace {
            name: workspace.name.trim().to_string(),
            color: workspace.color.clone(),
            icon: workspace.icon.clone(),
        });
    }

    let supported_set: HashSet<&str> = supported.iter().map(String::as_str).collect();
    let report = &mut plan.report;
    for raw in &bundle.connections {
        match plan_connection(
            raw,
            &supported_set,
            &workspace_keys,
            &mut used_identities,
            &mut used_names,
        ) {
            Ok(connection) => {
                report.renamed += usize::from(connection.renamed);
                report.ungrouped += usize::from(connection.ungrouped);
                report.needs_repair += usize::from(connection.needs_repair);
                plan.connections.push(connection);
            }
            Err(SkipReason::Duplicate) => report.skipped += 1,
            Err(SkipReason::Unsupported) => report.unsupported += 1,
            Err(SkipReason::PortForwarding) => report.skipped_port_forwarding += 1,
        }
    }

    plan.report.workspaces_created = plan.workspaces.len();
    plan.report.added = plan.connections.len();
    plan
}

fn plan_connection(
    raw: &BundleConnection,
    supported: &HashSet<&str>,
    workspace_keys: &HashSet<String>,
    used_identities: &mut HashSet<(String, String)>,
    used_names: &mut HashSet<String>,
) -> Result<PlannedConnection, SkipReason> {
    let kind = canonical_kind(&raw.connection_type).ok_or(SkipReason::Unsupported)?;
    if !supported.contains(kind.as_str()) {
        return Err(SkipReason::Unsupported);
    }
    // 端口转发的 params 里保存着其它连接的本地 id；新库会重新分配 id，先跳过避免引用错乱。
    if kind == "PortForwarding" {
        return Err(SkipReason::PortForwarding);
    }
    let mut name = raw.name.trim().to_string();
    if name.is_empty() {
        name = "未命名连接".to_string();
    }
    if !used_identities.insert((normalize_name(&name), kind.clone())) {
        return Err(SkipReason::Duplicate);
    }
    let renamed = used_names.contains(&normalize_name(&name));
    if renamed {
        name = unique_name(&name, used_names);
    }
    used_names.insert(normalize_name(&name));
    let (workspace_key, ungrouped) = resolve_workspace(raw.workspace.as_deref(), workspace_keys);
    Ok(PlannedConnection {
        name,
        connection_type: kind,
        params: raw.params.clone(),
        workspace_key,
        remark: raw.remark.clone(),
        selected_databases: raw.selected_databases.clone(),
        sync_enabled: raw.sync_enabled,
        renamed,
        ungrouped,
        needs_repair: one_core::storage::has_decrypt_failure_in_sensitive_fields(&raw.params),
    })
}

/// 只接受能被 [`ConnectionType`] 原样往返的类型名，防止 `from_str` 的静默回退。
pub(crate) fn canonical_kind(raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    let canonical = ConnectionType::from_str(trimmed).to_string();
    (canonical == trimmed).then_some(canonical)
}

/// 工作区名比对忽略大小写与首尾空白；不存在的工作区落到未分组。
fn resolve_workspace(
    workspace: Option<&str>,
    workspace_keys: &HashSet<String>,
) -> (Option<String>, bool) {
    let Some(workspace) = workspace else {
        return (None, false);
    };
    let key = normalize_name(workspace);
    if key.is_empty() {
        return (None, false);
    }
    if workspace_keys.contains(&key) {
        (Some(key), false)
    } else {
        (None, true)
    }
}

/// 名称判重用的规范化：忽略大小写与首尾空白。
pub(crate) fn normalize_name(name: &str) -> String {
    name.trim().to_lowercase()
}

/// 生成不与已有名称冲突的新名称，如 `开发库（2）`。
fn unique_name(base: &str, used: &HashSet<String>) -> String {
    let mut index = 2;
    loop {
        let candidate = format!("{base}（{index}）");
        if !used.contains(&normalize_name(&candidate)) {
            return candidate;
        }
        index += 1;
    }
}
