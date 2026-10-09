//! 用量历史视图。
//!
//! 数据来自 `agent_usage_samples` 流水（见 `persistence::usage_samples_since`）：
//! 快照里的 `context_tokens` 只有「现在用了多少」，画趋势必须靠只追加的采样。
//!
//! 布局分三层，从粗到细：
//! 1. 顶部汇总——窗口内采样次数 / 涉及会话数 / 峰值占用；
//! 2. 按天柱状图——看「最近几天是不是越聊越满」；
//! 3. 会话列表——每条会话最近一次占用 + 自己的迷你趋势。
//!
//! 聚合逻辑（[`session_usage`] / [`daily_usage`] / [`sparkline_bars`]）都是纯函数，
//! 与渲染分开，便于直接断言分桶、排序与归一化。

use std::collections::HashMap;

use gpui::{
    AnyElement, App, Entity, InteractiveElement, IntoElement, MouseButton, ParentElement,
    SharedString, StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
    relative,
};
use gpui_component::Sizable;
use gpui_component::button::{Button, ButtonVariants};
use one_core::llm::usage_history::AgentUsageSample;
use rust_i18n::t;

use crate::agent_view::AgentChatView;
use crate::session_sidebar::{DAY_SECS, local_midnight, now_unix};
use crate::theme::{AgentChatTheme, sp};
use crate::usage::{UsagePressure, format_token_count};

/// 历史窗口长度（天）。
pub const USAGE_HISTORY_DAYS: i64 = 14;

/// 每个会话的迷你趋势最多画多少根柱子。
///
/// 流水本身封顶 200 条，全画出来会挤成一片；取尾部这么多根既能看出走势，
/// 又不会把行撑爆。
pub const SPARKLINE_BARS_MAX: usize = 32;

/// 迷你趋势条的高度。
const SPARKLINE_HEIGHT: f32 = 22.0;
/// 迷你趋势条的最小可见宽度。
const SPARKLINE_BAR_WIDTH: f32 = 3.0;
/// 按天柱状图的高度。
const DAILY_CHART_HEIGHT: f32 = 56.0;

/// 一个会话在窗口内的用量轨迹。
#[derive(Clone, Debug, PartialEq)]
pub struct SessionUsage {
    pub uid: String,
    /// 窗口内的占用采样，时间正序。
    pub samples: Vec<u64>,
    /// 最近一次采样值。
    pub latest: u64,
    /// 最近一次采样时的窗口大小。
    pub window: Option<u64>,
    pub model: Option<String>,
    /// 最近一次采样时间（Unix 秒）。
    pub last_at: i64,
}

impl SessionUsage {
    /// 窗口内峰谷差：看这条会话是「一直在涨」还是「压缩过又回落」。
    pub fn peak(&self) -> u64 {
        self.samples.iter().copied().max().unwrap_or(self.latest)
    }

    pub fn pressure(&self) -> UsagePressure {
        crate::usage::ContextUsage::new(self.latest, self.window).pressure()
    }
}

/// 按天汇总的一根柱子。
#[derive(Clone, Debug, PartialEq)]
pub struct DailyUsage {
    /// 该天本地零点（Unix 秒）。
    pub midnight: i64,
    /// 该天的采样次数。
    pub samples: usize,
    /// 该天的峰值占用。
    pub peak: u64,
    /// 该天涉及多少个会话。
    pub sessions: usize,
}

/// 视图要展示的全部数据。
#[derive(Clone, Debug, Default)]
pub struct UsageHistoryData {
    /// 按最近采样时间倒序的会话轨迹。
    pub sessions: Vec<SessionUsage>,
    /// 按天从旧到新，**空天也保留**（用 0 占位，柱子不会因为没数据而错位）。
    pub daily: Vec<DailyUsage>,
    /// 会话标题（uid → 标题），从侧栏摘要里取。
    pub titles: HashMap<String, SharedString>,
}

impl UsageHistoryData {
    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }

    /// 窗口内采样总次数。
    pub fn sample_count(&self) -> usize {
        self.sessions.iter().map(|usage| usage.samples.len()).sum()
    }

    /// 窗口内峰值占用。
    pub fn peak(&self) -> Option<u64> {
        self.daily
            .iter()
            .map(|day| day.peak)
            .max()
            .filter(|peak| *peak > 0)
    }

    /// 会话标题；没有对应摘要时退回 uid（会话可能刚被删）。
    pub fn title(&self, uid: &str) -> SharedString {
        self.titles
            .get(uid)
            .cloned()
            .unwrap_or_else(|| SharedString::from(uid.to_string()))
    }
}

