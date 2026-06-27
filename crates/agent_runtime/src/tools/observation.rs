//! 工具观测结果(Observation)。
//!
//! 工具执行的产物。**无论成功或失败都会写回会话历史**,供 Planner 据此决策。
//! 写回模型的文本通过 [`ToolObservation::model_text`] 截断,避免超长输出撑爆
//! 上下文;完整数据保留在 [`ObservationData`] 中,可另行持久化 / 展示。

use crate::error::ToolError;
use crate::ids::ToolCallId;
use crate::resource::ResourceId;
use crate::tools::spec::ToolName;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 观测数据载荷。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum ObservationData {
    /// 纯文本输出(命令 stdout、消息等)。
    Text(String),
    /// 结构化 JSON。
    Json(serde_json::Value),
    /// 表格(SQL 查询结果等)。
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<serde_json::Value>>,
    },
    /// 无数据载荷。
    Empty,
}

impl ObservationData {
    /// 渲染为可读文本,用于反馈给模型。
    pub fn to_text(&self) -> String {
        match self {
            ObservationData::Text(t) => t.clone(),
            ObservationData::Json(v) => {
                serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
            }
            ObservationData::Table { columns, rows } => {
                let mut out = columns.join(" | ");
                out.push('\n');
                for row in rows {
                    let cells: Vec<String> = row
                        .iter()
                        .map(|c| match c {
                            serde_json::Value::String(s) => s.clone(),
                            other => other.to_string(),
                        })
                        .collect();
                    out.push_str(&cells.join(" | "));
                    out.push('\n');
                }
                out
            }
            ObservationData::Empty => String::new(),
        }
    }
}

/// 一次工具调用的观测结果。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolObservation {
    pub call_id: ToolCallId,
    pub tool_name: ToolName,
    pub resource_id: Option<ResourceId>,
    pub success: bool,
    /// 简短摘要(一行),模型友好。
    pub summary: String,
    /// 完整数据载荷。
    pub data: ObservationData,
    pub started_at: DateTime<Utc>,
    pub finished_at: DateTime<Utc>,
}

impl ToolObservation {
    /// 构造成功观测;时间戳暂设为当前时刻,通常由 ToolRouter 在分发时覆盖。
    pub fn success(
        call_id: ToolCallId,
        tool_name: ToolName,
        summary: impl Into<String>,
        data: ObservationData,
    ) -> Self {
        let now = Utc::now();
        Self {
            call_id,
            tool_name,
            resource_id: None,
            success: true,
            summary: summary.into(),
            data,
            started_at: now,
            finished_at: now,
        }
    }

    /// 构造失败观测。
    pub fn failure(call_id: ToolCallId, tool_name: ToolName, message: impl Into<String>) -> Self {
        let now = Utc::now();
        let message = message.into();
        Self {
            call_id,
            tool_name,
            resource_id: None,
            success: false,
            summary: message.clone(),
            data: ObservationData::Text(message),
            started_at: now,
            finished_at: now,
        }
    }

    /// 由 [`ToolError`] 构造失败观测。
    pub fn from_error(call_id: ToolCallId, tool_name: ToolName, error: &ToolError) -> Self {
        Self::failure(call_id, tool_name, error.to_string())
    }

    pub fn with_resource(mut self, resource_id: Option<ResourceId>) -> Self {
        self.resource_id = resource_id;
        self
    }

    /// 执行耗时(毫秒)。
    pub fn duration_ms(&self) -> i64 {
        (self.finished_at - self.started_at).num_milliseconds()
    }

    /// 生成反馈给模型的文本,按 `max_bytes` 在字符边界处截断。
    pub fn model_text(&self, max_bytes: usize) -> String {
        let status = if self.success { "成功" } else { "失败" };
        let body = self.data.to_text();
        let mut text = if body.is_empty() || body == self.summary {
            format!("[{status}] {}", self.summary)
        } else {
            format!("[{status}] {}\n{}", self.summary, body)
        };
        truncate_on_char_boundary(&mut text, max_bytes);
        text
    }
}

/// 在不超过 `max_bytes` 的前提下,于字符边界处截断字符串,并追加省略标记。
fn truncate_on_char_boundary(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let marker = "…（已截断）";
    let budget = max_bytes.saturating_sub(marker.len());
    let mut end = budget.min(text.len());
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(marker);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs_with_body(body: &str) -> ToolObservation {
        ToolObservation::success(
            ToolCallId::from_string("call_1"),
            ToolName::new("echo"),
            "ok",
            ObservationData::Text(body.to_string()),
        )
    }

    #[test]
    fn model_text_truncates_at_char_boundary() {
        let obs = obs_with_body(&"中".repeat(100));
        let text = obs.model_text(40);
        assert!(text.len() <= 40 + "…（已截断）".len());
        // 不应在多字节字符中间切断(能正常作为 UTF-8 字符串持有即说明边界正确)。
        assert!(text.contains("已截断"));
    }

    #[test]
    fn model_text_keeps_short_output() {
        let obs = obs_with_body("hi");
        assert_eq!(obs.model_text(200), "[成功] ok\nhi");
    }
}
