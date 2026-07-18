//! Agent 运行时卡片:把 Planner 计划与工具执行渲染成 codex 风格卡片。
//!
//! 复用 `ai_chat_view` 既有的卡片机制([`CardRegistry`]):
//! - `agent.plan`:计划清单(目标 + 分步 + 状态 + 风险);
//! - `agent.tool`:一次工具执行(调用 + 观测结果合并为一张卡片,随事件演进)。
//! - `agent.confirm`:工具执行前的人工确认请求。
//!
//! 卡片的数据载体是消息 `content` 中的 JSON;[`AgentTranscript`](crate::agent_transcript)
//! 负责在收到 [`RuntimeEvent`](agent_runtime::RuntimeEvent) 时写入 / 更新这些 JSON。
//! 这里定义共享的数据结构(序列化契约)与渲染实现,二者共用同一份 schema。

use crate::card::{CardMessage, CardRegistry, ChatCard};
use crate::theme::{active_agent_chat_theme, themed_markdown};
use gpui::prelude::FluentBuilder;
use gpui::{
    Action, Anchor, AnyElement, App, AppContext, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, Size,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};

const MAX_TOOL_OUTPUT_JSON_CHARS: usize = 4000;
const MAX_TERMINAL_TOOL_OUTPUT_CHARS: usize = 64_000;
const TOOL_JSON_MIN_ROWS: usize = 6;
const TOOL_JSON_MAX_ROWS: usize = 14;
const TOOL_JSON_LINE_HEIGHT_PX: f32 = 18.0;
const TOOL_JSON_VERTICAL_PADDING_PX: f32 = 20.0;

struct ToolJsonInputState {
    input: Entity<InputState>,
    value: String,
}

/// 工具执行卡片的 `kind`。
pub const TOOL_CARD: &str = "agent.tool";
/// 子代理任务卡片的 `kind`。
pub const SUBAGENT_CARD: &str = "agent.subagent";
/// 工具确认卡片的 `kind`。
pub const TOOL_CONFIRM_CARD: &str = "agent.confirm";
/// ACP 协议权限请求卡片的 `kind`。
pub const ACP_PERMISSION_CARD: &str = "acp.permission";

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
    /// 目标资源 id,用于多资源任务分组展示。
    #[serde(default)]
    pub target_id: Option<String>,
    /// 目标资源展示名。缺失时 UI 可回退到 `target_id`。
    #[serde(default)]
    pub target_label: Option<String>,
    /// 工具入参摘要,用于卡片头部。
    #[serde(default)]
    pub input_summary: String,
    /// 脱敏后的工具入参 JSON,用于展开详情。
    #[serde(default)]
    pub input_json: String,
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

/// 子代理任务卡片数据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SubAgentCardData {
    pub subagent_id: String,
    pub name: String,
    pub task: String,
    /// 是否仍在执行。
    pub running: bool,
    /// 执行结果(完成后才有):成功 / 失败。
    #[serde(default)]
    pub success: Option<bool>,
    /// 最近进展或最终摘要。
    #[serde(default)]
    pub summary: String,
}

/// 工具执行确认卡片数据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolConfirmCardData {
    pub call_id: String,
    pub tool_name: String,
    /// 批量审批中的每个工具调用。为空表示旧的单工具确认卡。
    #[serde(default)]
    pub items: Vec<ToolConfirmItemData>,
    /// 工具入参摘要,用于确认卡片头部。
    #[serde(default)]
    pub input_summary: String,
    /// 脱敏后的工具入参 JSON。
    #[serde(default)]
    pub input_json: String,
    pub question: String,
    #[serde(default = "default_tool_confirm_status")]
    pub status: String,
}

/// 批量工具确认卡片中的单个待执行项。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ToolConfirmItemData {
    pub call_id: String,
    pub tool_name: String,
    #[serde(default)]
    pub input_summary: String,
    #[serde(default)]
    pub input_json: String,
}

/// ACP 权限请求卡片数据。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpPermissionCardData {
    pub request_id: String,
    pub session_id: String,
    pub tool_call_id: String,
    pub tool_name: String,
    pub summary: String,
    #[serde(default)]
    pub details_json: String,
    pub options: Vec<AcpPermissionOptionData>,
    #[serde(default = "default_tool_confirm_status")]
    pub status: String,
    #[serde(default)]
    pub selected_option_name: String,
}

/// ACP 协议原样提供的权限选项。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AcpPermissionOptionData {
    pub option_id: String,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ai_chat_view, no_json)]
pub struct ApproveToolCall {
    pub call_id: String,
}

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ai_chat_view, no_json)]
pub struct RejectToolCall {
    pub call_id: String,
}

#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ai_chat_view, no_json)]
pub struct SelectAcpPermissionOption {
    pub request_id: String,
    pub option_id: String,
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

impl SubAgentCardData {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

impl ToolConfirmCardData {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }
    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

impl AcpPermissionCardData {
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    pub fn from_json(s: &str) -> Option<Self> {
        serde_json::from_str(s).ok()
    }
}

fn default_tool_confirm_status() -> String {
    "pending".into()
}

// ============================================================================
// 渲染
// ============================================================================

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

    fn render(&self, msg: &CardMessage, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let Some(data) = ToolCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };

        let (status_glyph, status_color) = if data.running {
            ("●", theme.muted_foreground)
        } else if data.success == Some(true) {
            ("✓", cx.theme().success)
        } else if data.success == Some(false) {
            ("✗", cx.theme().danger)
        } else {
            ("•", theme.muted_foreground)
        };
        let has_details =
            !data.input_json.is_empty() || !data.summary.is_empty() || !data.data_text.is_empty();
        let expanded = has_details && self.is_expanded(msg.id);
        let toggle_state = self.expanded.clone();
        let message_id = msg.id.to_string();
        let toggle_id = SharedString::from(format!("agent-tool-card-toggle-{}", data.call_id));
        let hover_bg = theme.panel_hover;

        let mut card = v_flex()
            .debug_selector(|| "agent-tool-card".to_string())
            .w_full()
            .min_w_0()
            .gap_2()
            .p_2()
            .rounded_lg()
            .border_1()
            .border_color(theme.border)
            .bg(theme.panel)
            .child(
                h_flex()
                    .id(toggle_id)
                    .w_full()
                    .min_w_0()
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
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_color(status_color)
                            .child(status_glyph),
                    )
                    .child(tool_card_title(&data, cx))
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(tool_status_label(&data)),
                    )
                    .when(has_details, |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(if expanded {
                                    t!("AgentUi.collapse_details").to_string()
                                } else {
                                    t!("AgentUi.expand_details").to_string()
                                }),
                        )
                    }),
            );

        if expanded && !data.input_json.is_empty() {
            card = card.child(tool_card_json_block(
                t!("AgentUi.input").to_string(),
                SharedString::from(format!("agent-tool-input-{}", data.call_id)),
                data.input_json.clone(),
                window,
                cx,
            ));
        }

        if expanded {
            let terminal_output = terminal_exec_output_text(&data);
            if !terminal_output.is_empty() {
                card = card.child(tool_card_text_block(
                    t!("AgentUi.output").to_string(),
                    SharedString::from(format!("agent-tool-output-{}", data.call_id)),
                    terminal_output,
                    window,
                    cx,
                ));
            } else {
                let output = distinct_tool_output_json(&data);
                if !output.is_empty() {
                    card = card.child(tool_card_json_block(
                        t!("AgentUi.output").to_string(),
                        SharedString::from(format!("agent-tool-output-{}", data.call_id)),
                        output,
                        window,
                        cx,
                    ));
                }
            }
        }

        card.into_any_element()
    }
}

fn terminal_exec_output_text(data: &ToolCardData) -> String {
    if !is_terminal_exec_tool(&data.tool_name) {
        return String::new();
    }
    let source = if data.data_text.trim().is_empty() {
        data.summary.trim()
    } else {
        data.data_text.trim()
    };
    let output = serde_json::from_str::<serde_json::Value>(source)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .and_then(|object| object.get("output"))
                .and_then(|output| output.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();
    if output.trim().is_empty() {
        return String::new();
    }
    truncate_chars(&output, MAX_TERMINAL_TOOL_OUTPUT_CHARS)
}

/// 子代理任务卡片渲染器。
struct SubAgentCard {
    expanded: Arc<Mutex<HashSet<String>>>,
}

impl SubAgentCard {
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

impl ChatCard for SubAgentCard {
    fn kind(&self) -> &'static str {
        SUBAGENT_CARD
    }

    fn render(&self, msg: &CardMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
        let Some(data) = SubAgentCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };
        render_subagent_card(
            &data,
            msg.id,
            self.is_expanded(msg.id),
            self.expanded.clone(),
            cx,
        )
    }
}

struct ToolConfirmCard;

impl ChatCard for ToolConfirmCard {
    fn kind(&self) -> &'static str {
        TOOL_CONFIRM_CARD
    }

    fn render(&self, msg: &CardMessage, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let Some(data) = ToolConfirmCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };
        let is_pending = data.status == "pending";
        let call_id = data.call_id.clone();
        let approve_call_id = call_id.clone();
        let reject_call_id = call_id.clone();

        let mut card = v_flex()
            .w_full()
            .min_w_0()
            .items_stretch()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().warning.opacity(0.35))
            .bg(theme.panel)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().text_lg().text_color(cx.theme().danger).child("?"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .truncate()
                            .child(confirm_card_header(&data)),
                    ),
            )
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .gap_2()
                    .items_center()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(confirm_card_title(&data)),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(confirm_status_color(&data.status, cx))
                            .child(confirm_status_label(&data.status)),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(
                        themed_markdown(
                            SharedString::from(format!("agent-tool-confirm-{}", msg.id)),
                            data.question.clone(),
                            &theme,
                        )
                        .selectable(true),
                    ),
            );

        if data.items.len() > 1 {
            card = card.child(render_confirm_batch_items(&data, cx));
        }

        if !data.input_json.is_empty() {
            card = card.child(tool_card_json_block(
                t!("AgentUi.pending_input").to_string(),
                SharedString::from(format!("agent-tool-confirm-input-{}", msg.id)),
                data.input_json.clone(),
                window,
                cx,
            ));
        }

        if is_pending {
            card = card.child(
                h_flex()
                    .w_full()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new(SharedString::from(format!("reject-tool-{call_id}")))
                            .debug_selector(|| "agent-tool-reject".to_string())
                            .with_size(Size::Small)
                            .danger()
                            .label(t!("AgentUi.reject").to_string())
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(RejectToolCall {
                                        call_id: reject_call_id.clone(),
                                    }),
                                    cx,
                                );
                            }),
                    )
                    .child(
                        Button::new(SharedString::from(format!("approve-tool-{call_id}")))
                            .debug_selector(|| "agent-tool-approve".to_string())
                            .with_size(Size::Small)
                            .primary()
                            .label(t!("AgentUi.execute").to_string())
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(ApproveToolCall {
                                        call_id: approve_call_id.clone(),
                                    }),
                                    cx,
                                );
                            }),
                    ),
            );
        } else {
            card = card.child(
                h_flex().w_full().justify_end().gap_2().child(
                    Button::new(SharedString::from(format!("resolved-tool-{call_id}")))
                        .with_size(Size::Small)
                        .disabled(true)
                        .label(confirm_status_label(&data.status)),
                ),
            );
        }

        card.into_any_element()
    }
}

