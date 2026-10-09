//! 轮次投影：把扁平的 [`ChatMessageUI`] 列表切分成「轮次」阅读单元。
//!
//! 为什么要这一层：转录本身是一条扁平的追加流，用户 → 过程 → 结论没有边界，
//! 于是「折叠过程」「轮次页脚」「按轮次定位」都无从下手。这里用**纯函数**把
//! 扁平流投影成轮次，不依赖 GPUI，便于用普通单元测试把语义锁死。
//!
//! 三条硬纪律（都是踩过坑才有的）：
//!
//! 1. **风险不参与过程折叠，但留在时间线原位。** 待审批卡片、被拒/失败的工具与
//!    子代理同时记入 [`TurnProjection::process`]（保持原始顺序）与
//!    [`TurnProjection::risks`]（`process` 的有序子集）：过程区收起时**仍要**显示
//!    它们，但**不能**把它们挪到轮次末尾 —— 那会让失败的工具卡片一直钉在底部。
//! 2. **轮次身份用运行时 `turn_id`，缺失时退化为首个消息 id。** 不用数组下标、
//!    不用显示文本 —— 否则预算裁剪后身份会整体错位。
//! 3. **未测到的时间不猜。** `started_at` / `finished_at` 缺失时渲染层不显示时长，
//!    不按「大概是 0」处理。

use std::collections::HashMap;

use crate::agent_cards::{
    ACP_PERMISSION_CARD, COMPACTION_CARD, SUBAGENT_CARD, SubAgentCardData, TOOL_CARD,
    TOOL_CONFIRM_CARD, ToolCardData, ToolConfirmCardData,
};
use crate::agent_diff::FileChangeSummary;
use crate::{ChatMessageUI, ChatRole, MessageVariant};

/// 轮次的收尾结果。只由真实运行时事件落值，**不得**从消息文本推断。
///
/// 旧实现把「任意系统提示」当作失败，于是 ACP 控制面提示（「正在创建会话」、
/// 「上下文边界」）会让页脚误报红色「失败」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnOutcome {
    Completed,
    Cancelled,
    Failed,
}

/// 轮次的时间信息（Unix 秒）。任一侧缺失即视为未知。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TurnTiming {
    pub started_at: Option<i64>,
    pub finished_at: Option<i64>,
}

impl TurnTiming {
    /// 只有两端都有值时才给时长；否则 `None`（不猜测）。
    pub fn duration_secs(&self) -> Option<i64> {
        match (self.started_at, self.finished_at) {
            (Some(start), Some(end)) => Some((end - start).max(0)),
            _ => None,
        }
    }
}

/// 按轮次 id 记录的时间表。
#[derive(Clone, Debug, Default)]
pub struct TurnTimings {
    by_turn: HashMap<String, TurnTiming>,
    outcomes: HashMap<String, TurnOutcome>,
}

impl TurnTimings {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, turn_id: &str) -> Option<TurnTiming> {
        self.by_turn.get(turn_id).copied()
    }

    /// 该轮真实收尾结果；没有收到终态事件时为 `None`。
    pub fn outcome(&self, turn_id: &str) -> Option<TurnOutcome> {
        self.outcomes.get(turn_id).copied()
    }

    /// 记录轮次收尾结果。终态事件每轮只到达一次，重复时后到的覆盖先前值。
    pub fn note_outcome(&mut self, turn_id: impl Into<String>, outcome: TurnOutcome) {
        self.outcomes.insert(turn_id.into(), outcome);
    }

    /// 记录轮次开始；重复调用只保留最早一次（重放事件不应把起点往后推）。
    pub fn note_started(&mut self, turn_id: impl Into<String>, at: i64) {
        let entry = self.by_turn.entry(turn_id.into()).or_default();
        if entry.started_at.is_none() {
            entry.started_at = Some(at);
        }
    }

    /// 记录轮次结束；重复调用只保留最早一次。
    pub fn note_finished(&mut self, turn_id: impl Into<String>, at: i64) {
        let entry = self.by_turn.entry(turn_id.into()).or_default();
        if entry.finished_at.is_none() {
            entry.finished_at = Some(at);
        }
    }

    pub fn clear(&mut self) {
        self.by_turn.clear();
        self.outcomes.clear();
    }

    pub fn len(&self) -> usize {
        self.by_turn.len()
    }

    pub fn is_empty(&self) -> bool {
        self.by_turn.is_empty()
    }
}

