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

use agent_runtime::ToolAction;
use crate::agent_diff::{FileChangeSummary, patch_from_summary};
use crate::card::{CardMessage, CardRegistry, ChatCard};
use crate::theme::{AgentChatTheme, active_agent_chat_theme, themed_markdown};
use gpui::prelude::FluentBuilder;
use gpui::{
    Action, Anchor, AnyElement, App, AppContext, Entity, InteractiveElement, IntoElement,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, Size,
    button::{Button, ButtonVariants},
    diff::{Diff, DiffFile, DiffHunkSeparator, DiffMode, DiffState},
    h_flex,
    input::{Editor, EditorState},
    menu::{DropdownMenu, PopupMenuItem},
    v_flex,
};
use one_assets::IconName;
use rust_i18n::t;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

const MAX_TOOL_OUTPUT_JSON_CHARS: usize = 4000;
const MAX_TERMINAL_TOOL_OUTPUT_CHARS: usize = 64_000;
const TOOL_JSON_MIN_ROWS: usize = 6;
const TOOL_JSON_MAX_ROWS: usize = 14;
const TOOL_JSON_LINE_HEIGHT_PX: f32 = 18.0;
const TOOL_JSON_VERTICAL_PADDING_PX: f32 = 20.0;

struct ToolJsonInputState {
    input: Entity<EditorState>,
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
/// 计划进度卡片的 `kind`（一行进度，展开看步骤）。
pub const PLAN_CARD: &str = "agent.plan";
/// 对话压缩卡片的 `kind`（一行说明，展开看摘要）。
pub const COMPACTION_CARD: &str = "agent.compaction";

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
    /// 调用方声明的动作类别。UI 据此选动词;`Other` 时改显示工具名。
    #[serde(default)]
    pub action: ToolAction,
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
    /// 本次调用改动的文件(已算成可渲染的 diff 行)。空表示没有文件改动。
    #[serde(default)]
    pub file_changes: Vec<FileChangeSummary>,
    /// 执行耗时(毫秒)。为 0 或缺失时不显示。
    #[serde(default)]
    pub duration_ms: Option<i64>,
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

/// 对话压缩卡片数据。
///
/// 历史里被压缩掉的旧上下文：过去这里把整份摘要直接铺在转录里，两屏就没了。
/// 现在只留一行说明，正文交给展开。
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CompactionCardData {
    /// 被压缩进摘要的历史条目数。
    #[serde(default)]
    pub items: usize,
    /// 摘要正文。
    #[serde(default)]
    pub text: String,
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

impl CompactionCardData {
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
    /// Diff 组件的状态缓存:`DiffState` 是带订阅的实体,而卡片每帧重渲染,
    /// 必须跨帧复用。key 含 rows 指纹,内容变了就换新实体(旧实体无其余
    /// 强引用,GC 回收)。
    diff_states: Arc<Mutex<HashMap<String, WeakEntity<DiffState>>>>,
}

impl ToolCard {
    fn new() -> Self {
        Self {
            expanded: Arc::new(Mutex::new(HashSet::new())),
            diff_states: Arc::new(Mutex::new(HashMap::new())),
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

        let (status_glyph, status_color) = tool_status_style(&data, cx);
        let expanded_diff = data.file_changes.iter().any(FileChangeSummary::has_rows);
        let has_details = expanded_diff
            || !data.input_json.is_empty()
            || !data.summary.is_empty()
            || !data.data_text.is_empty();
        let expanded = has_details && self.is_expanded(msg.id);
        let toggle_state = self.expanded.clone();
        let message_id = msg.id.to_string();
        let toggle_id = SharedString::from(format!("agent-tool-card-toggle-{}", data.call_id));
        let hover_bg = theme.panel_hover;
        // 子代理入口要发出去的两个值；先取出来，免得按钮的 `move` 闭包跟后面的
        // `data` 借用打架。
        let subagent_target = subagent_session_of_card(&data);
        let subagent_open_id = subagent_target.clone().unwrap_or_default();
        let subagent_open_title = tool_row_title(&data);

        // 一次工具调用 = 一行。没有容器、没有边框、没有底色:重装饰会让一摞
        // 调用看起来像一摞表单,而它们只是同一件事的连续步骤。hover 才给底色。
        let card = v_flex()
            .debug_selector(|| "agent-tool-card".to_string())
            .w_full()
            .min_w_0()
            .gap_1()
            .child(
                h_flex()
                    .id(toggle_id)
                    .debug_selector(|| "agent-tool-row".to_string())
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
                                toggle_expanded(&toggle_state, &message_id);
                                cx.refresh_windows();
                            })
                    })
                    .child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(status_color)
                            .child(status_glyph),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_xs()
                            .text_color(theme.foreground)
                            .child(tool_row_title(&data)),
                    )
                    .children(tool_row_meta_chips(&data, cx).into_iter().map(
                        |(text, color)| {
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(color)
                                .child(text)
                        },
                    ))
                    // 子代理卡片的详情入口。
                    //
                    // 放在标题行末尾而不是塞进展开区：它要的是「跳出去看另一条会话的
                    // 完整推理」，不是「就地展开这一段 JSON」。点了之后由视图去
                    // `session/load`，由宿主把详情面板摆到右侧。
                    .when(subagent_target.is_some(), |this| {
                        this.child(
                            Button::new(SharedString::from(format!(
                                "agent-tool-subagent-open-{subagent_open_id}"
                            )))
                            .ghost()
                            .xsmall()
                            .label(t!("AgentUi.subagent_open_detail").to_string())
                            .on_click(move |_, window, cx| {
                                window.dispatch_action(
                                    Box::new(OpenSubagentDetail {
                                        acp_session_id: subagent_open_id.clone(),
                                        title: subagent_open_title.clone(),
                                    }),
                                    cx,
                                );
                            }),
                        )
                    })
                    .when(has_details, |this| {
                        this.child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .xsmall()
                            .text_color(theme.muted_foreground)
                            .flex_shrink_0(),
                        )
                    }),
            );

        let mut card = card;

        if expanded {
            // 有 diff 就**不给**入参 / 输出:入参里装的正是这次改动的两份文件内容,
            // 再摊开一遍等于同一件事说两遍,而且更难看。
            if expanded_diff {
                let diff_blocks = build_diff_blocks(&data, &self.diff_states, cx);
                card = card.child(tool_card_diff_block(&diff_blocks, cx));
            } else {
                card = card.child(tool_card_detail_block(&data, window, cx));
            }
        }

        card.into_any_element()
    }
}

