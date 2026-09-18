//! ACP 会话列表:协议响应 → 展示投影,以及「能不能列 / 怎么打开」的能力判定。
//!
//! 这一层不碰 GPUI、不碰网络,只做两件容易写错的事:把 `session/list` 的响应
//! 翻成视图要的形状,以及集中回答能力问题。之所以把判定收在这里,是因为
//! 不变量 11(**能力缺失就不显示**)最容易在渲染层被写坏——散着写就会冒出
//! 「点得动但一定失败」的入口。

use agent_client_protocol::schema::{AgentCapabilities, SessionInfo};

/// 一条可展示的 ACP 会话。
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct AcpSessionSummary {
    /// 协议会话 id。这是稳定身份,标题只用来显示,不做地址。
    pub(crate) id: String,
    /// Agent 给的标题。缺失就缺省,不编。
    pub(crate) title: Option<String>,
    /// ISO 8601 的最后活动时间,原样透传;格式化成「几天前」是渲染层的事。
    pub(crate) updated_at: Option<String>,
    pub(crate) cwd: std::path::PathBuf,
}

impl AcpSessionSummary {
    /// 展示标签:标题为空则退回会话 id。
    ///
    /// 刻意**不**造「未命名会话」这类文案——列表项必须能和 agent 那边的 id 对上。
    pub(crate) fn label(&self) -> &str {
        self.title
            .as_deref()
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .unwrap_or(&self.id)
    }
}

/// 打开一个历史会话的方式。协议给了两条路,语义不同,不能混着用。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AcpSessionOpen {
    /// `session/load`:回放历史,客户端据此重建转录。
    Load,
    /// `session/resume`:接着跑,不回放历史。
    Resume,
}

/// agent 是否声明支持 `session/list`。不支持就别显示列表。
pub(crate) fn acp_session_list_supported(capabilities: &AgentCapabilities) -> bool {
    capabilities.session_capabilities.list.is_some()
}

/// 打开历史会话的方式;两条路都不支持就是 `None`。
///
/// `load` 优先于 `resume`:前者能带回历史,对用户更有用,也是协议里更明确的一条
/// (`session/load` 走顶层 `load_session` 能力,不随 `session_capabilities` 变)。
pub(crate) fn acp_session_open_kind(capabilities: &AgentCapabilities) -> Option<AcpSessionOpen> {
    if capabilities.load_session {
        return Some(AcpSessionOpen::Load);
    }
    capabilities
        .session_capabilities
        .resume
        .is_some()
        .then_some(AcpSessionOpen::Resume)
}

/// 把 `session/list` 拉回来的会话投影成列表项,按 `updated_at` 倒序。
///
/// 入参是翻页后的原始 [`SessionInfo`] 列表(见 `AcpConnection::list_all_sessions`),
/// 不要求调用方先包一个 `ListSessionsResponse`。
///
/// ISO 8601 的字典序就是时间序,所以不引时间库;没有时间戳的排在最后
/// (空 `Option` 在倒序里自然落底),而不是假装它「最新」。
pub(crate) fn acp_session_summaries(sessions: &[SessionInfo]) -> Vec<AcpSessionSummary> {
    let mut sessions: Vec<AcpSessionSummary> = sessions
        .iter()
        .map(|info| AcpSessionSummary {
            id: info.session_id.to_string(),
            title: info.title.clone(),
            updated_at: info.updated_at.clone(),
            cwd: info.cwd.clone(),
        })
        .collect();
    sessions.sort_by(|left, right| right.updated_at.cmp(&left.updated_at));
    sessions
}

#[cfg(test)]
mod tests {
    use agent_client_protocol::schema::{
        AgentCapabilities, SessionCapabilities, SessionId, SessionInfo, SessionListCapabilities,
        SessionResumeCapabilities,
    };

    use super::{
        AcpSessionOpen, acp_session_list_supported, acp_session_open_kind, acp_session_summaries,
    };

    fn info(id: &'static str, title: Option<&str>, updated_at: Option<&str>) -> SessionInfo {
        SessionInfo::new(id, "/work")
            .title(title.map(str::to_string))
            .updated_at(updated_at.map(str::to_string))
    }

    #[test]
    fn list_capability_is_read_from_session_capabilities() {
        assert!(!acp_session_list_supported(&AgentCapabilities::default()));

        let mut capabilities = AgentCapabilities::default();
        capabilities.session_capabilities = SessionCapabilities::new()
            .list(SessionListCapabilities::new());
        assert!(acp_session_list_supported(&capabilities));
    }

    #[test]
    fn load_wins_over_resume_and_absence_is_none() {
        let mut capabilities = AgentCapabilities::default();
        assert_eq!(None, acp_session_open_kind(&capabilities));

        capabilities.session_capabilities =
            SessionCapabilities::new().resume(SessionResumeCapabilities::new());
        assert_eq!(
            Some(AcpSessionOpen::Resume),
            acp_session_open_kind(&capabilities)
        );

        // 两条都支持时选 load:它能带回历史,对用户更有用。
        capabilities.load_session = true;
        assert_eq!(
            Some(AcpSessionOpen::Load),
            acp_session_open_kind(&capabilities)
        );
    }

    #[test]
    fn summaries_strip_blank_titles_and_fall_back_to_the_id() {
        let sessions = [
            info("s-blank", Some("   "), None),
            info("s-none", None, None),
            info("s-titled", Some("  Plan review  "), None),
        ];

        let summaries = acp_session_summaries(&sessions);

        let label_of = |id: &str| {
            summaries
                .iter()
                .find(|summary| summary.id == id)
                .expect("summary present")
                .label()
                .to_string()
        };
        assert_eq!("s-blank", label_of("s-blank"));
        assert_eq!("s-none", label_of("s-none"));
        assert_eq!("Plan review", label_of("s-titled"));
    }

    #[test]
    fn summaries_sort_newest_first_and_push_missing_timestamps_last() {
        let sessions = [
            info("no-timestamp", None, None),
            info("older", None, Some("2026-01-01T00:00:00Z")),
            info("newer", None, Some("2026-06-28T00:00:00Z")),
        ];

        let summaries = acp_session_summaries(&sessions);
        let ids: Vec<&str> = summaries
            .iter()
            .map(|summary| summary.id.as_str())
            .collect();

        assert_eq!(vec!["newer", "older", "no-timestamp"], ids);
    }

    #[test]
    fn summaries_keep_the_protocol_id_verbatim() {
        // id 里有空格和大写也必须原样带过去:它是后面 load/resume 的唯一地址。
        let sessions = [info(
            "Session A/b",
            Some("titled"),
            Some("2026-01-01"),
        )];

        assert_eq!("Session A/b", acp_session_summaries(&sessions)[0].id);
    }

    #[test]
    fn session_id_round_trips_through_summary() {
        // 直接钉住投影用的是协议 id,而不是标题或下标。
        let sessions = [info("abc", Some("xyz"), None)];
        let summaries = acp_session_summaries(&sessions);

        assert_eq!(SessionId::from("abc").0, summaries[0].id.as_str().into());
    }
}
