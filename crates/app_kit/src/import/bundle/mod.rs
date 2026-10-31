//! 连接迁移包格式与导入规划。
//!
//! 迁移包是跨产品搬运连接的唯一载体：全量产品与两个独立产品读写同一份
//! `{version, exported_at, app_id, workspaces, connections}` JSON。本模块只做
//! 解析与纯函数规划，真正落库（仓库插入、事件广播）在 `import/mod.rs`。

pub(crate) mod plan;

use serde::{Deserialize, Serialize};

pub use plan::ImportReport;

/// 迁移包格式版本；读取比它更高的版本时直接拒绝，避免误读未来字段。
pub const BUNDLE_VERSION: u32 = 1;

/// 跨产品搬运的连接与工作区快照。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConnectionBundle {
    /// 格式版本，当前为 [`BUNDLE_VERSION`]。
    pub version: u32,
    /// 导出时间（Unix 秒），仅用于展示。
    #[serde(default)]
    pub exported_at: i64,
    /// 导出方产品 id（如 `navop` / `navop-shell`）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub app_id: Option<String>,
    /// 导出方的工作区；导入时按名称匹配，缺失则新建。
    #[serde(default)]
    pub workspaces: Vec<BundleWorkspace>,
    /// 导出方的连接。
    #[serde(default)]
    pub connections: Vec<BundleConnection>,
}

/// 迁移包中的工作区。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleWorkspace {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

/// 迁移包中的连接；`params` 是从 `StoredConnection` 原样搬运的 JSON 字符串。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BundleConnection {
    pub name: String,
    pub connection_type: String,
    #[serde(default)]
    pub params: String,
    /// 所属工作区名称，不是 id：跨产品时 id 会重新分配。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workspace: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub remark: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub selected_databases: Option<String>,
    #[serde(default = "default_sync_enabled")]
    pub sync_enabled: bool,
}

fn default_sync_enabled() -> bool {
    true
}

#[cfg(test)]
mod tests;