/// 这张工具卡片是不是一次**子代理**调用？是就给出它那条子会话的协议 id。
///
/// 判据落在观测文本上（外部 agent 的 `task` 工具会把子会话地址写在输出里，
/// 见 [`crate::acp::subagent_link_from_observation`]）。放在渲染期而不是 reducer 里，是为了不往
/// [`ToolCardData`] 上加一个只有子代理才用得到的字段——那会牵动十几处构造点，
/// 而这张卡片本来就每帧都在解析自己的 JSON（`ToolCardData::from_json`）。
///
/// 解析前先做一次廉价的标记预筛：绝大多数工具卡片的输出里连 `<task id=` 都
/// 不出现，直接跳过 JSON 解析。
pub(crate) fn subagent_session_of_card(data: &ToolCardData) -> Option<String> {
    for text in [&data.summary, &data.data_text] {
        if !text.contains("<task id=") && !text.contains("\"parentSessionId\"") {
            continue;
        }
        if let Some(link) = crate::acp::subagent_link_from_observation(text) {
            return Some(link.session_id);
        }
    }
    None
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

/// 计划进度卡片：过程区里的一行进度，展开看每一步。
///
/// 计划以前只活在输入框上方的触发器里：往回翻历史时看不到当时打算怎么做。
struct PlanCard {
    expanded: Arc<Mutex<HashSet<String>>>,
}

impl PlanCard {
    fn new() -> Self {
        Self {
            expanded: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn is_expanded(&self, message_id: &str) -> bool {
        self.expanded
            .lock()
            .map(|set| set.contains(message_id))
            .unwrap_or(false)
    }
}

impl ChatCard for PlanCard {
    fn kind(&self) -> &'static str {
        PLAN_CARD
    }

    fn render(&self, msg: &CardMessage, _window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let Some(data) = PlanCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };

        let expandable = !data.steps.is_empty();
        let expanded = expandable && self.is_expanded(msg.id);
        let toggle_state = self.expanded.clone();
        let message_id = msg.id.to_string();
        let (done, total) = data.progress();
        let meta = if total == 0 {
            t!("AgentUi.no_plan").to_string()
        } else {
            format!("{done}/{total} {}", plan_progress_state(&data, done, total))
        };

        let mut card = v_flex()
            .debug_selector(|| "agent-plan-card".to_string())
            .w_full()
            .min_w_0()
            .gap_1()
            .child(agent_info_row(
                SharedString::from(format!("agent-plan-row-{}", msg.id)),
                "agent-plan-row",
                IconName::ListChecks,
                t!("AgentUi.plan").to_string(),
                Some(meta),
                expanded,
                expandable,
                &theme,
                Box::new(move |cx| {
                    toggle_expanded(&toggle_state, &message_id);
                    cx.refresh_windows();
                }),
            ));

        if expanded {
            card = card.child(
                v_flex()
                    .debug_selector(|| "agent-plan-steps".to_string())
                    .w_full()
                    .min_w_0()
                    .pl_4()
                    .gap_0p5()
                    .children(
                        data.steps
                            .iter()
                            .map(|step| plan_step_row(step, &theme, cx)),
                    ),
            );
        }

        card.into_any_element()
    }
}

/// 计划一行的右端事实：还有工作时说「进行中 / 待执行」，全部结束时说「已完成」。
fn plan_progress_state(data: &PlanCardData, done: usize, total: usize) -> String {
    if done >= total {
        return t!("AgentUi.completed_state").to_string();
    }
    if data
        .steps
        .iter()
        .any(|step| matches!(step.status.as_str(), "running" | "in_progress"))
    {
        return t!("AgentUi.in_progress").to_string();
    }
    t!("AgentUi.pending").to_string()
}

/// 计划里的一步：状态字形 + 标题，风险步把风险写在后面。
fn plan_step_row(step: &PlanStepData, theme: &AgentChatTheme, cx: &App) -> AnyElement {
    let (glyph, color) = match step.status.as_str() {
        "completed" => ("✓", cx.theme().success),
        "running" | "in_progress" => ("●", cx.theme().info),
        "failed" => ("✗", cx.theme().danger),
        _ => ("•", theme.muted_foreground),
    };
    let mut row = h_flex()
        .debug_selector(|| "agent-plan-step".to_string())
        .w_full()
        .min_w_0()
        .gap_2()
        .items_center()
        .px_1()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(color)
                .child(glyph),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.foreground)
                .child(step.title.clone()),
        );
    if !step.risk.trim().is_empty() {
        row = row.child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(step.risk.clone()),
        );
    }
    row.into_any_element()
}

/// 对话压缩卡片：一行说清压掉了多少历史，摘要正文折在后面。
struct CompactionCard {
    expanded: Arc<Mutex<HashSet<String>>>,
}