struct AcpPermissionCard;

impl ChatCard for AcpPermissionCard {
    fn kind(&self) -> &'static str {
        ACP_PERMISSION_CARD
    }

    fn render(&self, msg: &CardMessage, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let Some(data) = AcpPermissionCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };
        let pending = data.status == "pending";
        let mut card = v_flex()
            .w_full()
            .min_w_0()
            .gap_2()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().warning.opacity(0.35))
            .bg(theme.panel)
            .child(
                h_flex()
                    .w_full()
                    .min_w_0()
                    .items_center()
                    .gap_2()
                    .child(div().text_lg().text_color(cx.theme().warning).child("?"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .child(t!("AgentUi.acp_permission_request").to_string()),
                    )
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(acp_permission_status_color(&data.status, cx))
                            .child(acp_permission_status_label(&data)),
                    ),
            )
            .child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(data.summary.clone()),
            );

        if !data.details_json.is_empty() {
            card = card.child(
                div()
                    .debug_selector(|| "acp-permission-details".to_string())
                    .w_full()
                    .min_w_0()
                    .self_stretch()
                    .child(tool_card_json_block(
                        t!("AgentUi.request_details").to_string(),
                        SharedString::from(format!("acp-permission-details-{}", msg.id)),
                        data.details_json.clone(),
                        window,
                        cx,
                    )),
            );
        }

        if pending {
            card = card.child(render_acp_permission_actions(&data));
        } else {
            card = card.child(
                h_flex().w_full().justify_end().child(
                    Button::new(SharedString::from(format!(
                        "resolved-acp-permission-{}",
                        data.request_id
                    )))
                    .with_size(Size::Small)
                    .disabled(true)
                    .label(acp_permission_status_label(&data)),
                ),
            );
        }

        card.into_any_element()
    }
}

fn render_acp_permission_actions(data: &AcpPermissionCardData) -> AnyElement {
    let allow = preferred_acp_permission_option(&data.options, "allow_once", "allow");
    let reject = preferred_acp_permission_option(&data.options, "reject_once", "reject");
    let primary_ids = [allow, reject]
        .into_iter()
        .flatten()
        .map(|option| option.option_id.as_str())
        .collect::<HashSet<_>>();
    let additional = data
        .options
        .iter()
        .filter(|option| !primary_ids.contains(option.option_id.as_str()))
        .cloned()
        .collect::<Vec<_>>();
    let mut actions = h_flex()
        .debug_selector(|| "acp-permission-actions".to_string())
        .w_full()
        .min_w_0()
        .justify_end()
        .gap_2();

    if let Some(option) = reject {
        actions = actions.child(acp_permission_option_button(&data.request_id, option).danger());
    }
    if let Some(option) = allow {
        actions = actions.child(acp_permission_option_button(&data.request_id, option).success());
    }
    if !additional.is_empty() {
        let request_id = data.request_id.clone();
        actions = actions.child(
            Button::new(SharedString::from(format!(
                "acp-permission-more-{}",
                data.request_id
            )))
            .debug_selector(|| "acp-permission-more-options".to_string())
            .with_size(Size::Small)
            .compact()
            .label(t!("AgentUi.more_options").to_string())
            .dropdown_caret(true)
            .dropdown_menu_with_anchor(Anchor::TopRight, move |mut menu, _, _| {
                for option in &additional {
                    menu = menu.item(PopupMenuItem::new(option.name.clone()).action(Box::new(
                        SelectAcpPermissionOption {
                            request_id: request_id.clone(),
                            option_id: option.option_id.clone(),
                        },
                    )));
                }
                menu
            }),
        );
    }

    actions.into_any_element()
}

fn preferred_acp_permission_option<'a>(
    options: &'a [AcpPermissionOptionData],
    preferred_kind: &str,
    kind_prefix: &str,
) -> Option<&'a AcpPermissionOptionData> {
    options
        .iter()
        .find(|option| option.kind == preferred_kind)
        .or_else(|| {
            options
                .iter()
                .find(|option| option.kind.starts_with(kind_prefix))
        })
}