/// 一个轮次的阅读单元。
#[derive(Clone, Debug)]
pub struct TurnProjection<'a> {
    /// 稳定身份；用作渲染元素 id 与展开态覆盖表的 key。
    pub key: String,
    /// 运行时轮次 id；历史恢复 / 本地合成时为 `None`。
    pub turn_id: Option<String>,
    pub timing: TurnTiming,
    /// 真实收尾结果（来自终态事件）；未收到终态事件时为 `None`。
    pub outcome: Option<TurnOutcome>,
    /// 本轮的用户提问（本轮第一条 user 消息）。preamble 轮为 `None`。
    pub head: Option<&'a ChatMessageUI>,
    /// 过程：状态、工具、计划、子代理，以及结论之前的助手片段 ——
    /// **按消息原始顺序，且包含风险项**。
    pub process: Vec<&'a ChatMessageUI>,
    /// 结论：本轮最后一次助手正文。
    pub answer: Vec<&'a ChatMessageUI>,
    /// 风险：待审批 / 失败 / 被拒。**是 `process` 的有序子集**，任何折叠都不得隐藏；
    /// 但**不能**单独排到轮次末尾 —— 那会让失败的工具卡片脱离时间线、一直钉在底部。
    pub risks: Vec<&'a ChatMessageUI>,
    /// 是否仍在进行。
    pub live: bool,
}

impl TurnProjection<'_> {
    /// 有过程内容才谈得上折叠。
    pub fn has_process(&self) -> bool {
        !self.process.is_empty()
    }

    /// 过程区的默认展开态。
    ///
    /// - 仍在进行的轮次：展开（用户要看到实时活动）。
    /// - 已结束的轮次：收起（结论才是重点）。
    ///
    /// 注意风险**不在**过程区，因此不受这里影响。
    pub fn default_process_expanded(&self) -> bool {
        self.live
    }

    /// 这一轮里是否已经有一条**还在跑**的状态消息（例如「OpenCode 正在继续响应…」）。
    ///
    /// 那条消息本身就是「本轮仍在进行」的指示，且比页脚的「进行中」具体 ——
    /// 两者紧挨着渲染就是同一句话说两遍。渲染层据此省略页脚的「进行中」。
    pub fn has_live_status(&self) -> bool {
        self.process
            .iter()
            .chain(self.answer.iter())
            .any(|message| {
                matches!(
                    &message.variant,
                    MessageVariant::Status { is_done: false, .. }
                )
            })
    }

    /// 该轮是否以失败收尾（由真实终态事件判定，不看消息文本）。
    pub fn has_failure(&self) -> bool {
        self.outcome == Some(TurnOutcome::Failed)
    }

    /// 该轮是否以取消收尾。
    pub fn is_cancelled(&self) -> bool {
        self.outcome == Some(TurnOutcome::Cancelled)
    }

    /// 过程步数 = 工具卡片数。
    pub fn step_count(&self) -> usize {
        self.process
            .iter()
            .filter(|message| message.variant.card_kind() == Some(TOOL_CARD))
            .count()
    }

    /// 工具名 → 次数，按次数降序（同次数按名称升序，保证渲染稳定）。
    pub fn tool_breakdown(&self) -> Vec<(String, usize)> {
        let mut counts: HashMap<String, usize> = HashMap::new();
        for message in &self.process {
            if message.variant.card_kind() != Some(TOOL_CARD) {
                continue;
            }
            let Some(data) = ToolCardData::from_json(&message.content) else {
                continue;
            };
            *counts.entry(data.tool_name).or_default() += 1;
        }
        let mut pairs: Vec<(String, usize)> = counts.into_iter().collect();
        pairs.sort_by(|left, right| right.1.cmp(&left.1).then_with(|| left.0.cmp(&right.0)));
        pairs
    }

    /// 是否含待用户决策的卡片（待审批 / 待补充输入）。
    pub fn has_pending_decision(&self) -> bool {
        self.risks
            .iter()
            .any(|message| is_pending_decision(message))
    }

    /// 本轮改动过的文件，按路径去重（同一文件改多次只留最后一次的改动）。
    ///
    /// 去重而不是累加：同一文件被改三遍，用户想知道的是「这个文件变成了什么样」，
    /// 不是三次 diff 的增删之和——累加出来的 `+42 −39` 只会让人以为动了 42 行。
    /// 顺序按首次出现，保证渲染稳定。
    pub fn changed_files(&self) -> Vec<FileChangeSummary> {
        let mut paths: Vec<String> = Vec::new();
        let mut latest: HashMap<String, FileChangeSummary> = HashMap::new();
        for message in &self.process {
            if message.variant.card_kind() != Some(TOOL_CARD) {
                continue;
            }
            let Some(data) = ToolCardData::from_json(&message.content) else {
                continue;
            };
            for change in data.file_changes {
                if change.path.trim().is_empty() {
                    continue;
                }
                if !latest.contains_key(&change.path) {
                    paths.push(change.path.clone());
                }
                latest.insert(change.path.clone(), change);
            }
        }
        paths
            .into_iter()
            .filter_map(|path| latest.remove(&path))
            .collect()
    }
}