/// 按会话聚合采样，最近有动静的排前面。
pub fn session_usage(samples: &[AgentUsageSample]) -> Vec<SessionUsage> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: HashMap<String, Vec<&AgentUsageSample>> = HashMap::new();
    for sample in samples {
        let entry = grouped.entry(sample.uid.clone()).or_default();
        if entry.is_empty() {
            order.push(sample.uid.clone());
        }
        entry.push(sample);
    }
    let mut usages: Vec<SessionUsage> = order
        .into_iter()
        .filter_map(|uid| {
            let mut group = grouped.remove(&uid)?;
            group.sort_by_key(|sample| (sample.recorded_at, sample.id.unwrap_or(0)));
            let last = group.last()?;
            Some(SessionUsage {
                uid: uid.clone(),
                samples: group.iter().map(|sample| sample.used).collect(),
                latest: last.used,
                window: last.window,
                model: last.model.clone(),
                last_at: last.recorded_at,
            })
        })
        .collect();
    usages.sort_by(|left, right| {
        right
            .last_at
            .cmp(&left.last_at)
            .then_with(|| left.uid.cmp(&right.uid))
    });
    usages
}

/// 按本地日历天分桶，从 `days - 1` 天前一直排到今天（含空天）。
pub fn daily_usage(samples: &[AgentUsageSample], now: i64, days: i64) -> Vec<DailyUsage> {
    let days = days.max(1);
    let today = local_midnight(now);
    let mut buckets: Vec<DailyUsage> = (0..days)
        .rev()
        .map(|offset| DailyUsage {
            midnight: today - offset * DAY_SECS,
            samples: 0,
            peak: 0,
            sessions: 0,
        })
        .collect();
    let mut seen: HashMap<i64, std::collections::HashSet<&str>> = HashMap::new();
    for sample in samples {
        // 落在窗口外的采样（理论上进不来）直接忽略，不能往未来那根柱子上挂。
        let Some(slot) = buckets.iter_mut().find(|bucket| {
            sample.recorded_at >= bucket.midnight && sample.recorded_at < bucket.midnight + DAY_SECS
        }) else {
            continue;
        };
        slot.samples += 1;
        slot.peak = slot.peak.max(sample.used);
        if seen
            .entry(slot.midnight)
            .or_default()
            .insert(sample.uid.as_str())
        {
            slot.sessions += 1;
        }
    }
    buckets
}

/// 把一串占用值归一化成 0.0–1.0 的柱高，只取尾部至多 `max_bars` 个点。
///
/// 全零（或只有一个点）时给一个最小高度：柱高全是 0 的图看着像「没数据」，
/// 而不是「用得很少」。
pub fn sparkline_bars(values: &[u64], max_bars: usize) -> Vec<f32> {
    if values.is_empty() || max_bars == 0 {
        return Vec::new();
    }
    let tail = &values[values.len().saturating_sub(max_bars)..];
    let peak = tail.iter().copied().max().unwrap_or(0);
    if peak == 0 {
        return vec![MIN_BAR_RATIO; tail.len()];
    }
    tail.iter()
        .map(|value| (*value as f32 / peak as f32).clamp(MIN_BAR_RATIO, 1.0))
        .collect()
}

/// 柱子的最小高度比例：保证「有采样但值极小」也看得见。
const MIN_BAR_RATIO: f32 = 0.12;

/// 采集一次视图数据。
pub fn collect_usage_history(cx: &App) -> UsageHistoryData {
    let now = now_unix();
    let since = local_midnight(now) - (USAGE_HISTORY_DAYS - 1) * DAY_SECS;
    let samples = crate::persistence::usage_samples_since(cx, since);
    let titles = crate::persistence::list_summaries(cx)
        .into_iter()
        .map(|summary| (summary.id, summary.name))
        .collect();
    UsageHistoryData {
        sessions: session_usage(&samples),
        daily: daily_usage(&samples, now, USAGE_HISTORY_DAYS),
        titles,
    }
}

