//! 通用会话侧边栏:数据结构 + 纯视觉渲染辅助。
//!
//! 与具体业务的会话存储完全解耦:调用方把自己的会话映射成 [`SessionSummary`] 即可。
//! 折叠 / 展开、点击交互等编排由上层视图(`ChatView`)负责,本模块只提供「长什么样」。

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Div, FontWeight, Hsla, InteractiveElement, ParentElement, SharedString, Styled, div,
};
use gpui_component::{ActiveTheme, h_flex, v_flex};
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

/// 把 Unix 秒时间戳格式化为相对时间(简体中文)。
pub fn format_timestamp(timestamp: i64) -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::from_secs(0))
        .as_secs() as i64;

    let diff = now.saturating_sub(timestamp);

    if diff < 60 {
        "刚刚".to_string()
    } else if diff < 3600 {
        format!("{} 分钟前", diff / 60)
    } else if diff < 86400 {
        format!("{} 小时前", diff / 3600)
    } else if diff < 604800 {
        format!("{} 天前", diff / 86400)
    } else {
        format!("{} 周前", diff / 604800)
    }
}