impl CompactionCard {
    fn new() -> Self {
        Self {
            expanded: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    fn is_expanded(&self, message_id: &str) -> bool {
        self.expanded
            .lock()
            .map(|set| set.contains(message_id))
            .unwrap_or(false)
    }
}

impl ChatCard for CompactionCard {
    fn kind(&self) -> &'static str {
        COMPACTION_CARD
    }

    fn render(&self, msg: &CardMessage, window: &mut Window, cx: &mut App) -> AnyElement {
        let theme = active_agent_chat_theme(cx);
        let Some(data) = CompactionCardData::from_json(msg.content) else {
            return fallback(msg.content, cx);
        };

        let expandable = !data.text.trim().is_empty();
        let expanded = expandable && self.is_expanded(msg.id);
        let toggle_state = self.expanded.clone();
        let message_id = msg.id.to_string();
        let meta = (data.items > 0)
            .then(|| t!("AgentUi.context_compacted_items", count = data.items).to_string());

        let mut card = v_flex()
            .debug_selector(|| "agent-compaction-card".to_string())
            .w_full()
            .min_w_0()
            .gap_1()
            .child(agent_info_row(
                SharedString::from(format!("agent-compaction-row-{}", msg.id)),
                "agent-compaction-row",
                IconName::Archive,
                t!("AgentUi.context_compacted").to_string(),
                meta,
                expanded,
                expandable,
                &theme,
                Box::new(move |cx| {
                    toggle_expanded(&toggle_state, &message_id);
                    cx.refresh_windows();
                }),
            ));

        if expanded {
            card = card.child(detail_frame(
                SharedString::from(format!("agent-compaction-detail-{}", msg.id)),
                vec![(
                    t!("AgentUi.context_summary").to_string(),
                    ToolDetailPayload::Text(data.text.clone()),
                )],
                window,
                cx,
            ));
        }

        card.into_any_element()
    }
}

/// 非工具行的一行版式：图标 + 标题 + 右端事实 + 折叠箭头。
///
/// 计划 / 压缩这类「结构事件」跟工具行说同一种话，用同一种版式；它们本来就是
/// 同一段过程里的不同环节，各写一套只会看起来像两个 App 拼起来的。
fn agent_info_row(
    element_id: SharedString,
    debug: &'static str,
    icon: IconName,
    title: String,
    meta: Option<String>,
    expanded: bool,
    expandable: bool,
    theme: &AgentChatTheme,
    on_toggle: Box<dyn Fn(&mut App)>,
) -> AnyElement {
    let hover_bg = theme.panel_hover;
    let foreground = theme.foreground;
    let muted = theme.muted_foreground;
    h_flex()
        .id(element_id)
        .debug_selector(move || debug.to_string())
        .w_full()
        .min_w_0()
        .gap_2()
        .items_center()
        .px_1()
        .py_1()
        .rounded_md()
        .when(expandable, |this| {
            this.cursor_pointer()
                .hover(move |this| this.bg(hover_bg))
                .on_click(move |_, _, cx| on_toggle(cx))
        })
        .child(
            Icon::new(icon)
                .mono()
                .xsmall()
                .text_color(muted)
                .flex_shrink_0(),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(foreground)
                .child(title),
        )
        .children(meta.map(|meta| {
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(muted)
                .child(meta)
        }))
        .when(expandable, |this| {
            this.child(
                Icon::new(if expanded {
                    IconName::ChevronDown
                } else {
                    IconName::ChevronRight
                })
                .xsmall()
                .text_color(muted)
                .flex_shrink_0(),
            )
        })
        .into_any_element()
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

/// 折叠行的标题:**动词 + 目标**;命令行则是**工具名 + 命令**。
///
/// 只有调用方声明了动作类别才用动词;没声明就老实显示工具名 —— 从名字反推
/// 「这个工具在读取」属于虚构活动。命令行是唯一的例外:这行的重点是命令本身,
/// 而 `Bash` / `ssh.exec` 是这次调用自己声明的名字,比一个笼统的「执行」有信息量。
pub(crate) fn tool_row_title(data: &ToolCardData) -> String {
    let target = tool_row_target(data);
    if data.action == ToolAction::Execute {
        let head = if data.tool_name.trim().is_empty() {
            tool_action_label(data.action)
        } else {
            data.tool_name.clone()
        };
        return match target {
            Some(target) => format!("{head} {target}"),
            None => head,
        };
    }
    if data.action.has_verb() {
        let verb = tool_action_label(data.action);
        return match target {
            Some(target) => format!("{verb} {target}"),
            None => format!("{verb} {}", data.tool_name),
        };
    }
    match target {
        Some(target) => format!("{} {target}", data.tool_name),
        None => data.tool_name.clone(),
    }
}

/// 折叠行的目标:文件路径优先,其次资源,最后才是入参摘要。
///
/// 两类例外,都是「这行真正想说什么」的问题:
///
/// - 命令行:命令比资源有用(「Bash cargo test」而不是「Bash prod-a」);
/// - 检索行:被检索的关键字 / 模式比资源有用,资源只是一个查找范围。
fn tool_row_target(data: &ToolCardData) -> Option<String> {
    if let Some(change) = data.file_changes.first() {
        return Some(compact_path(&change.path));
    }
    let summary = data.input_summary.trim();
    let label = tool_card_target_label(data)
        .map(str::trim)
        .filter(|label| !label.is_empty())
        .map(str::to_string);
    let summary = (!summary.is_empty()).then(|| single_line_preview(summary));
    let target = match data.action {
        ToolAction::Execute | ToolAction::Search => summary.or(label),
        _ => label.or(summary),
    };
    target.map(|target| compact_row_target(&target))
}

/// 行内目标:绝对路径只留尾部两段,其余原样。
///
/// 只对绝对路径生效 —— `compact_path` 按 `/` 切段,拿命令去切会把
/// `cat /etc/hosts` 变成 `…/etc/hosts`。相对路径也不动:它已经够短,而且
/// `crates/a/b` 截成 `…/a/b` 反而看不出是哪儿。
fn compact_row_target(target: &str) -> String {
    if target.starts_with('/') {
        compact_path(target)
    } else {
        target.to_string()
    }
}

/// 行内展示用的单行摘要:取第一个非空行、折叠空白、超长截断。
///
/// 多行命令(heredoc、分号串)原样塞进一行,会被宽度截成半句话,还可能把行撑成
/// 两行;先取一行再截断,读到的至少是一句完整的话。
fn single_line_preview(text: &str) -> String {
    const MAX_CHARS: usize = 80;
    let first_line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or("");
    let collapsed = first_line.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate_chars(&collapsed, MAX_CHARS)
}

/// 路径在行内只留尾部两段,并加省略号前缀:`…/navop/crates/core/src/lib.rs` → `…/src/lib.rs`。
///
/// 省略号不是装饰:少了它,`src/lib.rs` 看起来就是一个相对路径,读者会以为
/// 「就是这个文件」;有了它才知道前面还有一段。完整路径留给展开后的 diff 文件头。
pub(crate) fn compact_path(path: &str) -> String {
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    let parts: Vec<&str> = trimmed.split('/').filter(|part| !part.is_empty()).collect();
    match parts.len() {
        0 => trimmed.to_string(),
        1 | 2 => parts.join("/"),
        len => format!("…/{}/{}", parts[len - 2], parts[len - 1]),
    }
}

/// 行右端元信息的类别。类别只用来选颜色,文案与颜色分开,便于单测。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum MetaKind {
    Added,
    Removed,
    Duration,
    Reason,
}

/// 行右端的元信息:**失败原因**,否则是**增删统计 + 耗时**。
///
/// 两段都留着:改动行既要知道改了多少、也要知道花了多久;只留一个会让人以为
/// 另一个不存在(而耗时是判断「这次是不是卡住了」的唯一线索)。
fn tool_row_meta_parts(data: &ToolCardData) -> Vec<(MetaKind, String)> {
    if data.success == Some(false) {
        let reason = data.summary.trim();
        if !reason.is_empty() {
            return vec![(MetaKind::Reason, truncate_chars(reason, 48))];
        }
    }
    let mut parts = Vec::new();
    if let Some((added, removed)) = file_change_totals(&data.file_changes) {
        if added > 0 {
            parts.push((MetaKind::Added, format!("+{added}")));
        }
        if removed > 0 {
            parts.push((MetaKind::Removed, format!("−{removed}")));
        }
    }
    if let Some(ms) = data.duration_ms.filter(|ms| *ms > 0) {
        parts.push((MetaKind::Duration, format_duration_ms(ms)));
    }
    parts
}

fn tool_row_meta_chips(data: &ToolCardData, cx: &App) -> Vec<(String, gpui::Hsla)> {
    tool_row_meta_parts(data)
        .into_iter()
        .map(|(kind, text)| (text, meta_kind_color(kind, cx)))
        .collect()
}

fn meta_kind_color(kind: MetaKind, cx: &App) -> gpui::Hsla {
    match kind {
        MetaKind::Added => cx.theme().success,
        MetaKind::Removed | MetaKind::Reason => cx.theme().danger,
        MetaKind::Duration => active_agent_chat_theme(cx).muted_foreground,
    }
}

/// 增删合计的两个 chip(+绿 / −红),块头和行尾共用。
pub(crate) fn diff_stat_chips(added: u32, removed: u32, cx: &App) -> Vec<(String, gpui::Hsla)> {
    let mut chips = Vec::new();
    if added > 0 {
        chips.push((format!("+{added}"), cx.theme().success));
    }
    if removed > 0 {
        chips.push((format!("−{removed}"), cx.theme().danger));
    }
    chips
}

/// 耗时的人类可读形式:不足一秒给毫秒,超过给一位小数的秒。
fn format_duration_ms(ms: i64) -> String {
    if ms < 1000 {
        format!("{ms}ms")
    } else {
        format!("{:.1}s", ms as f64 / 1000.0)
    }
}

/// 动作类别 → 动词短语。
pub(crate) fn tool_action_label(action: ToolAction) -> String {
    match action {
        ToolAction::Read => t!("AgentUi.action_read").to_string(),
        ToolAction::Edit => t!("AgentUi.action_edit").to_string(),
        ToolAction::Delete => t!("AgentUi.action_delete").to_string(),
        ToolAction::Move => t!("AgentUi.action_move").to_string(),
        ToolAction::Search => t!("AgentUi.action_search").to_string(),
        ToolAction::Execute => t!("AgentUi.action_execute").to_string(),
        ToolAction::Think => t!("AgentUi.action_think").to_string(),
        ToolAction::Fetch => t!("AgentUi.action_fetch").to_string(),
        ToolAction::SwitchMode => t!("AgentUi.action_switch_mode").to_string(),
        ToolAction::Other => String::new(),
    }
}

fn tool_status_style(data: &ToolCardData, cx: &App) -> (&'static str, gpui::Hsla) {
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

/// 整个卡片的增删合计(只在有文件改动时给出)。
pub(crate) fn file_change_totals(changes: &[FileChangeSummary]) -> Option<(u32, u32)> {
    if changes.is_empty() {
        return None;
    }
    let added: u32 = changes.iter().map(|change| change.added).sum();
    let removed: u32 = changes.iter().map(|change| change.removed).sum();
    Some((added, removed))
}

/// 文件改动块:每个文件一个头行 + 若干 diff 行。
///
/// 高度上限在 [`crate::agent_diff`] 里就切好了(`rows` 最多十几行),这里不再
/// 二次裁剪 —— 视口高度与改动规模无关,是这个模块的硬约束。
/// Diff 块里一个文件段的状态:头部数据 + 已就绪的 DiffState(无展示行时为
/// `None`,只渲染文件头)+ 未展示行数。
struct DiffBlockState {
    change: FileChangeSummary,
    state: Option<Entity<DiffState>>,
    hidden: u32,
}

/// 为每个值得展示的 `FileChangeSummary` 准备渲染状态,以
/// `(call_id, path, rows 指纹)` 缓存。卡片每帧都从 JSON 重建 `ToolCardData`,
/// 但 `DiffState` 是带订阅的实体、解析 patch 也要花钱——同一份内容跨帧复用;
/// 流式更新中 rows 增长会改变指纹,旧实体没有其余强引用,交给 GC。
///
/// 模式固定 **Unified 单栏**:工具卡嵌在聊天流里,宽度通常只够一栏,并排双栏
/// 会把右栏裁掉。要看并排,走文件头的「在 Review 中打开」——那里才是全宽。
fn build_diff_blocks(
    data: &ToolCardData,
    cache: &Arc<Mutex<HashMap<String, WeakEntity<DiffState>>>>,
    cx: &mut App,
) -> Vec<DiffBlockState> {
    let mut cache = cache.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    data.file_changes
        .iter()
        .filter(|change| !change.path.is_empty() || change.has_rows())
        .map(|change| {
            let state = if change.has_rows() {
                let key = format!(
                    "{}\u{0}{}\u{0}{:016x}",
                    data.call_id,
                    change.path,
                    file_change_fingerprint(change)
                );
                match cache.get(&key).and_then(WeakEntity::upgrade) {
                    Some(state) => Some(state),
                    None => match DiffFile::parse(&patch_from_summary(change)) {
                        Ok(files) => {
                            let files = files
                                .into_iter()
                                .map(|file| match diff_language_name(file.path()) {
                                    Some(language) => file.with_language(language),
                                    None => file,
                                })
                                .collect::<Vec<_>>();
                            let state = cx
                                .new(|cx| DiffState::new(files, cx).with_mode(DiffMode::Unified));
                            cache.insert(key, state.downgrade());
                            Some(state)
                        }
                        // 合成 patch 按契约必可解析;走到这里说明契约破了。
                        Err(error) => {
                            tracing::warn!("synthesized diff patch failed to parse: {error}");
                            None
                        }
                    },
                }
            } else {
                None
            };
            DiffBlockState {
                change: change.clone(),
                state,
                hidden: change.hidden,
            }
        })
        .collect()
}

/// rows 内容指纹:缓存 key 的一部分。流式更新中同一 call_id 的 rows 会增长,
/// 指纹变化使旧缓存失效。
fn file_change_fingerprint(change: &FileChangeSummary) -> u64 {
    use std::hash::{Hash, Hasher};

    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    change.rows.hash(&mut hasher);
    change.added.hash(&mut hasher);
    change.removed.hash(&mut hasher);
    change.created.hash(&mut hasher);
    hasher.finish()
}

/// Diff 组件自带滚动、需要给定高度。单栏(Unified)下每个展示行独占一行,
/// 故按行数估:行高取组件的等宽小字号 20px,再给 hunk 分隔条与内边距留余量。
/// 保底 72、封顶 320(约 16 行),与卡片「只给摘要」的定位一致。
fn diff_block_height(change: &FileChangeSummary) -> f32 {
    (change.rows.len() as f32 * 20.0 + 16.0).clamp(72.0, 320.0)
}

/// 文件扩展名的小写形式,作为 Diff 组件语法高亮的语言名。
///
/// 组件语言表认**小写扩展名**并自带别名映射(`rs` → `rust`、`ts` → `typescript`),
/// 对不上号只是少一层高亮,不影响渲染。与工作区审阅面板里的同名逻辑一致。
fn diff_language_name(path: &str) -> Option<String> {
    let extension = path.rsplit_once('.')?.1;
    if extension.is_empty() || extension.contains('/') || extension.contains('\\') {
        return None;
    }
    Some(extension.to_lowercase())
}

fn tool_card_diff_block(blocks: &[DiffBlockState], cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    v_flex()
        .debug_selector(|| "agent-tool-diff".to_string())
        .w_full()
        .min_w_0()
        .gap_2()
        .px_1()
        .children(blocks.iter().map(|block| {
            v_flex()
                .w_full()
                .min_w_0()
                .rounded_md()
                .bg(theme.code_background)
                .overflow_hidden()
                .child(tool_diff_file_header(&block.change, cx))
                // 单栏(Unified)渲染,行号与增删标记由 DiffState 决定;头部用上面的
                // 自绘文件头,保持与整卡一致的信息密度与「打开」动作。`state` 缺席
                // 只可能是「有路径但无展示行」,此时仅显示文件头。
                .when_some(block.state.as_ref(), |this, state| {
                    this.child(
                        Diff::new(state)
                            .w_full()
                            .h(px(diff_block_height(&block.change)))
                            .header_visible(false)
                            .hunk_separator(DiffHunkSeparator::Simple),
                    )
                })
                .when(block.hidden > 0, |this| {
                    this.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .px_2()
                            .py_1()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(
                                t!("AgentUi.diff_rows_hidden", count = block.hidden)
                                    .to_string(),
                            ),
                    )
                })
                .into_any_element()
        }))
        .into_any_element()
}

/// diff 块的文件头:完整路径 + 新建标记 + `+N −M` + 「打开」。
fn tool_diff_file_header(change: &FileChangeSummary, cx: &App) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let path = change.path.clone();
    let can_open = !path.is_empty();
    h_flex()
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_xs()
                .text_color(theme.foreground)
                .child(if change.path.is_empty() {
                    t!("AgentUi.diff_unknown_file").to_string()
                } else {
                    change.path.clone()
                }),
        )
        .when(change.created, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("AgentUi.diff_new_file").to_string()),
            )
        })
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().success)
                .child(format!("+{}", change.added)),
        )
        .when(change.removed > 0, |this| {
            this.child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().danger)
                    .child(format!("−{}", change.removed)),
            )
        })
        .when(can_open, |this| {
            this.child(
                // 卡片渲染器不能自己开面板:它只负责把动作发出去,由宿主决定
                // 把这个文件摆到哪个面板。和「执行 / 拒绝」走的是同一条路。
                Button::new(SharedString::from(format!("agent-tool-diff-open-{path}")))
                    .ghost()
                    .xsmall()
                    .label(t!("AgentUi.open_in_review").to_string())
                    .on_click(move |_, window, cx| {
                        window
                            .dispatch_action(Box::new(OpenFileInReview { path: path.clone() }), cx);
                    }),
            )
        })
        .into_any_element()
}

