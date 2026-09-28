//! 通用会话侧边栏:数据结构 + 纯视觉渲染辅助。
//!
//! 与具体业务的会话存储完全解耦:调用方把自己的会话映射成 [`SessionSummary`] 即可。
//! 折叠 / 展开、点击交互等编排由上层视图(`ChatView`)负责,本模块只提供「长什么样」。

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Div, FontWeight, Hsla, InteractiveElement, ParentElement, SharedString, Styled, div,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
use rust_i18n::t;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 通用会话摘要(与具体业务的会话模型解耦)。
#[derive(Clone, Debug)]
pub struct SessionSummary {
    /// 会话唯一标识(字符串,兼容任意业务的 id 形态)。
    pub id: String,
    /// 显示名称。
    pub name: SharedString,
    /// 最后更新时间(Unix 秒)。
    pub updated_at: i64,
    /// 会话归属的工作区根目录。`None` 表示旧数据 / 未分组。
    pub workspace_root: Option<String>,
    /// 会话由外部 agent（ACP）承载时的来源短标识（如 `codex`）。
    ///
    /// `None` = 本地会话。行上标出来源，用户才知道点它会去连外部 agent，
    /// 而不是把这段对话当成存在本地的历史。
    pub external_agent: Option<SharedString>,
}

#[derive(Clone, Copy, Debug)]
pub struct SessionRowStyle {
    pub foreground: Hsla,
    pub muted_foreground: Hsla,
    pub selected_background: Hsla,
    pub selected_foreground: Hsla,
    pub hover_background: Hsla,
}

impl SessionRowStyle {
    pub fn from_app(cx: &App) -> Self {
        Self {
            foreground: cx.theme().foreground,
            muted_foreground: cx.theme().muted_foreground,
            selected_background: cx.theme().accent,
            selected_foreground: cx.theme().accent_foreground,
            hover_background: cx.theme().list_hover,
        }
    }
}

impl SessionSummary {
    pub fn new(id: impl Into<String>, name: impl Into<SharedString>, updated_at: i64) -> Self {
        Self {
            id: id.into(),
            name: name.into(),
            updated_at,
            workspace_root: None,
            external_agent: None,
        }
    }

    /// 附加工作区归属(链式)。
    pub fn with_workspace_root(mut self, root: Option<String>) -> Self {
        self.workspace_root = root;
        self
    }

    /// 附加「外部 agent 承载」的来源标记(链式)。
    pub fn with_external_agent(mut self, agent: Option<SharedString>) -> Self {
        self.external_agent = agent;
        self
    }
}

/// 渲染单个会话行的视觉部分。
///
/// 返回 [`Div`],调用方可继续 `.id(..).on_click(..)` 附加交互(因此交互逻辑留在上层)。
pub fn session_row(session: &SessionSummary, is_current: bool, cx: &App) -> Div {
    session_row_with_style(session, is_current, SessionRowStyle::from_app(cx))
}

pub fn session_row_with_style(
    session: &SessionSummary,
    is_current: bool,
    style: SessionRowStyle,
) -> Div {
    h_flex()
        .w_full()
        .gap_2()
        .items_center()
        .px_2()
        .py_1()
        .rounded_md()
        .cursor_pointer()
        .text_color(style.foreground)
        .when(!is_current, |this| {
            this.hover(move |this| this.bg(style.hover_background))
        })
        .when(is_current, |this| {
            this.bg(style.selected_background)
                .text_color(style.selected_foreground)
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_0p5()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .overflow_hidden()
                        .text_ellipsis()
                        .child(session.name.clone()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(if is_current {
                            style.selected_foreground.opacity(0.72)
                        } else {
                            style.muted_foreground
                        })
                        .child(match session.external_agent.as_ref() {
                            Some(agent) => format!(
                                "{} · {}",
                                agent,
                                format_timestamp(session.updated_at)
                            ),
                            None => format_timestamp(session.updated_at),
                        }),
                ),
        )
}

/// 相对时间的量级与数量，与文案分开以便纯逻辑测试。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelativeTimeUnit {
    JustNow,
    Minutes(i64),
    Hours(i64),
    Days(i64),
    Weeks(i64),
}