/// 用量历史浮层。
///
/// 覆盖整个面板（与附件预览同一套做法）：数据来自本地库，不需要网络，
/// 所以打开即取、取完即画，不做骨架屏。
pub fn render_usage_history(
    data: &UsageHistoryData,
    theme: &AgentChatTheme,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let backdrop_close = on_close.clone();
    let button_close = on_close;
    let content = div()
        .debug_selector(|| "ai-chat-usage-history".to_string())
        .occlude()
        .w(relative(0.9))
        .max_w(sp(720.0))
        .max_h(relative(0.85))
        .flex()
        .flex_col()
        .rounded(px(12.0))
        .border_1()
        .border_color(theme.border_strong)
        .bg(theme.raised)
        .overflow_hidden()
        .child(header(theme, button_close))
        .child(body(data, theme));

    div()
        .debug_selector(|| "ai-chat-usage-history-layer".to_string())
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(theme.overlay_strong)
        .on_mouse_down(MouseButton::Left, move |_event, window, cx| {
            backdrop_close(window, cx)
        })
        .child(content)
        .into_any_element()
}

fn header(
    theme: &AgentChatTheme,
    on_close: impl Fn(&mut Window, &mut App) + Clone + 'static,
) -> AnyElement {
    let button_close = on_close;
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(sp(12.0))
        .px(sp(16.0))
        .py(sp(12.0))
        .border_b_1()
        .border_color(theme.border)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(sp(2.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.foreground)
                        .child(t!("AgentUi.usage_title").to_string()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(t!("AgentUi.usage_history_range").to_string()),
                ),
        )
        .child(
            Button::new("agent-usage-history-close")
                .ghost()
                .small()
                .label(t!("AgentUi.close").to_string())
                .on_click(move |_event, window, cx| button_close(window, cx)),
        )
        .into_any_element()
}

fn body(data: &UsageHistoryData, theme: &AgentChatTheme) -> AnyElement {
    if data.is_empty() {
        return div()
            .flex()
            .flex_1()
            .items_center()
            .justify_center()
            .p(sp(32.0))
            .child(
                div()
                    .text_sm()
                    .text_color(theme.text_ghost)
                    .child(t!("AgentUi.usage_history_empty").to_string()),
            )
            .into_any_element();
    }
    div()
        .id("agent-usage-history-body")
        .flex()
        .flex_col()
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(summary_row(data, theme))
        .child(daily_chart(data, theme))
        .child(session_list(data, theme))
        .into_any_element()
}

fn summary_row(data: &UsageHistoryData, theme: &AgentChatTheme) -> AnyElement {
    let peak = data
        .peak()
        .map(format_token_count)
        .unwrap_or_else(|| t!("AgentUi.usage_unknown").to_string());
    div()
        .flex()
        .gap(sp(24.0))
        .px(sp(16.0))
        .py(sp(12.0))
        .child(summary_item(
            t!("AgentUi.usage_history_sessions").to_string(),
            data.sessions.len().to_string(),
            theme,
        ))
        .child(summary_item(
            t!("AgentUi.usage_history_samples").to_string(),
            data.sample_count().to_string(),
            theme,
        ))
        .child(summary_item(
            t!("AgentUi.usage_history_peak").to_string(),
            peak,
            theme,
        ))
        .into_any_element()
}

fn summary_item(label: String, value: String, theme: &AgentChatTheme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(sp(2.0))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(label),
        )
        .child(div().text_sm().text_color(theme.foreground).child(value))
        .into_any_element()
}

/// 按天柱状图：柱子高度按窗口内峰值归一化。
fn daily_chart(data: &UsageHistoryData, theme: &AgentChatTheme) -> AnyElement {
    let values: Vec<u64> = data.daily.iter().map(|day| day.peak).collect();
    let ratios = sparkline_bars(&values, values.len());
    let peak = values.iter().copied().max().unwrap_or(0);
    div()
        .flex()
        .flex_col()
        .gap(sp(6.0))
        .px(sp(16.0))
        .pb(sp(12.0))
        .child(
            div()
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.usage_history_daily").to_string()),
        )
        .child(
            div()
                .flex()
                .items_end()
                .gap(sp(4.0))
                .h(px(DAILY_CHART_HEIGHT))
                .when(peak == 0, |chart| {
                    chart.child(
                        div()
                            .text_xs()
                            .text_color(theme.text_ghost)
                            .child(t!("AgentUi.usage_history_empty_short").to_string()),
                    )
                })
                .children(
                    ratios
                        .into_iter()
                        .map(|ratio| {
                            div()
                                .flex_1()
                                .h(relative(ratio))
                                .min_h(px(2.0))
                                .rounded_t(px(3.0))
                                .bg(theme.chart_bullish)
                        })
                        .collect::<Vec<_>>(),
                ),
        )
        .child(
            div()
                .flex()
                .justify_between()
                .text_xs()
                .text_color(theme.text_ghost)
                .child(format_day(data.daily.first().map(|day| day.midnight)))
                .child(t!("AgentUi.date_today").to_string()),
        )
        .into_any_element()
}