/// 后端的「打开这个文件」请求。
///
/// 卡片渲染器不持有宿主 Entity,也不该知道审阅面板在哪;它只把路径发出去,
/// 由 `AgentChatView` 转成视图事件、宿主决定落位。
#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ai_chat_view, no_json)]
pub struct OpenFileInReview {
    pub path: String,
}

/// 卡片上的「查看推理过程」请求。
///
/// 与 [`OpenFileInReview`] 同一条路数：卡片只知道子会话的协议 id 和标题，
/// 实际去 `session/load`、把推理摆到哪个面板，都由 `AgentChatView` 与宿主接手。
#[derive(Clone, Action, PartialEq, Eq, Deserialize)]
#[action(namespace = ai_chat_view, no_json)]
pub struct OpenSubagentDetail {
    /// 子代理子会话的协议 id。
    pub acp_session_id: String,
    /// 卡片标题，作为详情面板的表头。
    pub title: String,
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

/// 展开块里一节载荷的形式。
#[derive(Clone, Debug, PartialEq, Eq)]
enum ToolDetailPayload {
    /// 已格式化的 JSON。
    Json(String),
    /// 原样文本(命令输出、文件内容)。
    Text(String),
}

impl ToolDetailPayload {
    fn text(&self) -> &str {
        match self {
            Self::Json(text) | Self::Text(text) => text,
        }
    }