/// 选量级：`diff_secs` 为「距今秒数」。
pub fn relative_time_unit(diff_secs: i64) -> RelativeTimeUnit {
    const MINUTE: i64 = 60;
    const HOUR: i64 = 60 * MINUTE;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;

    let diff = diff_secs.max(0);
    if diff < MINUTE {
        RelativeTimeUnit::JustNow
    } else if diff < HOUR {
        RelativeTimeUnit::Minutes(diff / MINUTE)
    } else if diff < DAY {
        RelativeTimeUnit::Hours(diff / HOUR)
    } else if diff < WEEK {
        RelativeTimeUnit::Days(diff / DAY)
    } else {
        RelativeTimeUnit::Weeks(diff / WEEK)
    }
}

/// 量级 → 本地化文案。
pub fn relative_time_label(unit: RelativeTimeUnit) -> String {
    match unit {
        RelativeTimeUnit::JustNow => t!("AgentUi.time_just_now").to_string(),
        RelativeTimeUnit::Minutes(count) => {
            t!("AgentUi.time_minutes_ago", count = count).to_string()
        }
        RelativeTimeUnit::Hours(count) => t!("AgentUi.time_hours_ago", count = count).to_string(),
        RelativeTimeUnit::Days(count) => t!("AgentUi.time_days_ago", count = count).to_string(),
        RelativeTimeUnit::Weeks(count) => t!("AgentUi.time_weeks_ago", count = count).to_string(),
    }
}

/// 把 Unix 秒时间戳格式化为相对时间（跟随界面语言）。
pub fn format_timestamp(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs() as i64;

    relative_time_label(relative_time_unit(now.saturating_sub(timestamp)))
}

/// 按查询串过滤会话：对名称与 id 做大小写不敏感的子串匹配。
///
/// 空（或全空白）查询原样返回全部。
pub fn filter_sessions<'a>(summaries: &'a [SessionSummary], query: &str) -> Vec<&'a SessionSummary> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return summaries.iter().collect();
    }
    summaries
        .iter()
        .filter(|summary| {
            summary.name.to_lowercase().contains(&query)
                || summary.id.to_lowercase().contains(&query)
        })
        .collect()
}

/// 一个工作区分组：`root` 为 `None` 表示旧数据 / 未分组。
#[derive(Clone, Debug)]
pub struct WorkspaceGroup<'a> {
    pub root: Option<String>,
    pub sessions: Vec<&'a SessionSummary>,
}

impl WorkspaceGroup<'_> {
    /// 组标题：工作区目录名；未分组用 locale 文案。
    pub fn label(&self) -> SharedString {
        match &self.root {
            Some(root) => std::path::Path::new(root)
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| root.clone())
                .into(),
            None => t!("Workbench.workspace_ungrouped").into(),
        }
    }
}

/// 按会话归属的工作区分组（组内保持输入的近期优先顺序）。
///
/// 组顺序按各组内最近会话时间降序（最近用过的组在前），未分组永远垫底。
pub fn group_sessions_by_workspace<'a>(
    summaries: &[&'a SessionSummary],
) -> Vec<WorkspaceGroup<'a>> {
    let mut groups: Vec<WorkspaceGroup> = Vec::new();
    for summary in summaries {
        match groups
            .iter_mut()
            .find(|group| group.root == summary.workspace_root)
        {
            Some(group) => group.sessions.push(summary),
            None => groups.push(WorkspaceGroup {
                root: summary.workspace_root.clone(),
                sessions: vec![summary],
            }),
        }
    }
    // 组内按最近会话时间降序（不依赖调用方传入顺序）；未分组（root 为
    // None）永远垫底，组间按组内最近会话时间降序。
    for group in &mut groups {
        group
            .sessions
            .sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
    }
    groups.sort_by(|a, b| {
        let a_ungrouped = a.root.is_none();
        let b_ungrouped = b.root.is_none();
        a_ungrouped.cmp(&b_ungrouped).then_with(|| {
            let a_latest = a
                .sessions
                .iter()
                .map(|session| session.updated_at)
                .max()
                .unwrap_or(0);
            let b_latest = b
                .sessions
                .iter()
                .map(|session| session.updated_at)
                .max()
                .unwrap_or(0);
            b_latest.cmp(&a_latest)
        })
    });
    groups
}