fn acp_permission_option_button(request_id: &str, option: &AcpPermissionOptionData) -> Button {
    let action_request_id = request_id.to_string();
    let option_id = option.option_id.clone();
    Button::new(SharedString::from(format!(
        "acp-permission-option-{request_id}-{option_id}"
    )))
    .debug_selector({
        let kind = option.kind.clone();
        move || format!("acp-permission-{kind}")
    })
    .with_size(Size::Small)
    .compact()
    .label(option.name.clone())
    .on_click(move |_, window, cx| {
        window.dispatch_action(
            Box::new(SelectAcpPermissionOption {
                request_id: action_request_id.clone(),
                option_id: option_id.clone(),
            }),
            cx,
        );
    })
}

// ============================================================================
// 渲染辅助
// ============================================================================

fn tool_card_title(data: &ToolCardData, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    div()
        .flex_1()
        .min_w_0()
        .text_sm()
        .text_color(theme.foreground)
        .truncate()
        .child(tool_card_title_text(data))
        .into_any_element()
}

fn tool_card_title_text(data: &ToolCardData) -> String {
    let mut parts = vec![
        tool_card_prefix(&data.tool_name).to_string(),
        data.tool_name.clone(),
    ];
    if let Some(target) = tool_card_target_label(data) {
        parts.push(format!("@{target}"));
    }
    if !data.input_summary.is_empty() {
        parts.push(data.input_summary.clone());
    }
    parts.join(" · ")
}

fn tool_card_target_label(data: &ToolCardData) -> Option<&str> {
    data.target_label
        .as_deref()
        .filter(|label| !label.is_empty())
        .or_else(|| data.target_id.as_deref().filter(|id| !id.is_empty()))
}

fn confirm_card_header(data: &ToolConfirmCardData) -> String {
    if data.items.len() > 1 {
        return t!("AgentUi.batch_tool_confirmation").to_string();
    }
    if is_terminal_exec_tool(&data.tool_name) {
        t!("AgentUi.terminal_execution_confirmation").to_string()
    } else {
        t!("AgentUi.tool_execution_confirmation").to_string()
    }
}

fn confirm_card_title(data: &ToolConfirmCardData) -> String {
    if data.items.len() > 1 {
        return t!("AgentUi.pending_tools", count = data.items.len()).to_string();
    }
    let prefix = tool_card_prefix(&data.tool_name);
    if data.input_summary.is_empty() || !data.input_json.is_empty() {
        format!("{prefix} · {}", data.tool_name)
    } else {
        format!("{prefix} · {} · {}", data.tool_name, data.input_summary)
    }
}

fn render_confirm_batch_items(data: &ToolConfirmCardData, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .children(data.items.iter().map(|item| {
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .px_2()
                .py_1()
                .rounded_md()
                .bg(theme.background)
                .child(
                    div()
                        .flex_shrink_0()
                        .max_w(px(120.0))
                        .truncate()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(item.tool_name.clone()),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_xs()
                        .text_color(theme.foreground)
                        .child(item.input_summary.clone()),
                )
        }))
        .into_any_element()
}

fn tool_card_prefix(tool_name: &str) -> String {
    if is_terminal_exec_tool(tool_name) {
        t!("AgentUi.terminal_execution").to_string()
    } else {
        t!("AgentUi.tool").to_string()
    }
}

fn is_terminal_exec_tool(tool_name: &str) -> bool {
    matches!(tool_name, "terminal_exec" | "terminal.exec")
}

fn tool_output_json(data: &ToolCardData) -> String {
    let source = if data.data_text.trim().is_empty() {
        data.summary.trim()
    } else {
        data.data_text.trim()
    };
    if source.is_empty() {
        return String::new();
    }
    let formatted = serde_json::from_str::<serde_json::Value>(source)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| {
            serde_json::to_string_pretty(&serde_json::json!({ "output": source }))
                .unwrap_or_default()
        });
    truncate_chars(&formatted, MAX_TOOL_OUTPUT_JSON_CHARS)
}

fn distinct_tool_output_json(data: &ToolCardData) -> String {
    let output = tool_output_json(data);
    if output.is_empty() || tool_output_duplicates_input(data, &output) {
        String::new()
    } else {
        output
    }
}

fn tool_output_duplicates_input(data: &ToolCardData, output_json: &str) -> bool {
    json_values_equal(&data.input_json, output_json)
        || wrapped_output_matches_input_summary(&data.input_summary, output_json)
}

fn json_values_equal(left: &str, right: &str) -> bool {
    let Ok(left) = serde_json::from_str::<serde_json::Value>(left) else {
        return false;
    };
    let Ok(right) = serde_json::from_str::<serde_json::Value>(right) else {
        return false;
    };
    left == right
}

fn wrapped_output_matches_input_summary(input_summary: &str, output_json: &str) -> bool {
    let input_summary = input_summary.trim();
    if input_summary.is_empty() {
        return false;
    }
    serde_json::from_str::<serde_json::Value>(output_json)
        .ok()
        .and_then(|value| {
            value
                .as_object()
                .and_then(|object| object.get("output"))
                .and_then(|output| output.as_str())
                .map(|output| output.trim() == input_summary)
        })
        .unwrap_or(false)
}