/// 工具分布的紧凑文本，例如 `fs.read ×2 · fs.grep ×1`。
///
/// 超出 `max` 项时以 `…` 收尾，避免长轮次把折叠头撑成一行工具名。
/// **不做语义归类**（不把 `fs.read` 猜成「读取」）：工具名是扩展提供的，
/// 猜错会变成虚构活动。
pub fn breakdown_text(breakdown: &[(String, usize)], max: usize) -> Option<String> {
    if breakdown.is_empty() || max == 0 {
        return None;
    }
    let mut parts: Vec<String> = breakdown
        .iter()
        .take(max)
        .map(|(name, count)| format!("{name} ×{count}"))
        .collect();
    if breakdown.len() > max {
        parts.push("…".to_string());
    }
    Some(parts.join(" · "))
}

/// 消息是否表示「仍在进行」。
///
/// 只看 `is_streaming` 不够：一次工具执行、一个子代理、一条未完成的轻量状态
/// 都可能没有流式文本，但用户显然希望它们保持可见。漏判会让运行中的过程区
/// 在默认收起状态下一闪而过。
///
/// 也用于判断「列表末尾还要不要再挂一条『执行中…』」——轮次页脚已经在显示
/// 「进行中」时，再挂一条就是同义重复。
pub(crate) fn is_live_message(message: &ChatMessageUI) -> bool {
    if message.is_streaming {
        return true;
    }
    if let MessageVariant::Status { is_done, .. } = &message.variant {
        return !is_done;
    }
    match message.variant.card_kind() {
        Some(kind) if kind == TOOL_CARD => {
            ToolCardData::from_json(&message.content).is_some_and(|data| data.running)
        }
        Some(kind) if kind == SUBAGENT_CARD => {
            SubAgentCardData::from_json(&message.content).is_some_and(|data| data.success.is_none())
        }
        _ => false,
    }
}

