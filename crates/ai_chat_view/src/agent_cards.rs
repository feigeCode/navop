//! Agent 运行时卡片:把 Planner 计划与工具执行渲染成 codex 风格卡片。
//!
//! 复用 `ai_chat_view` 既有的卡片机制([`CardRegistry`]):
//! - `agent.plan`:计划清单(目标 + 分步 + 状态 + 风险);
//! - `agent.tool`:一次工具执行(调用 + 观测结果合并为一张卡片,随事件演进)。
//!
//! 卡片的数据载体是消息 `content` 中的 JSON;[`AgentTranscript`](crate::agent_transcript)
//! 负责在收到 [`RuntimeEvent`](agent_runtime::RuntimeEvent) 时写入 / 更新这些 JSON。
//! 这里定义共享的数据结构(序列化契约)与渲染实现,二者共用同一份 schema。

use crate::card::{CardMessage, CardRegistry, ChatCard};
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

/// 工具执行卡片的 `kind`。
pub const TOOL_CARD: &str = "agent.tool";

// ============================================================================
// 数据契约(reducer 写入 / 卡片读取共用)
// ============================================================================

/// 计划卡片数据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanCardData {
    pub goal: String,
    pub status: String,
    pub steps: Vec<PlanStepData>,
}

/// 计划中的一步。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlanStepData {
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: String,
    #[serde(default)]
    pub risk: String,
    #[serde(default)]
    pub tool: Option<String>,
}

/// 工具执行卡片数据(调用 + 观测合并)。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolCardData {
    pub call_id: String,
    pub tool_name: String,
    /// 是否仍在执行。
    pub running: bool,
    /// 执行结果(完成后才有):成功 / 失败。
    #[serde(default)]
    pub success: Option<bool>,
    /// 观测摘要。
    #[serde(default)]
    pub summary: String,
    /// 观测数据文本(可能较长,展示时截断)。
    #[serde(default)]
    pub data_text: String,
}

impl PlanCardData {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }

    /// 进度统计:`(已离开 pending/running 的步骤数, 总步骤数)`。
    pub fn progress(&self) -> (usize, usize) {
        let total = self.steps.len();
        let done = self
            .steps
            .iter()
            .filter(|s| !matches!(s.status.as_str(), "pending" | "running"))
            .count();
        (done, total)
    }
}

impl ToolCardData {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

// ============================================================================
// 渲染
// ============================================================================

/// 渲染计划分步列表(供输入框上方的 Tasks Popover 复用)。
pub fn render_plan_list(data: &PlanCardData, cx: &mut App) -> AnyElement {
    let rows: Vec<AnyElement> = data
        .steps
        .iter()
        .enumerate()
        .map(|(i, step)| {
            let (glyph, color) = step_status_glyph(&step.status, cx);
            h_flex()
                .w_full()
                .gap_2()
                .items_start()
                .child(div().text_color(color).child(glyph))
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_0p5()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().foreground)
                                .child(format!("{}. {}", i + 1, step.title)),
                        )
                        .when_some(
                            (!step.description.is_empty()).then_some(step.description.clone()),
                            |this, desc| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(desc),
                                )
                            },
                        ),
                )
                .when_some(risk_badge(&step.risk, cx), |this, badge| this.child(badge))
                .into_any_element()
        })
        .collect();

    v_flex()
        .w_full()
        .gap_2()
        .p_3()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(
            h_flex()
                .w_full()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child("执行计划"),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(plan_status_label(&data.status)),
                ),
        )
        .child(v_flex().w_full().gap_1p5().children(rows))
        .into_any_element()
}

/// 工具执行卡片渲染器。
struct ToolCard {
    expanded: Arc<Mutex<HashSet<String>>>,
}