fn tool_card_json_block(
    label: impl Into<SharedString>,
    id: SharedString,
    content: String,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let height = tool_json_height(&content);
    let input = tool_json_input(id.clone(), content, window, cx);
    v_flex()
        .debug_selector(|| "agent-tool-json-block".to_string())
        .w_full()
        .min_w_0()
        .self_stretch()
        .items_stretch()
        .gap_1()
        .px_1()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label.into()),
        )
        .child(
            h_flex()
                .debug_selector(|| "agent-tool-json-frame".to_string())
                .w_full()
                .min_w_0()
                .self_stretch()
                .items_stretch()
                .h(height)
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.code_background)
                .overflow_hidden()
                .child(
                    div()
                        .debug_selector(|| "agent-tool-json-input-slot".to_string())
                        .flex_1()
                        .w_full()
                        .min_w_0()
                        .self_stretch()
                        .h_full()
                        .child(
                            Input::new(&input)
                                .bare()
                                .flex_1()
                                .w_full()
                                .min_w_0()
                                .h_full()
                                .appearance(false)
                                .disabled(true)
                                .text_xs()
                                .text_color(theme.code_foreground),
                        ),
                ),
        )
        .into_any_element()
}

fn tool_card_text_block(
    label: impl Into<SharedString>,
    id: SharedString,
    content: String,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let height = tool_json_height(&content);
    let input = tool_text_input(id.clone(), content, window, cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .px_1()
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label.into()),
        )
        .child(
            div()
                .w_full()
                .min_w_0()
                .h(height)
                .rounded(cx.theme().radius)
                .border_1()
                .border_color(theme.border)
                .bg(theme.code_background)
                .overflow_hidden()
                .child(
                    Input::new(&input)
                        .bare()
                        .w_full()
                        .min_w_0()
                        .h_full()
                        .appearance(false)
                        .disabled(true)
                        .text_xs()
                        .text_color(theme.code_foreground),
                ),
        )
        .into_any_element()
}

fn tool_json_height(content: &str) -> gpui::Pixels {
    let rows = content
        .lines()
        .count()
        .clamp(TOOL_JSON_MIN_ROWS, TOOL_JSON_MAX_ROWS);
    px(rows as f32 * TOOL_JSON_LINE_HEIGHT_PX + TOOL_JSON_VERTICAL_PADDING_PX)
}

fn tool_text_input(
    id: SharedString,
    content: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let state = window.use_keyed_state(
        SharedString::from(format!("{}-text-input", id)),
        cx,
        |window, cx| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("text")
                    .line_number(false)
                    .rows(TOOL_JSON_MAX_ROWS)
                    .soft_wrap(false)
                    .default_value(content.clone())
            });
            ToolJsonInputState {
                input,
                value: content.clone(),
            }
        },
    );
    state.update(cx, |data, cx| {
        if data.value != content {
            data.value = content.clone();
            data.input.update(cx, |input, cx| {
                input.set_value(content, window, cx);
            });
        }
        data.input.clone()
    })
}

fn tool_json_input(
    id: SharedString,
    content: String,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    let state = window.use_keyed_state(
        SharedString::from(format!("{}-json-input", id)),
        cx,
        |window, cx| {
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .code_editor("json")
                    .line_number(false)
                    .rows(TOOL_JSON_MAX_ROWS)
                    .soft_wrap(false)
                    .default_value(content.clone())
            });
            ToolJsonInputState {
                input,
                value: content.clone(),
            }
        },
    );
    state.update(cx, |data, cx| {
        if data.value != content {
            data.value = content.clone();
            data.input.update(cx, |input, cx| {
                input.set_value(content, window, cx);
            });
        }
        data.input.clone()
    })
}

fn fallback(content: &str, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    div()
        .w_full()
        .min_w_0()
        .p_2()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(
            themed_markdown(
                SharedString::from("agent-card-fallback"),
                t!("AgentUi.unparseable_agent_card", content = content).to_string(),
                &theme,
            )
            .text_xs()
            .selectable(true),
        )
        .into_any_element()
}

fn tool_status_label(data: &ToolCardData) -> String {
    if data.running {
        t!("AgentUi.running").to_string()
    } else if data.success == Some(false) {
        t!("AgentUi.failed").to_string()
    } else {
        t!("AgentUi.completed_state").to_string()
    }
}

fn confirm_status_label(status: &str) -> String {
    match status {
        "approved" => t!("AgentUi.approved").to_string(),
        "rejected" => t!("AgentUi.rejected").to_string(),
        _ => t!("AgentUi.pending_confirmation").to_string(),
    }
}

fn confirm_status_color(status: &str, cx: &App) -> gpui::Hsla {
    match status {
        "approved" => cx.theme().success,
        "rejected" => cx.theme().danger,
        _ => cx.theme().warning,
    }
}

fn acp_permission_status_label(data: &AcpPermissionCardData) -> String {
    match data.status.as_str() {
        "approved" | "rejected" if !data.selected_option_name.is_empty() => t!(
            "AgentUi.selected_option",
            option = data.selected_option_name
        )
        .to_string(),
        "approved" => t!("AgentUi.allowed").to_string(),
        "rejected" => t!("AgentUi.rejected").to_string(),
        "cancelled" => t!("AgentUi.cancelled").to_string(),
        _ => t!("AgentUi.awaiting_approval").to_string(),
    }
}

fn acp_permission_status_color(status: &str, cx: &App) -> gpui::Hsla {
    match status {
        "approved" => cx.theme().success,
        "rejected" | "cancelled" => cx.theme().danger,
        _ => cx.theme().warning,
    }
}

