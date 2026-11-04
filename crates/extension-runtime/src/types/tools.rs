use crate::extension::manifest::{ToolAnnotations, ToolRisk};

/// 一个已注册的扩展工具（`contributes.tools[]`）。
///
/// 清单侧 `ToolContrib` 的运行期形态：backend 别名已解析成 runtime key
/// （`{extension}::{runtime}`），执行器不再碰清单。
#[derive(Debug, Clone, PartialEq)]
pub struct RegisteredToolContribution {
    pub extension_id: String,
    pub id: String,
    pub title: String,
    pub description: String,
    /// `shellApp.backends[alias]` 解析出的 runtime key（`{extension}::{runtime}`）。
    pub runtime_key: String,
    /// provider 方法名，执行期原样透传。
    pub method: String,
    pub input_schema: serde_json::Value,
    pub risk: ToolRisk,
    pub annotations: ToolAnnotations,
    pub connection: bool,
    pub timeout_ms: u64,
    pub max_result_bytes: u64,
}