impl ToolCard {
    fn new() -> Self {
        Self {
            expanded: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn is_expanded(&self, message_id: &str) -> bool {
        self.expanded
            .lock()
            .map(|ids| ids.contains(message_id))
            .unwrap_or(false)
    }
}

impl ChatCard for ToolCard {
    fn kind(&self) -> &'static str {
        TOOL_CARD
    }

    fn render(&self, msg: &CardMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(data) = ToolCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };

        let (status_glyph, status_color) = if data.running {
            ("●", cx.theme().muted_foreground)
        } else if data.success == Some(true) {
            ("✓", cx.theme().success)
        } else if data.success == Some(false) {
            ("✗", cx.theme().danger)
        } else {
            ("•", cx.theme().muted_foreground)
        };
        let has_details = !data.summary.is_empty() || !data.data_text.is_empty();
        let expanded = has_details && self.is_expanded(msg.id);
        let toggle_state = self.expanded.clone();
        let message_id = msg.id.to_string();
        let toggle_id = SharedString::from(format!("agent-tool-card-toggle-{}", data.call_id));
        let hover_bg = cx.theme().background;

        let mut card = v_flex()
            .w_full()
            .gap_2()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().muted)
            .child(
                h_flex()
                    .id(toggle_id)
                    .w_full()
                    .gap_2()
                    .items_center()
                    .px_1()
                    .py_1()
                    .rounded_md()
                    .when(has_details, |this| {
                        this.cursor_pointer()
                            .hover(move |this| this.bg(hover_bg))
                            .on_click(move |_, _, cx| {
                                if let Ok(mut expanded_ids) = toggle_state.lock()
                                    && !expanded_ids.insert(message_id.clone())
                                {
                                    expanded_ids.remove(&message_id);
                                }
                                cx.refresh_windows();
                            })
                    })
                    .child(div().text_color(status_color).child(status_glyph))
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().foreground)
                            .child(format!("工具 · {}", data.tool_name)),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(tool_status_label(&data)),
                    )
                    .when(has_details, |this| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(if expanded {
                                    "收起详情"
                                } else {
                                    "展开详情"
                                }),
                        )
                    }),
            );

        if expanded && !data.summary.is_empty() {
            card = card.child(
                div()
                    .px_1()
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(data.summary.clone()),
            );
        }

        if expanded && !data.data_text.is_empty() {
            let preview = truncate_chars(&data.data_text, 600);
            card = card.child(
                div()
                    .w_full()
                    .p_2()
                    .rounded_md()
                    .bg(cx.theme().background)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(preview),
            );
        }

        card.into_any_element()
    }
}

// ============================================================================
// 渲染辅助
// ============================================================================

fn fallback(content: &str, cx: &App) -> AnyElement {
    div()
        .w_full()
        .p_2()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(format!("[无法解析的 Agent 卡片] {content}"))
        .into_any_element()
}

/// 步骤状态字形 + 颜色。
fn step_status_glyph(status: &str, cx: &App) -> (&'static str, gpui::Hsla) {
    match status {
        "running" => ("◐", cx.theme().muted_foreground),
        "observed" => ("◉", cx.theme().foreground),
        "completed" => ("✓", cx.theme().success),
        "failed" => ("✗", cx.theme().danger),
        "skipped" => ("–", cx.theme().muted_foreground),
        _ => ("○", cx.theme().muted_foreground),
    }
}

fn plan_status_label(status: &str) -> &'static str {
    match status {
        "draft" => "草稿",
        "running" => "执行中",
        "waiting_user" => "等待用户",
        "completed" => "已完成",
        "failed" => "已失败",
        _ => "",
    }
}

fn tool_status_label(data: &ToolCardData) -> &'static str {
    if data.running {
        "执行中…"
    } else if data.success == Some(false) {
        "失败"
    } else {
        "已完成"
    }
}

/// 高于 Read 的风险才显示徽标。
fn risk_badge(risk: &str, cx: &App) -> Option<AnyElement> {
    let (label, color) = match risk {
        "medium" => ("中风险", cx.theme().warning),
        "high" => ("高风险", cx.theme().danger),
        "critical" => ("危险", cx.theme().danger),
        _ => return None,
    };
    Some(
        div()
            .px_1p5()
            .rounded_md()
            .text_xs()
            .text_color(color)
            .border_1()
            .border_color(color)
            .child(label)
            .into_any_element(),
    )
}

/// 按字符截断,超出加省略号。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str("…（已截断）");
    out
}

/// 注册 Agent 运行时卡片到全局注册表。
pub fn register_agent_cards(cx: &mut App) {
    CardRegistry::register_global(cx, Arc::new(ToolCard::new()));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plan_card_data_roundtrips() {
        let data = PlanCardData {
            goal: "排查慢查询".into(),
            status: "running".into(),
            steps: vec![PlanStepData {
                title: "查看连接数".into(),
                description: "SHOW PROCESSLIST".into(),
                status: "pending".into(),
                risk: "read".into(),
                tool: Some("sql".into()),
            }],
        };
        let json = data.to_json();
        let back = PlanCardData::from_json(&json).expect("parse");
        assert_eq!(back.goal, "排查慢查询");
        assert_eq!(back.steps.len(), 1);
        assert_eq!(back.steps[0].tool.as_deref(), Some("sql"));
    }

    #[test]
    fn tool_card_data_roundtrips() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            running: false,
            success: Some(true),
            summary: "echo: hi".into(),
            data_text: "hi".into(),
        };
        let back = ToolCardData::from_json(&data.to_json()).expect("parse");
        assert_eq!(back.call_id, "call_1");
        assert_eq!(back.success, Some(true));
    }

    #[test]
    fn truncate_adds_marker() {
        let s = "a".repeat(10);
        assert_eq!(truncate_chars(&s, 100), s);
        assert!(truncate_chars(&s, 3).contains("已截断"));
    }
}