#[cfg(test)]
mod relative_time_tests {
    use super::*;

    #[test]
    fn picks_expected_unit_per_bucket() {
        assert_eq!(RelativeTimeUnit::JustNow, relative_time_unit(0));
        assert_eq!(RelativeTimeUnit::JustNow, relative_time_unit(59));
        assert_eq!(RelativeTimeUnit::Minutes(1), relative_time_unit(60));
        assert_eq!(RelativeTimeUnit::Minutes(59), relative_time_unit(3599));
        assert_eq!(RelativeTimeUnit::Hours(1), relative_time_unit(3600));
        assert_eq!(RelativeTimeUnit::Hours(23), relative_time_unit(86_399));
        assert_eq!(RelativeTimeUnit::Days(1), relative_time_unit(86_400));
        assert_eq!(RelativeTimeUnit::Days(6), relative_time_unit(604_799));
        assert_eq!(RelativeTimeUnit::Weeks(1), relative_time_unit(604_800));
        assert_eq!(RelativeTimeUnit::Weeks(4), relative_time_unit(604_800 * 4));
    }

    #[test]
    fn negative_delta_is_treated_as_just_now() {
        assert_eq!(RelativeTimeUnit::JustNow, relative_time_unit(-5));
    }
}

#[cfg(test)]
mod group_tests {
    use super::*;

    #[test]
    fn filter_matches_name_and_id_case_insensitively() {
        let sessions = vec![
            SessionSummary::new("a-1", "重构 Cargo 配置", 100),
            SessionSummary::new("b-2", "Fix login bug", 200),
            SessionSummary::new("c-3", "数据库巡检", 300),
        ];

        assert_eq!(3, filter_sessions(&sessions, "").len());
        assert_eq!(3, filter_sessions(&sessions, "   ").len());
        // 名称匹配（大小写不敏感）。
        assert_eq!(1, filter_sessions(&sessions, "LOGIN").len());
        // id 匹配。
        assert_eq!(1, filter_sessions(&sessions, "a-1").len());
        // CJK。
        assert_eq!(1, filter_sessions(&sessions, "巡检").len());
        assert!(filter_sessions(&sessions, "nope").is_empty());
    }

    #[test]
    fn workspace_groups_order_by_latest_and_put_ungrouped_last() {
        let sessions = vec![
            SessionSummary::new("a", "w2-new", 300).with_workspace_root(Some("/w2".into())),
            SessionSummary::new("b", "w1-old", 100).with_workspace_root(Some("/w1".into())),
            SessionSummary::new("c", "legacy", 200),
            SessionSummary::new("d", "w1-new", 400).with_workspace_root(Some("/w1".into())),
            SessionSummary::new("e", "w2-old", 50).with_workspace_root(Some("/w2".into())),
        ];
        let filtered = filter_sessions(&sessions, "");
        let groups = group_sessions_by_workspace(&filtered);

        let labels: Vec<_> = groups.iter().map(|g| g.root.clone()).collect();
        assert_eq!(
            vec![Some("/w1".into()), Some("/w2".into()), None],
            labels,
            "组间按最近会话降序，未分组垫底"
        );
        assert_eq!(vec!["d", "b"], groups[0]
            .sessions
            .iter()
            .map(|s| s.id.as_str())
            .collect::<Vec<_>>());
        assert_eq!(2, groups[0].sessions.len());
        assert_eq!(2, groups[1].sessions.len());
        assert_eq!(1, groups[2].sessions.len());
    }

    #[test]
    fn workspace_group_label_uses_directory_name() {
        let sessions =
            vec![SessionSummary::new("a", "s", 1).with_workspace_root(Some("/home/u/navop".into()))];
        let groups = group_sessions_by_workspace(&filter_sessions(&sessions, ""));

        assert_eq!("navop", groups[0].label().as_ref());
    }
}