fn session_list(data: &UsageHistoryData, theme: &AgentChatTheme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .border_t_1()
        .border_color(theme.border)
        .child(
            div()
                .px(sp(16.0))
                .py(sp(8.0))
                .text_xs()
                .text_color(theme.muted_foreground)
                .child(t!("AgentUi.usage_history_per_session").to_string()),
        )
        .children(
            data.sessions
                .iter()
                .map(|usage| session_row(usage, data, theme))
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

fn session_row(
    usage: &SessionUsage,
    data: &UsageHistoryData,
    theme: &AgentChatTheme,
) -> AnyElement {
    let accent = match usage.pressure() {
        UsagePressure::Calm => theme.gauge,
        UsagePressure::Elevated => theme.gauge_warning,
        UsagePressure::Critical => theme.gauge_danger,
    };
    let detail = match usage.window.filter(|window| *window > 0) {
        Some(window) => format!(
            "{}/{} tokens",
            format_token_count(usage.latest),
            format_token_count(window)
        ),
        None => format!("{} tokens", format_token_count(usage.latest)),
    };
    div()
        .flex()
        .items_center()
        .gap(sp(12.0))
        .px(sp(16.0))
        .py(sp(8.0))
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_w_0()
                .gap(sp(2.0))
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.foreground)
                        .overflow_hidden()
                        .whitespace_nowrap()
                        .child(data.title(&usage.uid)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.muted_foreground)
                        .child(subtitle(usage, &detail)),
                ),
        )
        .child(sparkline(usage, accent, theme))
        .into_any_element()
}

/// 行的第二行文案：`模型 · 当前读数`，被压缩过（峰值高于当前值）再缀上峰值。
///
/// 峰值只在「确实掉下来过」时才写：一直单调上涨那是废话，多了噪音。
fn subtitle(usage: &SessionUsage, detail: &str) -> String {
    let mut text = match usage.model.as_deref() {
        Some(model) => format!("{model} · {detail}"),
        None => detail.to_string(),
    };
    if usage.peak() > usage.latest {
        text.push_str(&format!(
            " · {} {}",
            t!("AgentUi.usage_history_peak"),
            format_token_count(usage.peak())
        ));
    }
    text
}

/// 一条会话的迷你趋势：柱高按该会话窗口内峰值归一化。
fn sparkline(usage: &SessionUsage, accent: gpui::Hsla, theme: &AgentChatTheme) -> AnyElement {
    let bars = sparkline_bars(&usage.samples, SPARKLINE_BARS_MAX);
    div()
        .flex_shrink_0()
        .w(sp(120.0))
        .h(px(SPARKLINE_HEIGHT))
        .flex()
        .items_end()
        .justify_end()
        .gap(px(2.0))
        .when(bars.is_empty(), |row| {
            row.child(div().w_full().h(px(1.0)).bg(theme.chart_grid))
        })
        .children(
            bars.into_iter()
                .map(|ratio| {
                    div()
                        .w(px(SPARKLINE_BAR_WIDTH))
                        .h(relative(ratio))
                        .min_h(px(2.0))
                        .rounded_t(px(2.0))
                        .bg(accent)
                })
                .collect::<Vec<_>>(),
        )
        .into_any_element()
}

/// 把某天的本地零点显示成 `MM-DD`。
fn format_day(midnight: Option<i64>) -> String {
    use chrono::{Local, TimeZone as _};

    let Some(midnight) = midnight else {
        return String::new();
    };
    match Local.timestamp_opt(midnight, 0).single() {
        Some(datetime) => datetime.format("%m-%d").to_string(),
        None => String::new(),
    }
}