    fn is_json(&self) -> bool {
        matches!(self, Self::Json(_))
    }
}

/// 展开块里的一节:一个名字 + 一份载荷。
#[derive(Clone, Debug, PartialEq, Eq)]
struct ToolDetailSection {
    label: String,
    payload: ToolDetailPayload,
}

/// 工具输出的原始文本:数据优先,其次摘要。
fn raw_output_text(data: &ToolCardData) -> String {
    if data.data_text.trim().is_empty() {
        data.summary.trim().to_string()
    } else {
        data.data_text.trim().to_string()
    }
}

/// 把一段原始载荷规整成「JSON 或文本」:能解析成 JSON 就格式化,否则原样当文本。
///
/// 纯文本**不再**被包成 `{"output": "..."}`:多一层括号、多一层 `\n` 转义,只为
/// 看上去「统一」,离原文更远 —— 命令输出和文件内容本来就该按原样读。
fn normalize_payload(raw: &str) -> Option<ToolDetailPayload> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    match serde_json::from_str::<serde_json::Value>(raw) {
        Ok(value) => serde_json::to_string_pretty(&value)
            .ok()
            .map(|text| ToolDetailPayload::Json(truncate_chars(&text, MAX_TOOL_OUTPUT_JSON_CHARS))),
        Err(_) => Some(ToolDetailPayload::Text(raw.to_string())),
    }
}

