//! 通用会话侧边栏:数据结构 + 纯视觉渲染辅助。
//!
//! 与具体业务的会话存储完全解耦:调用方把自己的会话映射成 [`SessionSummary`] 即可。
//! 折叠 / 展开、点击交互等编排由上层视图(`ChatView`)负责,本模块只提供「长什么样」。

use chrono::Datelike as _;
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
        }
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
                        .child(format_timestamp(session.updated_at)),
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

/// 会话按最后更新时间的分组（今天/昨天/本周/本月/今年/更早）。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SessionGroup {
    Today,
    Yesterday,
    ThisWeek,
    ThisMonth,
    ThisYear,
    Earlier,
}

impl SessionGroup {
    /// 本地化组名，供侧栏小节标题使用。
    pub fn label(self) -> String {
        match self {
            Self::Today => t!("Workbench.group_today").to_string(),
            Self::Yesterday => t!("Workbench.group_yesterday").to_string(),
            Self::ThisWeek => t!("Workbench.group_this_week").to_string(),
            Self::ThisMonth => t!("Workbench.group_this_month").to_string(),
            Self::ThisYear => t!("Workbench.group_this_year").to_string(),
            Self::Earlier => t!("Workbench.group_earlier").to_string(),
        }
    }
}

/// 把 Unix 秒时间戳换成 `offset` 时区的本地日期。
///
/// 非法时间戳（超出 chrono 表达范围）回落到 `NaiveDate::MIN`，会被归入「更早」。
fn local_date(timestamp: i64, offset: chrono::FixedOffset) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp(timestamp, 0)
        .map(|utc| utc.with_timezone(&offset).date_naive())
        .unwrap_or(chrono::NaiveDate::MIN)
}

/// 本地日所在周的周一。
fn week_start(date: chrono::NaiveDate) -> chrono::NaiveDate {
    date - chrono::Days::new(date.weekday().num_days_from_monday() as u64)
}

/// 以显式传入的 `now_local` 为基准归组，测试可注入确定时间。
///
/// 归组语义按本地日历日（而非固定秒数窗口）：
/// 今天 → 昨天 → 本周（≥ 本周一，且不是今昨两天）→ 本月 → 今年 → 更早。
/// 落在未来的时间戳（时钟偏差）视作今天，避免凭空多出一个「未来」组。
pub fn session_group_at(timestamp: i64, now_local: chrono::DateTime<chrono::FixedOffset>) -> SessionGroup {
    let now_date = now_local.date_naive();
    let date = local_date(timestamp, *now_local.offset());
    if date >= now_date {
        SessionGroup::Today
    } else if date == now_date - chrono::Days::new(1) {
        SessionGroup::Yesterday
    } else if date >= week_start(now_date) {
        SessionGroup::ThisWeek
    } else if date.year() == now_date.year() && date.month() == now_date.month() {
        SessionGroup::ThisMonth
    } else if date.year() == now_date.year() {
        SessionGroup::ThisYear
    } else {
        SessionGroup::Earlier
    }
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

/// 把（假定按更新时间降序的）会话聚成有序小节。
///
/// 输入顺序不被重排；乱序输入只会让同一组出现多段小节，渲染层无需特判。
pub fn group_sessions<'a>(
    summaries: &[&'a SessionSummary],
    now_local: chrono::DateTime<chrono::FixedOffset>,
) -> Vec<(SessionGroup, Vec<&'a SessionSummary>)> {
    let mut sections: Vec<(SessionGroup, Vec<&SessionSummary>)> = Vec::new();
    for summary in summaries {
        let group = session_group_at(summary.updated_at, now_local);
        match sections.last_mut() {
            Some((last_group, items)) if *last_group == group => items.push(summary),
            _ => sections.push((group, vec![summary])),
        }
    }
    sections
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
    use chrono::{FixedOffset, TimeZone};

    const TZ: i32 = 8 * 3600;

    fn at(y: i32, m: u32, d: u32, h: u32) -> chrono::DateTime<FixedOffset> {
        FixedOffset::east_opt(TZ)
            .unwrap()
            .with_ymd_and_hms(y, m, d, h, 0, 0)
            .single()
            .unwrap()
    }

    fn ts(y: i32, m: u32, d: u32) -> i64 {
        at(y, m, d, 12).timestamp()
    }

    #[test]
    fn buckets_follow_the_local_calendar() {
        // 2026-09-24 14:00 是周四；本周一是 09-21。
        let now = at(2026, 9, 24, 14);

        assert_eq!(SessionGroup::Today, session_group_at(ts(2026, 9, 24), now));
        assert_eq!(
            SessionGroup::Yesterday,
            session_group_at(ts(2026, 9, 23), now)
        );
        // 本周二：还没被今天/昨天收走，归本周。
        assert_eq!(SessionGroup::ThisWeek, session_group_at(ts(2026, 9, 22), now));
        // 上周日：同月但已出本周。
        assert_eq!(SessionGroup::ThisMonth, session_group_at(ts(2026, 9, 20), now));
        assert_eq!(SessionGroup::ThisYear, session_group_at(ts(2026, 8, 15), now));
        assert_eq!(SessionGroup::Earlier, session_group_at(ts(2025, 6, 1), now));
    }

    #[test]
    fn week_boundary_keeps_last_sunday_as_yesterday() {
        // 今天是周一：昨天（上周日）先于「本周」判定命中。
        let now = at(2026, 9, 21, 10);

        assert_eq!(
            SessionGroup::Yesterday,
            session_group_at(ts(2026, 9, 20), now)
        );
    }

    #[test]
    fn future_timestamps_and_garbage_fall_back_safely() {
        let now = at(2026, 9, 24, 14);

        // 时钟偏差：明天的会话视作今天，而不是冒出新组。
        assert_eq!(SessionGroup::Today, session_group_at(ts(2026, 9, 25), now));
        // 非法时间戳（早于 chrono 下限的极端值）归更早。
        assert_eq!(
            SessionGroup::Earlier,
            session_group_at(i64::MIN, now)
        );
    }

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
    fn group_sessions_keeps_input_order_and_merges_adjacent_groups() {
        let now = at(2026, 9, 24, 14);
        let sessions = vec![
            SessionSummary::new("a", "today-2", ts(2026, 9, 24)),
            SessionSummary::new("b", "today-1", ts(2026, 9, 24)),
            SessionSummary::new("c", "yesterday", ts(2026, 9, 23)),
            SessionSummary::new("d", "earlier", ts(2025, 1, 1)),
        ];
        let filtered = filter_sessions(&sessions, "");
        let sections = group_sessions(&filtered, now);

        assert_eq!(
            vec![
                (SessionGroup::Today, 2),
                (SessionGroup::Yesterday, 1),
                (SessionGroup::Earlier, 1),
            ],
            sections
                .iter()
                .map(|(group, items)| (*group, items.len()))
                .collect::<Vec<_>>()
        );
        // 降序输入下组内顺序保持不变。
        assert_eq!("today-2", sections[0].1[0].name.as_ref());
    }
}
