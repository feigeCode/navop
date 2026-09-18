//! 会话内搜索：把「查询 → 命中轮次」这一步做成纯逻辑。
//!
//! 形状照 Waku `transcript_search.rs`，按 Navop 的轮次阅读模型落位：
//!
//! - **字面量匹配**，不做正则、不做全词匹配；默认大小写不敏感（可显式区分）。
//! - **命中以「轮次」为单位**：跳转的对象是轮次，不是某一行文本。
//! - **命中数有上限**，超限时置 `truncated` 让 UI 显式提示，而不是静默少算。
//! - **generation 校验**：每次重算 / 关闭都递增，跳转回调里比对，不匹配直接放弃
//!   （防止键盘连按时旧的定位落到新查询上）。
//!
//! 明确不做：**子串高亮**。Markdown 渲染后的正文没有可靠的字符区间映射，
//! 高亮整条轮次是诚实的做法；假装标出精确区间属于虚构。

use crate::agent_cards::{
    ACP_PERMISSION_CARD, AcpPermissionCardData, SUBAGENT_CARD, SubAgentCardData, TOOL_CARD,
    TOOL_CONFIRM_CARD, ToolCardData, ToolConfirmCardData,
};
use crate::turn::TurnProjection;
use crate::{ChatMessageUI, MessageVariant};

/// 命中总数上限；超出即截断并提示。
pub const MAX_SEARCH_HITS: usize = 2000;

/// 一个轮次上的命中。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SearchHit {
    /// 轮次在 [`crate::project_turns`] 结果中的下标（与渲染顺序一致）。
    pub turn_index: usize,
    /// 该轮内的命中次数（≥ 1）。
    pub count: usize,
}

/// 会话内搜索状态（纯逻辑，可单测）。
#[derive(Clone, Debug, Default)]
pub struct TranscriptSearch {
    query: String,
    case_sensitive: bool,
    hits: Vec<SearchHit>,
    total: usize,
    truncated: bool,
    cursor: usize,
    generation: u64,
}

impl TranscriptSearch {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn query(&self) -> &str {
        &self.query
    }

    /// 是否有有效查询（空串 / 全空白不算）。
    pub fn is_active(&self) -> bool {
        !self.query.trim().is_empty()
    }

    pub fn case_sensitive(&self) -> bool {
        self.case_sensitive
    }

    pub fn hits(&self) -> &[SearchHit] {
        &self.hits
    }

    pub fn total(&self) -> usize {
        self.total
    }

    /// 命中数是否被上限截断。
    pub fn truncated(&self) -> bool {
        self.truncated
    }

    /// 代数；跳转回调据此丢弃迟到的定位。
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// 当前命中序号（0-based）。没有命中时为 0。
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// `(当前第几个, 共几个)`，从 1 开始计数；无命中时为 `None`。
    pub fn progress(&self) -> Option<(usize, usize)> {
        (self.total > 0).then(|| (self.cursor + 1, self.total))
    }

    /// 当前命中所在轮次。
    pub fn current_turn_index(&self) -> Option<usize> {
        self.hit_at(self.cursor).map(|hit| hit.turn_index)
    }

    /// 某个轮次是否是当前命中（渲染层据此区分「命中」与「当前命中」）。
    pub fn is_current_turn(&self, turn_index: usize) -> bool {
        self.current_turn_index() == Some(turn_index)
    }

    /// 某个轮次是否命中（任意位置）。
    pub fn hits_turn(&self, turn_index: usize) -> bool {
        self.hits.iter().any(|hit| hit.turn_index == turn_index)
    }

    /// 设置查询并重算。查询变化会把游标回到第一个命中。
    pub fn set_query(&mut self, query: impl Into<String>, messages: &[ChatMessageUI]) {
        let query = query.into();
        if query == self.query {
            return;
        }
        self.query = query;
        self.cursor = 0;
        self.refresh(messages);
    }

    /// 切换大小写敏感并重算。
    pub fn set_case_sensitive(&mut self, case_sensitive: bool, messages: &[ChatMessageUI]) {
        if case_sensitive == self.case_sensitive {
            return;
        }
        self.case_sensitive = case_sensitive;
        self.cursor = 0;
        self.refresh(messages);
    }