/// 消息在过程区**收起**时是否仍要显示。
///
/// 三类：风险项（失败 / 待审批，见 [`is_risk_message`]）、仍在进行的项
/// （未完成的工具、子代理、状态行）与结构标记（对话压缩边界）。前者藏了会
/// 漏掉失败，中者藏了会让人以为卡住了，后者藏了会让人看不懂上下文为什么变短
/// —— 都不许被折叠吞掉。位置仍由 `process` 决定，不做重排。
pub fn survives_collapse(message: &ChatMessageUI) -> bool {
    is_risk_message(message) || is_live_message(message) || is_structural_marker(message)
}

/// 消息是否是过程里的**结构标记**：压缩边界之类。
///
/// 它既不是风险、也不是进行中的步骤，但它是解释性的：这段历史为什么变短了。
/// 它自己只有一行，收起来省的版面不值这个代价。
fn is_structural_marker(message: &ChatMessageUI) -> bool {
    message.variant.card_kind() == Some(COMPACTION_CARD)
}

/// 消息是否属于「风险」：必须无条件展示，不能被过程折叠吞掉。
pub fn is_risk_message(message: &ChatMessageUI) -> bool {
    if matches!(message.role, ChatRole::System) {
        // 失败 / 取消 / 预算裁断这类系统提示只在特定措辞下出现，
        // 这里按「系统提示一律可见」处理：它们本来就短且信息密度高。
        return true;
    }
    match message.variant.card_kind() {
        Some(kind) if kind == TOOL_CONFIRM_CARD => {
            confirm_status(message).is_none_or(|status| status != "resolved")
        }
        Some(kind) if kind == ACP_PERMISSION_CARD => {
            confirm_status(message).is_none_or(|status| status != "resolved")
        }
        Some(kind) if kind == TOOL_CARD => ToolCardData::from_json(&message.content)
            .is_some_and(|data| data.success == Some(false)),
        Some(kind) if kind == SUBAGENT_CARD => SubAgentCardData::from_json(&message.content)
            .is_some_and(|data| data.success == Some(false)),
        _ => false,
    }
}

/// 是否仍需用户操作（决定是否显示输入区上方的决策栏）。
pub fn is_pending_decision(message: &ChatMessageUI) -> bool {
    match message.variant.card_kind() {
        Some(kind) if kind == TOOL_CONFIRM_CARD || kind == ACP_PERMISSION_CARD => {
            confirm_status(message).is_some_and(|status| status == "pending")
        }
        _ => false,
    }
}

fn confirm_status(message: &ChatMessageUI) -> Option<String> {
    ToolConfirmCardData::from_json(&message.content)
        .map(|data| data.status)
        .or_else(|| {
            crate::agent_cards::AcpPermissionCardData::from_json(&message.content)
                .map(|data| data.status)
        })
}

/// 消息是否算「结论」候选：助手正文（非卡片、非状态）。
fn is_answer_candidate(message: &ChatMessageUI) -> bool {
    matches!(message.role, ChatRole::Assistant)
        && matches!(message.variant, MessageVariant::Text)
        && !message.content.trim().is_empty()
}

/// 把扁平消息流投影成轮次。
///
/// 切分规则：每条 user 消息开启一个新轮次；第一条 user 之前的消息归入一个
/// preamble 轮（`head` 为 `None`）。
pub fn project_turns<'a>(
    messages: &'a [ChatMessageUI],
    timings: &TurnTimings,
) -> Vec<TurnProjection<'a>> {
    let mut turns: Vec<TurnProjection<'a>> = Vec::new();
    let mut index = 0;

    while index < messages.len() {
        let head = (messages[index].role == ChatRole::User).then_some(&messages[index]);
        let mut body: Vec<&ChatMessageUI> = Vec::new();
        if head.is_some() {
            index += 1;
        }
        while index < messages.len() && messages[index].role != ChatRole::User {
            body.push(&messages[index]);
            index += 1;
        }
        turns.push(build_turn(head, body, timings));
    }

    turns
}