/// 展开块要说的话:**入参**和**输出**,同一句话不说两遍。
///
/// 输出只是把入参、或行里已经显示过的摘要原样念一遍时就不再单独给一节 —— 重复
/// 一遍读者会以为发现了第二件事。
fn tool_detail_sections(data: &ToolCardData) -> Vec<ToolDetailSection> {
    let input = normalize_payload(&data.input_json);
    let output = if is_terminal_exec_tool(&data.tool_name) {
        let text = terminal_exec_output_text(data);
        (!text.trim().is_empty()).then(|| ToolDetailPayload::Text(text))
    } else {
        normalize_payload(&raw_output_text(data))
    };

    let output_repeats_input = match (&input, &output) {
        (Some(left), Some(right)) => left.text().trim() == right.text().trim(),
        _ => false,
    };
    let output_repeats_summary = match &output {
        Some(payload) => {
            let summary = data.input_summary.trim();
            !summary.is_empty() && payload.text().trim() == summary
        }
        None => false,
    };

    let mut sections = Vec::new();
    if let Some(payload) = input.filter(|_| !output_repeats_input) {
        sections.push(ToolDetailSection {
            label: t!("AgentUi.input").to_string(),
            payload,
        });
    }
    if let Some(payload) = output.filter(|_| !output_repeats_summary) {
        sections.push(ToolDetailSection {
            label: t!("AgentUi.output").to_string(),
            payload,
        });
    }
    sections
}

/// 展开块:入参 + 输出。
///
/// 合成**一个**框、节间一条细线、节名缩成框内的小字头:两个框加两行标题看起来
/// 像两张表单,而它们只是同一次调用的两面。
fn tool_card_detail_block(data: &ToolCardData, window: &mut Window, cx: &mut App) -> AnyElement {
    detail_frame(
        SharedString::from(format!("agent-tool-detail-{}", data.call_id)),
        tool_detail_sections(data)
            .into_iter()
            .map(|section| (section.label, section.payload))
            .collect(),
        window,
        cx,
    )
}