    /// 按当前查询重算命中。文本变化（流式输出）后调用。
    pub fn refresh(&mut self, messages: &[ChatMessageUI]) {
        self.generation = self.generation.wrapping_add(1);
        self.hits.clear();
        self.total = 0;
        self.truncated = false;

        let needle = self.normalized_query();
        if needle.is_empty() {
            self.cursor = 0;
            return;
        }

        // 搜索不需要时间信息：`TurnTimings::new()` 只是空 map，不分配堆内存。
        // 传入空表而不是让投影层猜时间，保证「没有时间」不会变成「时间 = 0」。
        let timings = crate::TurnTimings::new();
        for (turn_index, turn) in crate::turn::project_turns(messages, &timings)
            .iter()
            .enumerate()
        {
            let mut count = 0;
            for text in turn_texts(turn) {
                count += count_matches(&text, &needle, self.case_sensitive);
                if self.total + count >= MAX_SEARCH_HITS {
                    break;
                }
            }
            if count == 0 {
                continue;
            }
            let remaining = MAX_SEARCH_HITS.saturating_sub(self.total);
            let count = count.min(remaining);
            self.total += count;
            self.hits.push(SearchHit { turn_index, count });
            if self.total >= MAX_SEARCH_HITS {
                self.truncated = true;
                break;
            }
        }

        if self.total == 0 {
            self.cursor = 0;
        } else if self.cursor >= self.total {
            self.cursor = self.total - 1;
        }
    }

    /// 移动游标并返回要跳转到的轮次下标；无命中时返回 `None`。
    ///
    /// 环绕：到末尾再往后回到第一个，反之亦然。
    pub fn step(&mut self, forward: bool) -> Option<usize> {
        if self.total == 0 {
            return None;
        }
        self.generation = self.generation.wrapping_add(1);
        self.cursor = if forward {
            (self.cursor + 1) % self.total
        } else {
            (self.cursor + self.total - 1) % self.total
        };
        self.current_turn_index()
    }

    /// 清空查询与命中（关闭 findbar / 切换会话时调用）。
    pub fn clear(&mut self) {
        self.query.clear();
        self.hits.clear();
        self.total = 0;
        self.truncated = false;
        self.cursor = 0;
        self.generation = self.generation.wrapping_add(1);
    }

    fn normalized_query(&self) -> String {
        let trimmed = self.query.trim();
        if self.case_sensitive {
            trimmed.to_string()
        } else {
            trimmed.to_lowercase()
        }
    }

    fn hit_at(&self, cursor: usize) -> Option<&SearchHit> {
        let mut offset = 0;
        for hit in &self.hits {
            if cursor < offset + hit.count {
                return Some(hit);
            }
            offset += hit.count;
        }
        None
    }
}

/// 一个轮次里参与搜索的文本：提问、过程、结论、风险，全部计入。
pub fn turn_texts(turn: &TurnProjection<'_>) -> Vec<String> {
    let mut out = Vec::new();
    for message in turn
        .head
        .into_iter()
        .chain(turn.process.iter().copied())
        .chain(turn.answer.iter().copied())
        .chain(turn.risks.iter().copied())
    {
        let text = message_search_text(message);
        if !text.trim().is_empty() {
            out.push(text);
        }
    }
    out
}

/// 一条消息里参与搜索的**人类可读**文本。
///
/// 卡片不搜原始 JSON：`content` 对卡片来说就是 JSON 本身，直接搜会把内部字段名
/// 变成命中项，而用户看到的只有渲染后的摘要。这里只取卡片上真正会显示出来的字段。
pub fn message_search_text(message: &ChatMessageUI) -> String {
    let mut parts: Vec<String> = Vec::new();
    match &message.variant {
        MessageVariant::Text | MessageVariant::SqlResult => {
            parts.push(message.content.clone());
        }
        MessageVariant::Status { title, .. } => parts.push(title.clone()),
        MessageVariant::Card { kind } => {
            if let Some(text) = card_search_text(kind, &message.content) {
                parts.push(text);
            }
        }
    }
    parts.push(message.reasoning_content.clone());
    parts.retain(|part| !part.trim().is_empty());
    parts.join("\n")
}

fn card_search_text(kind: &str, content: &str) -> Option<String> {
    match kind {
        TOOL_CARD => {
            let data = ToolCardData::from_json(content)?;
            Some(
                [
                    data.tool_name,
                    data.target_label.unwrap_or_default(),
                    data.target_id.unwrap_or_default(),
                    data.input_summary,
                    data.summary,
                    data.data_text,
                ]
                .join("\n"),
            )
        }
        TOOL_CONFIRM_CARD => {
            let data = ToolConfirmCardData::from_json(content)?;
            let mut parts = vec![data.tool_name, data.input_summary, data.question];
            parts.extend(data.items.into_iter().flat_map(|item| {
                [item.tool_name, item.input_summary]
            }));
            Some(parts.join("\n"))
        }
        ACP_PERMISSION_CARD => {
            let data = AcpPermissionCardData::from_json(content)?;
            Some([data.tool_name, data.summary].join("\n"))
        }
        SUBAGENT_CARD => {
            let data = SubAgentCardData::from_json(content)?;
            Some([data.name, data.task, data.summary].join("\n"))
        }
        // `chart-json` 之类不是文本内容，不参与搜索。
        _ => None,
    }
}