fn render_subagent_card(
    data: &SubAgentCardData,
    message_id: &str,
    is_expanded: bool,
    expanded_ids: Arc<Mutex<HashSet<String>>>,
    cx: &mut App,
) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let has_details = !data.task.is_empty() || !data.summary.is_empty();
    let expanded = has_details && is_expanded;
    let mut card = v_flex()
        .w_full()
        .min_w_0()
        .gap_2()
        .p_2()
        .rounded_lg()
        .border_1()
        .border_color(theme.border)
        .bg(theme.panel)
        .child(subagent_header(
            data,
            message_id,
            has_details,
            expanded_ids,
            cx,
        ));
    if expanded {
        card = card.child(subagent_details(data, cx));
    }
    card.into_any_element()
}

fn subagent_header(
    data: &SubAgentCardData,
    message_id: &str,
    has_details: bool,
    expanded_ids: Arc<Mutex<HashSet<String>>>,
    cx: &mut App,
) -> AnyElement {
    let (status_glyph, status_color) = subagent_status_style(data, cx);
    let message_id = message_id.to_string();
    let theme = active_agent_chat_theme(cx);
    let hover_bg = theme.panel_hover;
    h_flex()
        .id(SharedString::from(format!(
            "agent-subagent-card-toggle-{}",
            data.subagent_id
        )))
        .w_full()
        .min_w_0()
        .gap_2()
        .items_center()
        .px_1()
        .py_1()
        .when(has_details, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| {
                    toggle_expanded(&expanded_ids, &message_id);
                    cx.refresh_windows();
                })
        })
        .child(
            div()
                .flex_shrink_0()
                .text_color(status_color)
                .child(status_glyph),
        )
        .child(subagent_title(data, cx))
        .child(subagent_status(data, cx))
        .into_any_element()
}

fn toggle_expanded(expanded_ids: &Arc<Mutex<HashSet<String>>>, message_id: &str) {
    if let Ok(mut ids) = expanded_ids.lock()
        && !ids.insert(message_id.to_string())
    {
        ids.remove(message_id);
    }
}

fn subagent_title(data: &SubAgentCardData, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    div()
        .flex_1()
        .min_w_0()
        .text_sm()
        .text_color(theme.foreground)
        .truncate()
        .child(t!("AgentUi.subagent_name", name = data.name).to_string())
        .into_any_element()
}

fn subagent_status(data: &SubAgentCardData, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    div()
        .flex_shrink_0()
        .text_xs()
        .text_color(theme.muted_foreground)
        .child(subagent_status_label(data))
        .into_any_element()
}

fn subagent_status_style(data: &SubAgentCardData, cx: &App) -> (&'static str, gpui::Hsla) {
    let theme = active_agent_chat_theme(cx);
    if data.running {
        ("●", theme.muted_foreground)
    } else if data.success == Some(true) {
        ("✓", cx.theme().success)
    } else if data.success == Some(false) {
        ("✗", cx.theme().danger)
    } else {
        ("•", theme.muted_foreground)
    }
}

fn subagent_status_label(data: &SubAgentCardData) -> String {
    if data.running {
        t!("AgentUi.running").to_string()
    } else if data.success == Some(false) {
        t!("AgentUi.failed").to_string()
    } else {
        t!("AgentUi.completed_state").to_string()
    }
}

fn subagent_details(data: &SubAgentCardData, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    v_flex()
        .w_full()
        .min_w_0()
        .gap_1()
        .px_1()
        .when(!data.task.is_empty(), |this| {
            this.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(
                        themed_markdown(
                            SharedString::from(format!("agent-subagent-task-{}", data.subagent_id)),
                            t!("AgentUi.purpose_markdown", value = data.task).to_string(),
                            &theme,
                        )
                        .selectable(true),
                    ),
            )
        })
        .when(!data.summary.is_empty(), |this| {
            this.child(
                div()
                    .w_full()
                    .min_w_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(
                        themed_markdown(
                            SharedString::from(format!(
                                "agent-subagent-summary-{}",
                                data.subagent_id
                            )),
                            data.summary.clone(),
                            &theme,
                        )
                        .text_xs()
                        .selectable(true),
                    ),
            )
        })
        .into_any_element()
}

/// 按字符截断,超出加省略号。
fn truncate_chars(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let mut out: String = text.chars().take(max_chars).collect();
    out.push_str(t!("AgentUi.truncated_suffix").as_ref());
    out
}

