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