fn build_turn<'a>(
    head: Option<&'a ChatMessageUI>,
    body: Vec<&'a ChatMessageUI>,
    timings: &TurnTimings,
) -> TurnProjection<'a> {
    // 结论取**最后一次**助手正文：中途的插话属于过程。
    let answer_index = body
        .iter()
        .rposition(|message| is_answer_candidate(message));

    let mut process = Vec::new();
    let mut answer = Vec::new();
    let mut risks = Vec::new();
    let mut live = false;

    for (position, message) in body.into_iter().enumerate() {
        if is_live_message(message) {
            live = true;
        }
        if is_risk_message(message) {
            risks.push(message);
        }
        // 风险项**同时**留在过程序列里，顺序不变 —— `risks` 只是它的视图。
        // 抽走它们会让失败卡片被渲染到轮次末尾（用户看到的「一直钉在底部」）。
        if Some(position) == answer_index {
            answer.push(message);
        } else {
            process.push(message);
        }
    }

    let turn_id = head
        .and_then(|message| message.turn_id.clone())
        .or_else(|| process.first().and_then(|message| message.turn_id.clone()))
        .or_else(|| answer.first().and_then(|message| message.turn_id.clone()))
        .or_else(|| risks.first().and_then(|message| message.turn_id.clone()));

    let key = turn_id
        .clone()
        .or_else(|| head.map(|message| message.id.clone()))
        .or_else(|| {
            process
                .first()
                .or(answer.first())
                .or(risks.first())
                .map(|message| message.id.clone())
        })
        .unwrap_or_else(|| "preamble".to_string());

    let timing = turn_id
        .as_deref()
        .and_then(|id| timings.get(id))
        .unwrap_or_default();
    let outcome = turn_id.as_deref().and_then(|id| timings.outcome(id));

    TurnProjection {
        key,
        turn_id,
        timing,
        outcome,
        head,
        process,
        answer,
        risks,
        live,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent_cards::{AcpPermissionCardData, TOOL_CARD, TOOL_CONFIRM_CARD, ToolCardData};

    fn tool_card(call_id: &str, name: &str, success: Option<bool>) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CARD,
            ToolCardData {
                call_id: call_id.to_string(),
                tool_name: name.to_string(),
                action: agent_runtime::ToolAction::from_tool_name(name),
                target_id: None,
                target_label: None,
                input_summary: String::new(),
                input_json: String::new(),
                running: success.is_none(),
                success,
                summary: String::new(),
                data_text: String::new(),
                file_changes: Vec::new(),
                duration_ms: None,
            }
            .to_json(),
        )
    }

    fn confirm_card(call_id: &str, status: &str) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CONFIRM_CARD,
            ToolConfirmCardData {
                call_id: call_id.to_string(),
                tool_name: "fs.write".to_string(),
                items: Vec::new(),
                input_summary: String::new(),
                input_json: String::new(),
                question: "允许写入?".to_string(),
                status: status.to_string(),
            }
            .to_json(),
        )
    }

    fn user(text: &str, turn: &str) -> ChatMessageUI {
        ChatMessageUI::user(text).with_turn_id(Some(turn))
    }

    fn assistant(text: &str, turn: &str) -> ChatMessageUI {
        ChatMessageUI::assistant(text).with_turn_id(Some(turn))
    }

    #[test]
    fn splits_turns_on_user_messages() {
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            assistant("A1", "t1"),
            user("Q2", "t2"),
            assistant("A2", "t2"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(2, turns.len());
        assert_eq!(Some("Q1"), turns[0].head.map(|m| m.content.as_str()));
        assert_eq!(1, turns[0].process.len());
        assert_eq!(vec!["A1"], as_contents(&turns[0].answer));
        assert_eq!(Some("Q2"), turns[1].head.map(|m| m.content.as_str()));
        assert!(turns[1].process.is_empty());
        assert_eq!(vec!["A2"], as_contents(&turns[1].answer));
    }

    #[test]
    fn messages_before_the_first_user_form_a_preamble_turn() {
        let preamble = ChatMessageUI::system("会话已恢复");
        let expected_key = preamble.id.clone();
        let messages = vec![preamble, user("Q1", "t1"), assistant("A1", "t1")];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(2, turns.len());
        assert!(turns[0].head.is_none());
        assert_eq!(None, turns[0].turn_id);
        assert_eq!(expected_key, turns[0].key, "无轮次时身份退化为首条消息 id");
        assert_eq!(1, turns[0].risks.len(), "系统提示属风险区，不被折叠");
    }

    #[test]
    fn only_the_last_assistant_text_becomes_the_answer() {
        let messages = vec![
            user("Q1", "t1"),
            assistant("先说一半", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            assistant("最终结论", "t1"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(vec!["最终结论"], as_contents(&turns[0].answer));
        // 中途插话与工具卡片都留在过程区。
        let process = as_contents(&turns[0].process);
        assert_eq!("先说一半", process[0]);
        assert_eq!(2, process.len());
    }

    #[test]
    fn a_turn_without_assistant_text_keeps_everything_in_process() {
        let messages = vec![user("Q1", "t1"), tool_card("c1", "fs.read", Some(true))];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert!(turns[0].answer.is_empty());
        assert_eq!(1, turns[0].process.len());
    }

    #[test]
    fn pending_confirmation_is_a_risk_and_keeps_its_place_in_process() {
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            confirm_card("c1", "pending"),
            confirm_card("c2", "resolved"),
            assistant("A1", "t1"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(1, turns[0].risks.len());
        assert_eq!("c1", call_id_of(turns[0].risks[0]));
        assert!(turns[0].has_pending_decision());
        // 待决策卡片**在**过程序列里（收起时会被单独放行），另外两张是历史步骤。
        assert_eq!(3, turns[0].process.len());
    }

    #[test]
    fn a_failed_tool_keeps_its_place_in_the_process_sequence() {
        // 失败的工具卡是风险项，但**不能**被挪到轮次末尾：它必须和相邻步骤保持原顺序，
        // 否则表现就是「执行失败的工具一直钉在底部」。
        let messages = vec![
            user("Q1", "t1"),
            tool_card("ok", "fs.read", Some(true)),
            ChatMessageUI::status("继续响应", false),
            tool_card("boom", "ssh.exec", Some(false)),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        let actual: Vec<String> = turns[0]
            .process
            .iter()
            .map(|message| message.id.clone())
            .collect();
        let expected: Vec<String> = messages[1..]
            .iter()
            .map(|message| message.id.clone())
            .collect();
        assert_eq!(expected, actual, "过程序列必须与消息原始顺序逐条一致");
        assert_eq!(1, turns[0].risks.len());
        assert_eq!("boom", call_id_of(turns[0].risks[0]));
        assert_eq!(
            turns[0].process.last().map(|message| message.id.as_str()),
            turns[0].risks.last().map(|message| message.id.as_str()),
            "风险项是过程序列的**有序子集**：位置也一致"
        );
    }

    #[test]
    fn failed_tool_card_is_a_risk_but_successful_one_is_not() {
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            tool_card("c2", "ssh.exec", Some(false)),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(1, turns[0].risks.len());
        assert_eq!("c2", call_id_of(turns[0].risks[0]));
        assert_eq!(2, turns[0].process.len());
    }

    #[test]
    fn live_turn_expands_process_by_default_and_finished_turn_does_not() {
        let mut streaming = assistant("", "t1");
        streaming.is_streaming = true;
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", None),
            streaming,
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert!(turns[0].live);
        assert!(turns[0].default_process_expanded());

        let finished = vec![user("Q2", "t2"), tool_card("c2", "fs.read", Some(true))];
        let turns = project_turns(&finished, &TurnTimings::new());
        assert!(!turns[0].live);
        assert!(!turns[0].default_process_expanded());
    }

    #[test]
    fn a_running_tool_without_streaming_text_still_keeps_process_expanded() {
        // 只有工具在执行、还没有流式文本：用户必须能看见它，不能被默认收起。
        let messages = vec![user("Q1", "t1"), tool_card("c1", "ssh.exec", None)];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert!(turns[0].live);
        assert!(turns[0].default_process_expanded());
    }

    #[test]
    fn an_unfinished_status_message_keeps_process_expanded() {
        let messages = vec![user("Q1", "t1"), ChatMessageUI::status("思考中", false)];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert!(turns[0].live);
    }

    #[test]
    fn breakdown_text_truncates_with_an_ellipsis_and_never_guesses_labels() {
        let breakdown = vec![
            ("fs.read".to_string(), 2),
            ("fs.grep".to_string(), 1),
            ("ssh.exec".to_string(), 1),
        ];

        assert_eq!(
            Some("fs.read ×2 · fs.grep ×1 · …".to_string()),
            breakdown_text(&breakdown, 2)
        );
        assert_eq!(
            Some("fs.read ×2 · fs.grep ×1 · ssh.exec ×1".to_string()),
            breakdown_text(&breakdown, 3)
        );
        assert_eq!(None, breakdown_text(&[], 2));
        assert_eq!(None, breakdown_text(&breakdown, 0));
    }

    #[test]
    fn step_count_and_breakdown_only_count_tool_cards() {
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            tool_card("c2", "fs.read", Some(true)),
            tool_card("c3", "fs.grep", Some(true)),
            ChatMessageUI::status("思考中", true),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(3, turns[0].step_count());
        assert_eq!(
            vec![("fs.read".to_string(), 2), ("fs.grep".to_string(), 1)],
            turns[0].tool_breakdown()
        );
    }

    #[test]
    fn timings_are_read_by_turn_id_and_never_invented() {
        let mut timings = TurnTimings::new();
        timings.note_started("t1", 100);
        timings.note_finished("t1", 108);

        let messages = vec![user("Q1", "t1"), assistant("A1", "t1"), user("Q2", "t2")];
        let turns = project_turns(&messages, &timings);

        assert_eq!(Some(8), turns[0].timing.duration_secs());
        assert_eq!(None, turns[1].timing.duration_secs());
        assert_eq!(None, turns[1].timing.started_at);
    }

    #[test]
    fn note_started_keeps_the_earliest_timestamp() {
        let mut timings = TurnTimings::new();
        timings.note_started("t1", 100);
        timings.note_started("t1", 500);
        timings.note_finished("t1", 700);
        timings.note_finished("t1", 900);

        assert_eq!(
            Some(TurnTiming {
                started_at: Some(100),
                finished_at: Some(700)
            }),
            timings.get("t1")
        );
    }

    #[test]
    fn turn_key_falls_back_to_the_head_message_id_when_turn_id_is_missing() {
        let head = ChatMessageUI::user("Q1");
        let messages = vec![head, ChatMessageUI::assistant("A1")];
        let expected = messages[0].id.clone();

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(None, turns[0].turn_id.as_deref());
        assert_eq!(expected, turns[0].key);
    }

    #[test]
    fn turn_identity_comes_from_the_head_not_from_stale_trailing_events() {
        // 切分边界是 user 消息；轮次身份取 head 自带的 turn_id。
        // 迟到的旧轮事件即使物理上落在本轮切片里，也不会改写本轮身份。
        let messages = vec![
            user("Q1", "t1"),
            assistant("A1", "t1"),
            user("Q2", "t2"),
            tool_card("late", "fs.read", Some(true)).with_turn_id(Some("t1")),
            assistant("A2", "t2"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(2, turns.len());
        assert_eq!(Some("t1"), turns[0].turn_id.as_deref());
        assert_eq!(Some("t2"), turns[1].turn_id.as_deref());
        assert_eq!("t2", turns[1].key);
        assert_eq!(1, turns[1].process.len());
    }

    #[test]
    fn a_system_notice_does_not_mark_the_turn_as_failed() {
        // ACP 控制面 / 上下文边界这类系统提示与失败无关，不能把页脚刷成红色「失败」。
        let messages = vec![
            user("Q1", "t1"),
            ChatMessageUI::system("正在创建 ACP 会话"),
            assistant("A1", "t1"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());

        assert_eq!(None, turns[0].outcome);
        assert!(!turns[0].has_failure(), "普通系统提示不是失败");
        assert!(!turns[0].is_cancelled());
    }

    #[test]
    fn turn_outcome_comes_from_events_not_from_message_text() {
        let messages = vec![user("Q1", "t1"), assistant("A1", "t1")];

        let mut timings = TurnTimings::new();
        timings.note_outcome("t1", TurnOutcome::Failed);
        let turns = project_turns(&messages, &timings);
        assert!(turns[0].has_failure(), "终态事件报失败 ⇒ 本轮失败");
        assert!(!turns[0].is_cancelled());

        let mut timings = TurnTimings::new();
        timings.note_outcome("t1", TurnOutcome::Cancelled);
        let turns = project_turns(&messages, &timings);
        assert!(turns[0].is_cancelled());
        assert!(!turns[0].has_failure(), "取消不是失败");

        let mut timings = TurnTimings::new();
        timings.note_outcome("t1", TurnOutcome::Completed);
        let turns = project_turns(&messages, &timings);
        assert!(!turns[0].has_failure());
        assert!(!turns[0].is_cancelled());
    }

    fn as_contents<'a>(messages: &[&'a ChatMessageUI]) -> Vec<&'a str> {
        messages.iter().map(|m| m.content.as_str()).collect()
    }

    fn edit_card(call_id: &str, path: &str, old: &str, new: &str) -> ChatMessageUI {
        ChatMessageUI::card(
            TOOL_CARD,
            ToolCardData {
                call_id: call_id.to_string(),
                tool_name: "Edit".to_string(),
                action: agent_runtime::ToolAction::Edit,
                target_id: None,
                target_label: None,
                input_summary: String::new(),
                input_json: String::new(),
                running: false,
                success: Some(true),
                summary: String::new(),
                data_text: String::new(),
                file_changes: vec![FileChangeSummary::from_texts(path, Some(old), new)],
                duration_ms: None,
            }
            .to_json(),
        )
    }

    #[test]
    fn changed_files_collect_edits_across_the_turn() {
        let messages = vec![
            user("Q1", "t1"),
            tool_card("c1", "fs.read", Some(true)),
            edit_card("c2", "src/a.rs", "a\n", "b\n"),
            edit_card("c3", "src/b.rs", "x\n", "y\n"),
            assistant("A1", "t1"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());
        let changed = turns[0].changed_files();

        assert_eq!(
            vec!["src/a.rs", "src/b.rs"],
            changed.iter().map(|c| c.path.as_str()).collect::<Vec<_>>(),
            "只读的工具不算改动"
        );
    }

    #[test]
    fn editing_the_same_file_twice_reports_it_once() {
        let messages = vec![
            user("Q1", "t1"),
            edit_card("c1", "src/a.rs", "a\n", "b\n"),
            edit_card("c2", "src/a.rs", "b\n", "b\nc\n"),
            assistant("A1", "t1"),
        ];

        let turns = project_turns(&messages, &TurnTimings::new());
        let changed = turns[0].changed_files();

        // 去重而不是累加:第二次编辑的统计覆盖第一次的。
        assert_eq!(1, changed.len());
        assert_eq!(1, changed[0].added);
        assert_eq!(0, changed[0].removed);
    }

    fn call_id_of(message: &ChatMessageUI) -> String {
        ToolConfirmCardData::from_json(&message.content)
            .map(|data| data.call_id)
            .or_else(|| ToolCardData::from_json(&message.content).map(|data| data.call_id))
            .or_else(|| {
                AcpPermissionCardData::from_json(&message.content).map(|data| data.request_id)
            })
            .unwrap_or_default()
    }
}
