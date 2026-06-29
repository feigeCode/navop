//! 运行时会话历史。
//!
//! 记录一轮 / 多轮交互中的用户消息、助手消息、工具调用与观测结果,作为
//! 构造模型输入的事实来源。计划(`Plan`)不进历史,而是单独保存在
//! [`SessionState`](crate::runtime::SessionState) 中,以保持 `history -> tools`
//! 的单向依赖。
//!
//! 写回模型的观测会经 [`ToolObservation::model_text`] 截断;此处通过
//! `max_observation_bytes` 控制阈值。

use crate::runtime::InputImage;
use crate::tools::{ToolCall, ToolObservation};
use serde::{Deserialize, Serialize};

/// 历史中的单条记录。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum HistoryItem {
    /// 用户输入(文本 + 可选附带图片)。
    User {
        text: String,
        images: Vec<InputImage>,
    },
    /// 助手文本回复。
    Assistant(String),
    /// 助手文本回复及其独立 reasoning。reasoning 只用于 UI 恢复,不回灌模型上下文。
    AssistantWithReasoning { text: String, reasoning: String },
    /// 系统提示 / 内部说明。
    System(String),
    /// 一次工具调用。
    ToolCall(ToolCall),
    /// 工具观测结果。
    Observation(ToolObservation),
}

/// 默认最多保留的历史条目数。
const DEFAULT_MAX_ITEMS: usize = 200;
/// 默认单条观测反馈给模型时的最大字节数。
const DEFAULT_MAX_OBSERVATION_BYTES: usize = 4096;

/// 会话历史。可克隆,便于生成快照交给 Planner。
#[derive(Clone, Debug)]
pub struct RuntimeHistory {
    items: Vec<HistoryItem>,
    max_items: usize,
    max_observation_bytes: usize,
}

impl Default for RuntimeHistory {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeHistory {
    pub fn new() -> Self {
        Self {
            items: Vec::new(),
            max_items: DEFAULT_MAX_ITEMS,
            max_observation_bytes: DEFAULT_MAX_OBSERVATION_BYTES,
        }
    }

    pub fn with_limits(max_items: usize, max_observation_bytes: usize) -> Self {
        Self {
            items: Vec::new(),
            max_items: max_items.max(1),
            max_observation_bytes,
        }
    }

    /// 用既有条目构造历史(用于从持久化快照恢复),沿用默认上限并裁剪超额条目。
    pub fn from_items(items: Vec<HistoryItem>) -> Self {
        let mut history = Self::new();
        history.items = items;
        if history.items.len() > history.max_items {
            let overflow = history.items.len() - history.max_items;
            history.items.drain(0..overflow);
        }
        history
    }

    /// 写入一条记录,超出 `max_items` 时丢弃最旧的记录。
    pub fn record(&mut self, item: HistoryItem) {
        self.items.push(item);
        if self.items.len() > self.max_items {
            let overflow = self.items.len() - self.max_items;
            self.items.drain(0..overflow);
        }
    }

    pub fn record_user(&mut self, text: impl Into<String>) {
        self.record(HistoryItem::User {
            text: text.into(),
            images: Vec::new(),
        });
    }

    /// 写入一条带图片的用户输入。
    pub fn record_user_with_images(&mut self, text: impl Into<String>, images: Vec<InputImage>) {
        self.record(HistoryItem::User {
            text: text.into(),
            images,
        });
    }

    pub fn record_assistant(&mut self, text: impl Into<String>) {
        self.record(HistoryItem::Assistant(text.into()));
    }

    pub fn record_assistant_with_reasoning(
        &mut self,
        text: impl Into<String>,
        reasoning: impl Into<String>,
    ) {
        let text = text.into();
        let reasoning = reasoning.into();
        if reasoning.is_empty() {
            self.record_assistant(text);
        } else {
            self.record(HistoryItem::AssistantWithReasoning { text, reasoning });
        }
    }

    pub fn record_system(&mut self, text: impl Into<String>) {
        self.record(HistoryItem::System(text.into()));
    }

    pub fn record_tool_call(&mut self, call: ToolCall) {
        self.record(HistoryItem::ToolCall(call));
    }

    pub fn record_observation(&mut self, observation: ToolObservation) {
        self.record(HistoryItem::Observation(observation));
    }

    pub fn items(&self) -> &[HistoryItem] {
        &self.items
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }

    pub fn max_observation_bytes(&self) -> usize {
        self.max_observation_bytes
    }

    /// 返回最近一次观测。
    pub fn last_observation(&self) -> Option<&ToolObservation> {
        self.items.iter().rev().find_map(|item| match item {
            HistoryItem::Observation(obs) => Some(obs),
            _ => None,
        })
    }

    /// 最近一条用户输入。
    pub fn last_user_message(&self) -> Option<&str> {
        self.items.iter().rev().find_map(|item| match item {
            HistoryItem::User { text, .. } => Some(text.as_str()),
            _ => None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ids::ToolCallId;
    use crate::tools::{ObservationData, ToolName, ToolObservation};

    #[test]
    fn evicts_oldest_when_over_capacity() {
        let mut history = RuntimeHistory::with_limits(2, 1024);
        history.record_user("a");
        history.record_user("b");
        history.record_user("c");
        assert_eq!(history.len(), 2);
        assert_eq!(history.last_user_message(), Some("c"));
    }

    #[test]
    fn finds_last_observation() {
        let mut history = RuntimeHistory::new();
        history.record_user("hi");
        history.record_observation(ToolObservation::success(
            ToolCallId::from_string("call_1"),
            ToolName::new("echo"),
            "ok",
            ObservationData::Text("done".into()),
        ));
        assert!(history.last_observation().is_some());
        assert_eq!(history.last_observation().unwrap().summary, "ok");
    }
}