/// 展开内容的公共容器：**一个**框，节与节之间一条细线，节名是框内的小字头。
///
/// 入参 / 输出 / 摘要都走这里 —— 展开的东西只有一种长相。
fn detail_frame(
    id: SharedString,
    sections: Vec<(String, ToolDetailPayload)>,
    window: &mut Window,
    cx: &mut App,
) -> AnyElement {
    let theme = active_agent_chat_theme(cx);
    let border = theme.border;
    let muted = theme.muted_foreground;
    let code_foreground = theme.code_foreground;
    v_flex()
        .debug_selector(|| "agent-tool-detail-block".to_string())
        .w_full()
        .min_w_0()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(border)
        .bg(theme.code_background)
        .overflow_hidden()
        .children(
            sections
                .into_iter()
                .enumerate()
                .map(|(index, (label, payload))| {
                    let content = payload.text().to_string();
                    let height = tool_json_height(&content);
                    let section_id = SharedString::from(format!("{id}-{index}"));
                    let input = if payload.is_json() {
                        tool_json_input(section_id, content, window, cx)
                    } else {
                        tool_text_input(section_id, content, window, cx)
                    };
                    v_flex()
                        .w_full()
                        .min_w_0()
                        .when(index > 0, |this| this.border_t_1().border_color(border))
                        .child(
                            div()
                                .w_full()
                                .min_w_0()
                                .px_2()
                                .pt_1()
                                .text_xs()
                                .text_color(muted)
                                .child(label),
                        )
                        .child(
                            div().w_full().min_w_0().h(height).child(
                                Editor::new(&input)
                                    .w_full()
                                    .min_w_0()
                                    .h_full()
                                    .appearance(false)
                                    .bordered(false)
                                    .readonly(true)
                                    .text_xs()
                                    .text_color(code_foreground),
                            ),
                        )
                        .into_any_element()
                }),
        )
        .into_any_element()
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
                            Editor::new(&input)
                                .flex_1()
                                .w_full()
                                .min_w_0()
                                .h_full()
                                .appearance(false)
                                .bordered(false)
                                .readonly(true)
                                .text_xs()
                                .text_color(theme.code_foreground),
                        ),
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
) -> Entity<EditorState> {
    let state = window.use_keyed_state(
        SharedString::from(format!("{}-text-input", id)),
        cx,
        |window, cx| {
            let input = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("text")
                    .line_number(false)
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
) -> Entity<EditorState> {
    let state = window.use_keyed_state(
        SharedString::from(format!("{}-json-input", id)),
        cx,
        |window, cx| {
            let input = cx.new(|cx| {
                EditorState::new(window, cx)
                    .language("json")
                    .line_number(false)
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
    CardRegistry::register_global(cx, Arc::new(PlanCard::new()));
    CardRegistry::register_global(cx, Arc::new(CompactionCard::new()));
    CardRegistry::register_global(cx, Arc::new(ToolConfirmCard));
    CardRegistry::register_global(cx, Arc::new(AcpPermissionCard));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 一张最小可用的工具卡数据:其余用例只改自己关心的字段。
    fn echo_card() -> ToolCardData {
        ToolCardData {
            call_id: "call_1".into(),
            tool_name: "echo".into(),
            action: ToolAction::Other,
            target_id: None,
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: String::new(),
            file_changes: Vec::new(),
            duration_ms: None,
        }
    }

    /// 子代理卡片要能认出自己的子会话地址——这是「查看推理过程」入口的唯一判据。
    ///
    /// 判据是在**渲染期**从观测文本现解析的（见 [`subagent_session_of_card`]），
    /// 所以这里锁住正反两面：真实终态输出解得出来，普通工具卡片解不出来。
    #[test]
    fn a_subagent_card_exposes_its_child_session_id() {
        let mut data = echo_card();
        // OpenCode `task` 工具的终态输出原样：地址既在 `metadata` 上，也在
        // `output` 正文的 `<task id="…">` 里。
        data.data_text = serde_json::json!({
            "output": "<task id=\"ses_child\" state=\"completed\">done</task>",
            "metadata": {
                "parentSessionId": "ses_parent",
                "sessionId": "ses_child",
            },
        })
        .to_string();

        assert_eq!(
            Some("ses_child".to_string()),
            subagent_session_of_card(&data),
            "认不出子会话地址，卡片上就不会出现「查看推理过程」"
        );
    }

    /// 只有 `output` 正文、metadata 缺席（取消 / 失败）时也要认得出来。
    #[test]
    fn a_cancelled_subagent_card_still_exposes_its_child_session_id() {
        let mut data = echo_card();
        data.data_text = r#"{"error":"Task cancelled: <task id=\"ses_cancelled\" state=\"error\">"}"#.into();

        assert_eq!(
            Some("ses_cancelled".to_string()),
            subagent_session_of_card(&data)
        );
    }

    /// 普通工具卡片不能凭空长出「查看推理过程」入口：点开只会是一片空白。
    #[test]
    fn an_ordinary_tool_card_has_no_subagent_entry() {
        let mut data = echo_card();
        data.data_text = r#"{"stdout":"hello","exit_code":0}"#.into();
        data.summary = "echo: hello".into();

        assert_eq!(None, subagent_session_of_card(&data));

        // 只有 `sessionId`、没有 `parentSessionId` 的元数据不算子代理（太常见了）。
        data.data_text = r#"{"metadata":{"sessionId":"ses_whatever"},"stdout":"hi"}"#.into();
        assert_eq!(None, subagent_session_of_card(&data));
    }

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
            action: ToolAction::Other,
            target_id: Some("ssh-b".into()),
            target_label: Some("prod-b".into()),
            input_summary: "hi".into(),
            input_json: "{\"text\":\"hi\"}".into(),
            running: false,
            success: Some(true),
            summary: "echo: hi".into(),
            data_text: "hi".into(),
            file_changes: Vec::new(),
            duration_ms: None,
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
    fn tool_card_data_without_the_new_fields_still_parses() {
        // 旧会话里的卡片 JSON 没有 action / file_changes / duration_ms,
        // 读回来必须能降级,而不是整条消息渲染成「无法解析」。
        let legacy = serde_json::json!({
            "call_id": "call_1",
            "tool_name": "read_file",
            "input_summary": "/etc/hosts",
            "input_json": "",
            "running": false,
            "success": true,
            "summary": "ok",
            "data_text": "ok",
        })
        .to_string();
        let back = ToolCardData::from_json(&legacy).expect("parse");
        assert_eq!(ToolAction::Other, back.action);
        assert!(back.file_changes.is_empty());
        assert_eq!(None, back.duration_ms);
    }

    #[test]
    fn executed_command_outranks_the_resource_in_the_row_title() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "ssh.exec".into(),
            action: ToolAction::Execute,
            target_id: Some("ssh-b".into()),
            target_label: Some("prod-b".into()),
            input_summary: "df -h".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "ok".into(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        // 命令是这一行的重点,前面挂工具名(这次调用自己声明的名字),资源不重复。
        assert_eq!("ssh.exec df -h", tool_row_title(&data));
    }

    #[test]
    fn multi_line_commands_are_reduced_to_one_readable_line() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "Bash".into(),
            action: ToolAction::Execute,
            target_id: None,
            target_label: None,
            input_summary: "\n\n  cd /repo   &&\n  cargo test --lib\n ".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "ok".into(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        // 取第一个非空行、折叠空白:行不会被 heredoc / 换行撑成两行。
        assert_eq!("Bash cd /repo &&", tool_row_title(&data));
    }

    #[test]
    fn search_rows_show_the_query_rather_than_the_resource() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "grep".into(),
            action: ToolAction::Search,
            target_id: Some("/repo".into()),
            target_label: Some("repo".into()),
            input_summary: "tool_row_meta".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "ok".into(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        assert_eq!(
            format!("{} tool_row_meta", t!("AgentUi.action_search")),
            tool_row_title(&data)
        );
    }

    #[test]
    fn rows_without_a_declared_action_show_the_tool_name_instead_of_a_verb() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "mcp.fs.grep".into(),
            action: ToolAction::Other,
            target_id: None,
            target_label: None,
            input_summary: "TODO".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: "ok".into(),
            data_text: "ok".into(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        assert_eq!("mcp.fs.grep TODO", tool_row_title(&data));
    }

    #[test]
    fn file_changes_drive_the_row_title_and_the_diff_stats() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "Edit".into(),
            action: ToolAction::Edit,
            target_id: Some("/repo/crates/core/src/lib.rs".into()),
            target_label: None,
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: String::new(),
            data_text: String::new(),
            file_changes: vec![FileChangeSummary::from_texts(
                "/repo/crates/core/src/lib.rs",
                Some("a\nb\n"),
                "a\nc\n",
            )],
            duration_ms: Some(120),
        };

        assert_eq!(
            format!("{} …/src/lib.rs", t!("AgentUi.action_edit")),
            tool_row_title(&data)
        );
        assert_eq!(Some((1, 1)), file_change_totals(&data.file_changes));
        // 增删统计与耗时**并列**都留着:只留一个会让人以为另一个不存在。
        assert_eq!(
            vec![
                (MetaKind::Added, "+1".to_string()),
                (MetaKind::Removed, "−1".to_string()),
                (MetaKind::Duration, "120ms".to_string()),
            ],
            tool_row_meta_parts(&data)
        );
    }

    #[test]
    fn a_failed_row_reports_the_reason_instead_of_the_stats() {
        let mut data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "read_file".into(),
            action: ToolAction::Read,
            target_id: None,
            target_label: None,
            input_summary: "a.rs".into(),
            input_json: String::new(),
            running: false,
            success: Some(false),
            summary: "没有这个文件".into(),
            data_text: String::new(),
            file_changes: Vec::new(),
            duration_ms: Some(30),
        };

        let parts = tool_row_meta_parts(&data);
        assert_eq!(1, parts.len());
        assert_eq!(MetaKind::Reason, parts[0].0);
        assert!(parts[0].1.contains("没有这个文件"));

        // 没给失败理由时退回耗时,而不是留一个空位。
        data.summary = String::new();
        assert_eq!(
            vec![(MetaKind::Duration, "30ms".to_string())],
            tool_row_meta_parts(&data)
        );
    }

    #[test]
    fn compact_path_keeps_the_tail_of_a_path() {
        // 截断过的路径带省略号前缀:读者才知道前面还有一段。
        assert_eq!("…/src/lib.rs", compact_path("/repo/crates/core/src/lib.rs"));
        assert_eq!("lib.rs", compact_path("lib.rs"));
        assert_eq!("src/lib.rs", compact_path("src/lib.rs"));
        assert_eq!("", compact_path("   "));
    }

    #[test]
    fn a_read_row_shows_the_tail_of_the_file_it_read() {
        // 外部 agent 给的是绝对路径;行内只留尾部两段,不然一行全是路径。
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "read".into(),
            action: ToolAction::Read,
            target_id: Some("/repo/crates/agent_runtime/src/tools/action.rs".into()),
            target_label: Some("/repo/crates/agent_runtime/src/tools/action.rs".into()),
            input_summary: String::new(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: String::new(),
            data_text: String::new(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        assert_eq!(
            format!("{} …/tools/action.rs", t!("AgentUi.action_read")),
            tool_row_title(&data)
        );
    }

    #[test]
    fn a_command_row_is_never_path_truncated() {
        // 命令里也会出现 `/`,按路径切段会把它切坏(`cat /etc/hosts` → `…/etc/hosts`)。
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "bash".into(),
            action: ToolAction::Execute,
            target_id: None,
            target_label: None,
            input_summary: "cat /etc/hosts".into(),
            input_json: String::new(),
            running: false,
            success: Some(true),
            summary: String::new(),
            data_text: String::new(),
            file_changes: Vec::new(),
            duration_ms: None,
        };

        assert_eq!("bash cat /etc/hosts", tool_row_title(&data));
    }

    #[test]
    fn durations_are_formatted_for_humans() {
        assert_eq!("120ms", format_duration_ms(120));
        assert_eq!("1.5s", format_duration_ms(1500));
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
    fn json_output_is_pretty_printed() {
        let mut data = echo_card();
        data.data_text = "{\"rows\":[1]}".into();

        let sections = tool_detail_sections(&data);

        assert_eq!(1, sections.len());
        assert_eq!(t!("AgentUi.output").to_string(), sections[0].label);
        assert_eq!(
            ToolDetailPayload::Json("{\n  \"rows\": [\n    1\n  ]\n}".to_string()),
            sections[0].payload
        );
    }

    #[test]
    fn plain_text_output_stays_text() {
        let mut data = echo_card();
        data.data_text = "plain output".into();

        let sections = tool_detail_sections(&data);

        // 不再被包成 `{"output": "..."}`:命令输出 / 文件内容按原样读。
        assert_eq!(
            vec![ToolDetailSection {
                label: t!("AgentUi.output").to_string(),
                payload: ToolDetailPayload::Text("plain output".to_string()),
            }],
            sections
        );
    }

    #[test]
    fn input_and_output_are_separate_sections_with_labels() {
        let mut data = echo_card();
        data.input_json = "{\"sql\": \"select 1\"}".into();
        data.data_text = "{\"rows\":[{\"value\":1}]}".into();

        let sections = tool_detail_sections(&data);

        assert_eq!(2, sections.len());
        assert_eq!(t!("AgentUi.input").to_string(), sections[0].label);
        assert_eq!(t!("AgentUi.output").to_string(), sections[1].label);
        assert!(sections[0].payload.text().contains("select 1"));
        assert!(sections[1].payload.text().contains("rows"));
    }

    #[test]
    fn terminal_exec_output_text_extracts_multiline_output() {
        let data = ToolCardData {
            call_id: "call_1".into(),
            tool_name: "terminal_exec".into(),
            target_id: None,
            target_label: None,
            action: ToolAction::Other,
            file_changes: Vec::new(),
            duration_ms: None,
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
            action: ToolAction::Other,
            file_changes: Vec::new(),
            duration_ms: None,
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
    fn output_repeating_the_input_is_shown_once() {
        let mut data = echo_card();
        data.input_json = "{\n  \"rows\": [\n    1\n  ]\n}".into();
        data.data_text = "{\"rows\":[1]}".into();

        let sections = tool_detail_sections(&data);

        assert_eq!(1, sections.len());
        assert_eq!(t!("AgentUi.output").to_string(), sections[0].label);
    }

    #[test]
    fn output_repeating_the_row_summary_is_dropped() {
        let mut data = echo_card();
        data.input_summary = "hello".into();
        data.input_json = "{\n  \"text\": \"hello\"\n}".into();
        data.summary = "hello".into();
        data.data_text = "hello".into();

        let sections = tool_detail_sections(&data);

        assert_eq!(1, sections.len());
        assert_eq!(t!("AgentUi.input").to_string(), sections[0].label);
    }
}