impl AgentChatView {
    /// 用量历史浮层；浮层关着时返回 `None`。
    ///
    /// 数据在打开时取好放在 `self.usage_history` 里（见 `on_input_event` 里
    /// `OpenUsageHistory` 的分支），这里只负责画，不再查库。
    pub(super) fn render_usage_history(
        &self,
        theme: &AgentChatTheme,
        view: Entity<Self>,
    ) -> Option<AnyElement> {
        let data = self.usage_history.as_ref()?;
        // 关闭回调要能反查本视图：拿弱引用，免得覆盖层把面板钉在内存里。
        let view = view.downgrade();
        let on_close = move |_window: &mut Window, cx: &mut App| {
            let _ = view.update(cx, |this, cx| this.close_usage_history(cx));
        };
        Some(render_usage_history(data, theme, on_close))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(uid: &str, used: u64, at: i64) -> AgentUsageSample {
        let mut sample = AgentUsageSample::new(uid, used, at);
        sample.window = Some(200_000);
        sample.model = Some("claude-sonnet-4-5".to_string());
        sample
    }

    #[test]
    fn session_usage_groups_by_uid_and_sorts_by_recency() {
        let samples = vec![
            sample("a", 10, 100),
            sample("b", 20, 200),
            sample("a", 30, 300),
        ];
        let usages = session_usage(&samples);
        assert_eq!(
            vec!["a", "b"],
            usages.iter().map(|u| u.uid.as_str()).collect::<Vec<_>>()
        );
        assert_eq!(vec![10, 30], usages[0].samples, "组内按时间正序");
        assert_eq!(30, usages[0].latest);
        assert_eq!(300, usages[0].last_at);
        assert_eq!(30, usages[0].peak());
    }

    #[test]
    fn session_usage_of_empty_input_is_empty() {
        assert!(session_usage(&[]).is_empty());
    }

    #[test]
    fn daily_usage_keeps_empty_days_and_buckets_by_local_calendar_day() {
        let now = local_midnight(1_700_000_000) + DAY_SECS / 2;
        let today = local_midnight(now);
        // 今天两条（不同会话）、昨天一条、更早一条（落在窗口外）。
        let samples = vec![
            sample("a", 10, today + 60),
            sample("a", 40, today + 120),
            sample("b", 20, today + 180),
            sample("a", 5, today - 60),
            sample("a", 7, today - 30 * DAY_SECS),
        ];
        let days = daily_usage(&samples, now, 3);
        assert_eq!(3, days.len(), "3 天窗口就该给 3 根柱子");
        assert_eq!(
            vec![today - 2 * DAY_SECS, today - DAY_SECS, today],
            days.iter().map(|day| day.midnight).collect::<Vec<_>>(),
            "从旧到新连续排，空天也保留"
        );
        assert_eq!((0, 0, 0), (days[0].samples, days[0].peak, days[0].sessions));
        assert_eq!((1, 5, 1), (days[1].samples, days[1].peak, days[1].sessions));
        assert_eq!(
            (3, 40, 2),
            (days[2].samples, days[2].peak, days[2].sessions)
        );
    }

    #[test]
    fn daily_usage_ignores_samples_outside_the_window() {
        let now = local_midnight(1_700_000_000) + DAY_SECS / 2;
        let samples = vec![sample("a", 10, now + 5 * DAY_SECS)];
        let days = daily_usage(&samples, now, 3);
        assert!(
            days.iter().all(|day| day.samples == 0),
            "未来的采样不该被塞进最后一根柱子"
        );
    }

    #[test]
    fn subtitle_mentions_the_peak_only_after_the_context_dropped() {
        let now = 1_700_000_000;
        let rising = session_usage(&[
            AgentUsageSample::new("sess_a", 8_000, now - 60),
            AgentUsageSample::new("sess_a", 12_000, now),
        ]);
        let rising = &rising[0];
        assert!(
            !subtitle(rising, "12.0k").contains(t!("AgentUi.usage_history_peak").as_ref()),
            "一路涨上去不必报峰值"
        );

        let compacted = session_usage(&[
            AgentUsageSample::new("sess_a", 120_000, now - 60),
            AgentUsageSample::new("sess_a", 12_000, now),
        ]);
        assert!(
            subtitle(&compacted[0], "12.0k").contains(t!("AgentUi.usage_history_peak").as_ref()),
            "被压缩过就该把峰值露出来"
        );
    }

    #[test]
    fn sparkline_keeps_only_the_tail_and_normalizes_by_peak() {
        let values = vec![1, 2, 3, 4, 10];
        let bars = sparkline_bars(&values, 3);
        assert_eq!(3, bars.len(), "只取尾部 max_bars 个点");
        assert!((bars[2] - 1.0).abs() < 1e-6, "峰值归一化成满高");
        assert!(bars[0] < bars[2]);
    }

    #[test]
    fn sparkline_gives_flat_series_a_visible_height() {
        let bars = sparkline_bars(&[0, 0, 0], 3);
        assert_eq!(3, bars.len());
        assert!(
            bars.iter().all(|bar| *bar >= MIN_BAR_RATIO),
            "全零也该看得见，而不是画成空白"
        );
    }

    #[test]
    fn sparkline_of_empty_input_is_empty() {
        assert!(sparkline_bars(&[], 10).is_empty());
        assert!(sparkline_bars(&[1, 2], 0).is_empty());
    }
}