/// 字面量出现次数；`needle` 必须已经按大小写规则归一化。
pub fn count_matches(haystack: &str, needle: &str, case_sensitive: bool) -> usize {
    if needle.is_empty() {
        return 0;
    }
    if case_sensitive {
        return haystack.matches(needle).count();
    }
    let haystack = haystack.to_lowercase();
    if haystack.len() < needle.len() {
        return 0;
    }
    haystack.matches(needle).count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ChatMessageUI;

    fn texts(messages: &[ChatMessageUI]) -> Vec<String> {
        messages
            .iter()
            .map(message_search_text)
            .collect::<Vec<_>>()
    }

    fn search(query: &str, messages: &[ChatMessageUI]) -> TranscriptSearch {
        let mut search = TranscriptSearch::new();
        search.set_query(query, messages);
        search
    }

    fn conversation() -> Vec<ChatMessageUI> {
        vec![
            ChatMessageUI::user("MAX_TRANSCRIPT_MESSAGES 是干什么的？"),
            ChatMessageUI::assistant("它是硬预算：enforce_budget 会静默裁掉最旧的消息。"),
            ChatMessageUI::user("那 max_transcript_messages 的常量在哪？"),
            ChatMessageUI::assistant("在 agent_transcript.rs 顶部。"),
        ]
    }

    #[test]
    fn literal_matching_is_case_insensitive_by_default() {
        let messages = conversation();

        let search = search("MAX_TRANSCRIPT_MESSAGES", &messages);

        // 第 1 轮 1 次 + 第 2 轮 1 次（小写那个也算）。
        assert_eq!(2, search.total());
        assert_eq!(2, search.hits().len());
        assert_eq!(1, search.hits()[0].count);
        assert_eq!(1, search.hits()[1].count);
    }

    #[test]
    fn case_sensitive_matching_can_be_switched_on() {
        let messages = conversation();
        let mut search = search("MAX_TRANSCRIPT_MESSAGES", &messages);

        search.set_case_sensitive(true, &messages);

        assert_eq!(1, search.total());
        assert_eq!(0, search.hits()[0].turn_index);
    }

    #[test]
    fn empty_or_whitespace_query_is_inactive_and_yields_no_hits() {
        let messages = conversation();

        let mut search = TranscriptSearch::new();
        assert!(!search.is_active());
        assert_eq!(None, search.progress());

        search.set_query("   ", &messages);
        assert!(!search.is_active());
        assert_eq!(0, search.total());
        assert_eq!(None, search.step(true));
    }

    #[test]
    fn hits_are_grouped_by_turn_in_document_order() {
        let messages = conversation();

        let search = search("预算", &messages);

        // 「硬预算」在第一条助手回复里，属于第 0 轮。
        assert_eq!(1, search.total());
        assert_eq!(Some(0), search.current_turn_index());
        assert!(search.hits_turn(0));
        assert!(!search.hits_turn(1));
    }

    #[test]
    fn stepping_wraps_around_in_both_directions() {
        let messages = conversation();
        let mut search = search("的", &messages);
        let total = search.total();
        assert!(total >= 2);

        assert_eq!(Some((1, total)), search.progress());
        let second = search.step(true).expect("forward");
        assert_eq!(Some((2, total)), search.progress());
        assert_ne!(search.current_turn_index(), None);

        // 走到末尾之后回到第一个。
        for _ in 2..total {
            search.step(true);
        }
        assert_eq!(Some((total, total)), search.progress());
        let first = search.step(true).expect("wrap to first");
        assert_eq!(Some((1, total)), search.progress());
        assert!(first <= second.max(first));

        // 从第一个再往回绕到最后一个。
        let last = search.step(false).expect("wrap to last");
        assert_eq!(Some((total, total)), search.progress());
        assert!(last >= first);
    }

    #[test]
    fn set_query_resets_the_cursor_but_step_keeps_the_generation_moving() {
        let messages = conversation();
        let mut search = search("的", &messages);
        search.step(true);
        assert_eq!(1, search.cursor());

        let before = search.generation();
        search.set_query("预算", &messages);
        assert_eq!(0, search.cursor());
        assert!(search.generation() > before);
    }

    #[test]
    fn refresh_bumps_generation_so_stale_jumps_can_be_dropped() {
        let messages = conversation();
        let mut search = search("预算", &messages);

        let before = search.generation();
        search.refresh(&messages);
        assert!(search.generation() > before);

        // 关闭时也递增：迟到的跳转回调必须失效。
        let before = search.generation();
        search.clear();
        assert!(search.generation() > before);
        assert!(!search.is_active());
    }

    #[test]
    fn cursor_is_clamped_when_the_transcript_shrinks() {
        let messages = conversation();
        let mut search = search("的", &messages);
        for _ in 0..search.total() {
            search.step(true);
        }
        assert!(search.total() > 0);

        // 会话被裁断到只剩一条消息：游标必须收回到有效范围，不能越界。
        search.refresh(&messages[..1]);

        assert!(search.cursor() < search.total().max(1));
        assert!(search.current_turn_index().is_some() || search.total() == 0);
    }

    #[test]
    fn tool_cards_match_on_their_visible_fields_not_raw_json() {
        let mut card = ChatMessageUI::card(
            TOOL_CARD,
            ToolCardData {
                call_id: "c1".into(),
                tool_name: "fs.read".into(),
                target_id: None,
                target_label: None,
                input_summary: "agent_transcript.rs:29-41".into(),
                input_json: r#"{"path":"agent_transcript.rs"}"#.into(),
                running: false,
                success: Some(true),
                summary: "读取完成".into(),
                data_text: "const MAX_TRANSCRIPT_MESSAGES: usize = 500;".into(),
            }
            .to_json(),
        );
        // 内部字段名不应成为命中项。
        card.content.push_str("");

        let by_visible = search("enforce_budget", &[card.clone()]);
        assert_eq!(0, by_visible.total());

        let by_summary = search("读取完成", &[card.clone()]);
        assert_eq!(1, by_summary.total());

        let by_target = search("agent_transcript.rs", &[card]);
        assert_eq!(1, by_target.total());
    }

    #[test]
    fn status_titles_are_searchable_but_chart_cards_are_not() {
        let status = ChatMessageUI::status("正在检索 enforce_budget", false);
        let chart = ChatMessageUI::card("chart-json", r#"{"chart_type":"bar","x":"enforce_budget"}"#);

        assert_eq!(1, search("enforce_budget", &[status]).total());
        assert_eq!(0, search("enforce_budget", &[chart]).total());
    }

    #[test]
    fn every_occurrence_inside_one_turn_is_counted() {
        // 提问与结论同属一轮：命中数按**出现次数**累计，不是按轮次数。
        let messages = vec![ChatMessageUI::user("的的的"), ChatMessageUI::assistant("的")];

        let search = search("的", &messages);

        assert_eq!(1, search.hits().len());
        assert_eq!(4, search.hits()[0].count);
        assert_eq!(4, search.total());
        assert_eq!(Some(0), search.current_turn_index());
    }

    #[test]
    fn count_matches_handles_unicode_and_overlaps_free_of_panics() {
        assert_eq!(2, count_matches("预算预算", "预算", true));
        assert_eq!(0, count_matches("短", "更长的查询", true));
        assert_eq!(0, count_matches("abc", "", true));
        assert_eq!(2, count_matches("AbCabc", "abc", false));
        assert_eq!(1, count_matches("AbCabc", "abc", true));
    }

    #[test]
    fn normalizing_text_with_case_folding_chars_never_panics() {
        // 大小写折叠可能改变字节长度（如 'İ'），命中只按次数计，不受影响。
        let messages = vec![ChatMessageUI::user("İSTANBUL ve istanbul")];

        let search = search("istanbul", &messages);

        assert!(search.total() >= 1);
    }

    #[test]
    fn search_text_of_plain_messages_keeps_answer_and_reasoning() {
        let message = ChatMessageUI::assistant("回答").with_reasoning_content("推理");

        let text = message_search_text(&message);

        assert!(text.contains("回答"));
        assert!(text.contains("推理"));
    }

    #[test]
    fn malformed_cards_do_not_break_text_extraction() {
        let card = ChatMessageUI::card(TOOL_CARD, "{ not json");

        assert_eq!("", message_search_text(&card));
        assert_eq!(vec![""], texts(&[card]));
    }
}
