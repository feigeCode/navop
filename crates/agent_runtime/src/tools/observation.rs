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

/// 一次工具调用对单个文件的改动。
///
/// 携带**改动前后的完整文本**,而不是渲染好的 diff:diff 是展示层的选择
/// (单列/双列、留几行上下文),不该由产出方定死。写入方应保证 `new_text` 与
/// `old_text` 是同一份文件的完整内容,展示层据此自己算行级差异。
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileChange {
    /// 被改动的文件路径(协议通常给绝对路径)。
    pub path: String,
    /// 改动前的完整内容;`None` 表示新建文件。
    #[serde(default)]
    pub old_text: Option<String>,
    /// 改动后的完整内容。
    pub new_text: String,
}

impl FileChange {
    /// 新建文件(无旧内容)。
    pub fn created(path: impl Into<String>, new_text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            old_text: None,
            new_text: new_text.into(),
        }
    }

    /// 就地修改。
    pub fn modified(
        path: impl Into<String>,
        old_text: impl Into<String>,
        new_text: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            old_text: Some(old_text.into()),
            new_text: new_text.into(),
        }
    }

    /// 是否新建文件。
    pub fn is_created(&self) -> bool {
        self.old_text.is_none()
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
    /// 本次调用改动的文件。空表示这次调用没有文件改动(或产出方没提供)。
    #[serde(default)]
    pub file_changes: Vec<FileChange>,
    /// 这次调用**指向什么**——由产出方声明,不由展示层从工具名反推。
    ///
    /// 外部 agent 走 ACP 时,调用开始的 `tool_call` 只带工具名(占位),真实目标
    /// (文件路径 / 命令 / 查询)要等 `tool_call_update` 才到。展示层要的「这一行
    /// 是哪个文件 / 哪条命令」就落在这里;拿不到声明的产出方留空,展示层退回
    /// 各自的旧路径。`None` 与空串等价(空串由 [`Self::with_target`] 折成 `None`)。
    #[serde(default)]
    pub target: Option<String>,
    /// 产出方补发的**真实入参**(调用开始时可缺)。
    ///
    /// 与 `target` 同样的来由:ACP 的 `tool_call` 里 `rawInput` 是空对象,真实入参
    /// 在后续 `tool_call_update` 才给。展示层只在自持入参为空时才用它,不会覆盖
    /// 已经拿到的入参。
    #[serde(default)]
    pub arguments: Option<serde_json::Value>,
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
            file_changes: Vec::new(),
            target: None,
            arguments: None,
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
            file_changes: Vec::new(),
            target: None,
            arguments: None,
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

    /// 附加本次调用改动的文件。
    pub fn with_file_changes(mut self, file_changes: Vec<FileChange>) -> Self {
        self.file_changes = file_changes;
        self
    }

    /// 声明这次调用指向的目标(文件路径 / 命令 / 查询)。空串折成 `None`。
    pub fn with_target(mut self, target: Option<String>) -> Self {
        self.target = target.filter(|target| !target.trim().is_empty());
        self
    }

    /// 补发调用开始时缺失的真实入参。
    ///
    /// `Null` 与空对象都视为「没有入参」——那是「调用刚开始、agent 还没填」的
    /// 形态,不是一份有效入参,不能拿它覆盖展示层已有的入参。
    pub fn with_arguments(mut self, arguments: Option<serde_json::Value>) -> Self {
        self.arguments = arguments.filter(|arguments| match arguments {
            serde_json::Value::Null => false,
            serde_json::Value::Object(object) => !object.is_empty(),
            _ => true,
        });
        self
    }

    /// 是否带文件改动。
    pub fn has_file_changes(&self) -> bool {
        !self.file_changes.is_empty()
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

    #[test]
    fn blank_target_is_stored_as_absent() {
        let obs = obs_with_body("hi").with_target(Some("   ".to_string()));
        assert_eq!(None, obs.target);

        let obs = obs_with_body("hi").with_target(Some("src/lib.rs".to_string()));
        assert_eq!(Some("src/lib.rs".to_string()), obs.target);
    }

    #[test]
    fn placeholder_arguments_do_not_count_as_real_input() {
        // ACP 的 pending `tool_call` 给的就是这两种形态:都没内容,不能拿去覆盖入参。
        assert_eq!(
            None,
            obs_with_body("hi")
                .with_arguments(Some(serde_json::Value::Null))
                .arguments
        );
        assert_eq!(
            None,
            obs_with_body("hi")
                .with_arguments(Some(serde_json::json!({})))
                .arguments
        );

        let real = serde_json::json!({"filePath": "src/lib.rs"});
        assert_eq!(
            Some(real.clone()),
            obs_with_body("hi").with_arguments(Some(real)).arguments
        );
    }
}
