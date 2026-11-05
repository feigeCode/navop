use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `contributes.tools[]` 的工具风险等级。
///
/// 与 `agent_runtime::RiskLevel` 的语义对齐（read 自动执行，其余在 Manual
/// 模式需要审批），但清单侧保留自己的拼写，避免 extension-runtime 反向
/// 依赖 agent-runtime。
#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ToolRisk {
    Read,
    Low,
    Medium,
    High,
    Critical,
}

/// `contributes.tools[].annotations`：给宿主与模型的附加说明，全部可选。
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ToolAnnotations {
    #[serde(default, rename = "readOnly")]
    pub read_only: bool,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub idempotent: bool,
    #[serde(default, rename = "openWorld")]
    pub open_world: bool,
    #[serde(default, rename = "supportsParallel")]
    pub supports_parallel: bool,
}

/// `contributes.tools[]`：扩展向宿主 AI 声明的单个工具。
///
/// 一个工具 = provider 的一个方法；执行期宿主按 `backend` 别名解析 runtime，
/// 用 `method` 调 `resource.invoke`，与页面 `navop.resource.invoke` 同一条通道。
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct ToolContrib {
    /// 扩展内唯一，`[a-z0-9._-]` 且 ≤64；建议 `域.动作`（`containers.list`）。
    pub id: String,
    /// 人类可读短标题（≤40），出现在审批卡与工具列表。
    pub title: String,
    /// 给模型看的说明（≤1024）。
    pub description: String,
    /// `shellApp.backends` 的别名。
    pub backend: String,
    /// provider 方法名，原样透传（≤128）。
    pub method: String,
    /// JSON Schema 受限子集（见 `tools_validation`）；默认空对象 schema。
    #[serde(default = "default_input_schema", rename = "inputSchema")]
    pub input_schema: Value,
    pub risk: ToolRisk,
    #[serde(default)]
    pub annotations: ToolAnnotations,
    /// 是否需要扩展连接；true 时执行期从 AI 会话的目标资源取连接。
    #[serde(default)]
    pub connection: bool,
    #[serde(default = "default_timeout_ms", rename = "timeoutMs")]
    pub timeout_ms: u64,
    #[serde(default = "default_max_result_bytes", rename = "maxResultBytes")]
    pub max_result_bytes: u64,
}

fn default_input_schema() -> Value {
    serde_json::json!({ "type": "object", "properties": {} })
}

fn default_timeout_ms() -> u64 {
    30_000
}

fn default_max_result_bytes() -> u64 {
    131_072
}