/// 注册 Agent 运行时卡片到全局注册表。
pub fn register_agent_cards(cx: &mut App) {
    CardRegistry::register_global(cx, Arc::new(ToolCard::new()));
    CardRegistry::register_global(cx, Arc::new(SubAgentCard::new()));
    CardRegistry::register_global(cx, Arc::new(ToolConfirmCard));
    CardRegistry::register_global(cx, Arc::new(AcpPermissionCard));
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
            target_id: Some("ssh-b".into()),
            target_label: Some("prod-b".into()),
            input_summary: "hi".into(),
            input_json: "{\"text\":\"hi\"}".into(),
            running: false,
            success: Some(true),
            summary: "echo: hi".into(),
            data_text: "hi".into(),
        };
        let back = ToolCardData::from_json(&data.to_json()).expect("parse");
        assert_eq!(back.call_id, "call_1");
        assert_eq!(back.input_summary, "hi");
        assert_eq!(back.input_json, "{\"text\":\"hi\"}");
        assert_eq!(back.success, Some(true));
        assert_eq!(back.target_id.as_deref(), Some("ssh-b"));
        assert_eq!(back.target_label.as_deref(), Some("prod-b"));
    }

    #[test]
    fn tool_card_title_includes_target_label_for_multi_resource_results() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "ssh.exec".into(),
            target_id: Some("ssh-b".into()),
            target_label: Some("prod-b".into()),
            input_summary: "df -h".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "ok".into(),
        };

        assert_eq!(
            format!("{} · ssh.exec · @prod-b · df -h", t!("AgentUi.tool")),
            tool_card_title_text(&data)
        );
    }

    #[test]
    fn subagent_card_data_roundtrips() {
        let data = SubAgentCardData {
            subagent_id: "sub_1".into(),
            name: "reviewer".into(),
            task: "检查 runtime".into(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
        };
        let back = SubAgentCardData::from_json(&data.to_json()).expect("parse");
        assert_eq!(back.subagent_id, "sub_1");
        assert_eq!(back.name, "reviewer");
        assert_eq!(back.success, Some(true));
    }

    #[test]
    fn tool_confirm_card_data_roundtrips() {
        let data = ToolConfirmCardData {
            call_id: "call_1".into(),
            tool_name: "db_schema".into(),
            items: Vec::new(),
            input_summary: "show tables".into(),
            input_json: "{\"sql\":\"show tables\"}".into(),
            question: "确认执行工具 `db_schema` 吗?".into(),
            status: "pending".into(),
        };
        let back = ToolConfirmCardData::from_json(&data.to_json()).expect("parse");

        assert_eq!(back.call_id, "call_1");
        assert_eq!(back.tool_name, "db_schema");
        assert!(back.items.is_empty());
        assert_eq!(back.input_summary, "show tables");
        assert_eq!(back.input_json, "{\"sql\":\"show tables\"}");
        assert_eq!(back.question, "确认执行工具 `db_schema` 吗?");
        assert_eq!(back.status, "pending");
    }

    #[test]
    fn acp_permission_card_data_roundtrips_all_protocol_options() {
        let data = AcpPermissionCardData {
            request_id: "session:call".into(),
            session_id: "session".into(),
            tool_call_id: "call".into(),
            tool_name: "Write file".into(),
            summary: "ACP agent requests permission for Write file".into(),
            details_json: "{\"path\":\"/tmp/a\"}".into(),
            options: vec![
                AcpPermissionOptionData {
                    option_id: "reject".into(),
                    name: "Reject".into(),
                    kind: "reject_once".into(),
                },
                AcpPermissionOptionData {
                    option_id: "allow-once".into(),
                    name: "Allow once".into(),
                    kind: "allow_once".into(),
                },
                AcpPermissionOptionData {
                    option_id: "allow-always".into(),
                    name: "Always allow".into(),
                    kind: "allow_always".into(),
                },
            ],
            status: "pending".into(),
            selected_option_name: String::new(),
        };

        let back = AcpPermissionCardData::from_json(&data.to_json()).expect("parse");
        assert_eq!(ACP_PERMISSION_CARD, "acp.permission");
        assert_eq!("session:call", back.request_id);
        assert_eq!(3, back.options.len());
        assert_eq!("allow_always", back.options[2].kind);
        assert_eq!("pending", back.status);
    }

    #[test]
    fn batch_tool_confirm_card_data_roundtrips_and_titles_as_batch() {
        let data = ToolConfirmCardData {
            call_id: "call_a".into(),
            tool_name: "ssh_exec".into(),
            items: vec![
                ToolConfirmItemData {
                    call_id: "call_a".into(),
                    tool_name: "ssh_exec".into(),
                    input_summary: "rm -rf /tmp/a".into(),
                    input_json: String::new(),
                },
                ToolConfirmItemData {
                    call_id: "call_b".into(),
                    tool_name: "ssh_exec".into(),
                    input_summary: "rm -rf /tmp/b".into(),
                    input_json: String::new(),
                },
            ],
            input_summary: "rm -rf /tmp/a".into(),
            input_json: String::new(),
            question: "确认执行 2 个工具吗?".into(),
            status: "pending".into(),
        };
        let back = ToolConfirmCardData::from_json(&data.to_json()).expect("parse");

        assert_eq!(2, back.items.len());
        assert_eq!("call_b", back.items[1].call_id);
        assert_eq!(
            t!("AgentUi.batch_tool_confirmation"),
            confirm_card_header(&back)
        );
        assert_eq!(
            t!("AgentUi.pending_tools", count = 2),
            confirm_card_title(&back)
        );
    }

    #[test]
    fn confirm_card_title_omits_summary_when_input_details_are_visible() {
        let data = ToolConfirmCardData {
            call_id: "call_1".into(),
            tool_name: "db_schema".into(),
            items: Vec::new(),
            input_summary: "{\"connection\":\"8\",\"database\":\"ai_app3\"}".into(),
            input_json: "{\n  \"connection\": \"8\",\n  \"database\": \"ai_app3\"\n}".into(),
            question: "确认执行工具 `db_schema` 吗?".into(),
            status: "pending".into(),
        };

        assert_eq!(
            format!("{} · db_schema", t!("AgentUi.tool")),
            confirm_card_title(&data)
        );
    }

    #[test]
    fn confirm_card_title_keeps_summary_when_input_details_are_absent() {
        let data = ToolConfirmCardData {
            call_id: "call_1".into(),
            tool_name: "db_schema".into(),
            items: Vec::new(),
            input_summary: "show tables".into(),
            input_json: String::new(),
            question: "确认执行工具 `db_schema` 吗?".into(),
            status: "pending".into(),
        };

        assert_eq!(
            format!("{} · db_schema · show tables", t!("AgentUi.tool")),
            confirm_card_title(&data)
        );
    }

    #[test]
    fn terminal_exec_confirm_card_labels_terminal_execution() {
        let data = ToolConfirmCardData {
            call_id: "call_1".into(),
            tool_name: "terminal_exec".into(),
            items: Vec::new(),
            input_summary: "df -h".into(),
            input_json: String::new(),
            question: "确认执行工具 `terminal_exec` 吗?".into(),
            status: "pending".into(),
        };

        assert_eq!(
            t!("AgentUi.terminal_execution_confirmation"),
            confirm_card_header(&data)
        );
        assert_eq!(
            format!(
                "{} · terminal_exec · df -h",
                t!("AgentUi.terminal_execution")
            ),
            confirm_card_title(&data)
        );
        assert_eq!(
            t!("AgentUi.terminal_execution"),
            tool_card_prefix("terminal.exec")
        );
    }

    #[test]
    fn truncate_adds_marker() {
        let s = "a".repeat(10);
        assert_eq!(truncate_chars(&s, 100), s);
        assert!(truncate_chars(&s, 3).contains(t!("AgentUi.truncated_suffix").as_ref()));
    }

    #[test]
    fn tool_output_json_prefers_data_without_repeating_summary() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "{\"rows\":[1]}".into(),
        };

        let output = tool_output_json(&data);

        assert_eq!("{\n  \"rows\": [\n    1\n  ]\n}", output);
        assert!(!output.contains("summary"));
    }

    #[test]
    fn tool_output_json_wraps_plain_text_as_json() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "plain output".into(),
        };

        assert_eq!(
            "{\n  \"output\": \"plain output\"\n}",
            tool_output_json(&data)
        );
    }

    #[test]
    fn terminal_exec_output_text_extracts_multiline_output() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "terminal_exec".into(),
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: String::new(),
            data_text: serde_json::json!({
                "completion": "observed_output",
                "output": "line 1\nline 2\nline 3"
            })
            .to_string(),
        };

        assert_eq!("line 1\nline 2\nline 3", terminal_exec_output_text(&data));
    }

    #[test]
    fn terminal_exec_output_text_keeps_more_than_generic_json_limit() {
        let output = "a".repeat(MAX_TOOL_OUTPUT_JSON_CHARS + 100);
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "terminal_exec".into(),
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: String::new(),
            data_text: serde_json::json!({ "output": output }).to_string(),
        };

        let rendered = terminal_exec_output_text(&data);

        assert_eq!(MAX_TOOL_OUTPUT_JSON_CHARS + 100, rendered.len());
        assert!(!rendered.contains(t!("AgentUi.truncated_suffix").as_ref()));
    }

    #[test]
    fn tool_json_height_keeps_short_json_readable_and_long_json_bounded() {
        let min_height =
            px(TOOL_JSON_MIN_ROWS as f32 * TOOL_JSON_LINE_HEIGHT_PX
                + TOOL_JSON_VERTICAL_PADDING_PX);
        let max_height =
            px(TOOL_JSON_MAX_ROWS as f32 * TOOL_JSON_LINE_HEIGHT_PX
                + TOOL_JSON_VERTICAL_PADDING_PX);

        assert_eq!(min_height, tool_json_height("{\"sql\":\"select 1\"}"));
        assert_eq!(max_height, tool_json_height(&"{\n".repeat(40)));
    }

    #[test]
    fn distinct_tool_output_json_hides_structurally_equal_input() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: "{\n  \"rows\": [\n    1\n  ]\n}".into(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "{\"rows\":[1]}".into(),
        };

        assert_eq!("", distinct_tool_output_json(&data));
    }

    #[test]
    fn distinct_tool_output_json_hides_plain_output_matching_input_summary() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            target_id: None,
            target_label: None,
            input_summary: "hello".into(),
            input_json: "{\n  \"text\": \"hello\"\n}".into(),
            running: false,
            success: Some(true),
            summary: "hello".into(),
            data_text: "hello".into(),
        };

        assert_eq!("", distinct_tool_output_json(&data));
    }

    #[test]
    fn distinct_tool_output_json_keeps_different_output() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "query".into(),
            target_id: None,
            target_label: None,
            input_summary: "select 1".into(),
            input_json: "{\n  \"sql\": \"select 1\"\n}".into(),
            running: false,
            success: Some(true),
            summary: "1 row".into(),
            data_text: "{\"rows\":[{\"value\":1}]}".into(),
        };

        assert!(distinct_tool_output_json(&data).contains("\"rows\""));
    }
}
