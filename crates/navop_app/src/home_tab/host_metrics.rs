//! 首页连接卡片的远端资源占用（#279）。
//!
//! 用一次性的后台 SSH 会话采集 CPU、内存与磁盘占用，不打开终端页签；
//! 结果按连接缓存，过期后再采集。任何失败都只记 debug 日志，不影响首页交互。

use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};
use gpui_component::{
    ActiveTheme, Sizable, Size, h_flex, progress::Progress, tooltip::Tooltip, v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection,
};
use rust_i18n::t;

use super::HomePage;

/// 同一连接资源快照的缓存时间。
const HOST_METRICS_TTL: Duration = Duration::from_secs(300);
/// 定时检查过期快照的间隔；实际采集仍受 [`HOST_METRICS_TTL`] 控制。
const HOST_METRICS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// 同时进行的后台采集上限：首页一次可能渲染几十个连接，不能同时拉起几十个 SSH 会话。
const HOST_METRICS_MAX_IN_FLIGHT: usize = 2;

pub(crate) struct HostMetricsEntry {
    usage: terminal_view::HostResourceUsage,
    sampled_at: Instant,
}

enum HostMetricsState {
    Ready(terminal_view::HostResourceUsage),
    Pending,
    Unavailable,
}

#[derive(Default)]
pub(crate) struct HostMetricsCache {
    entries: HashMap<i64, HostMetricsEntry>,
    in_flight: HashSet<i64>,
    /// 最后一次尝试时间，包含凭据解析或连接失败，避免失败连接每分钟重试。
    last_attempt: HashMap<i64, Instant>,
}

impl HomePage {
    /// 在首页生命周期内周期检查过期快照；关闭设置时也会在这里恢复采集。
    pub(crate) fn start_host_metrics_refresh(&self, cx: &mut Context<Self>) {
        cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(HOST_METRICS_REFRESH_INTERVAL)
                    .await;
                if this
                    .update(cx, |this, cx| this.refresh_host_metrics(cx))
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();
    }

    /// 为首页的 SSH 连接排队后台采集（设置关闭或主密钥未解锁时直接跳过）。
    pub(super) fn refresh_host_metrics(&mut self, cx: &mut Context<Self>) {
        if !host_metrics_enabled(cx) || self.saved_connections_locked() {
            return;
        }

        // 每个任务完成后都会再次调用本方法，把超出并发上限的连接继续排空。
        while self.host_metrics.in_flight.len() < HOST_METRICS_MAX_IN_FLIGHT {
            let Some((connection_id, connection)) = self.next_host_metrics_candidate() else {
                break;
            };
            self.host_metrics
                .last_attempt
                .insert(connection_id, Instant::now());

            let Some(repository) = cx
                .global::<GlobalStorageState>()
                .storage
                .get::<ConnectionRepository>()
            else {
                tracing::debug!("采集远端资源占用失败：ConnectionRepository 不可用");
                break;
            };
            let connection = match repository.resolve_runtime_connection(&connection) {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::debug!(
                        connection_id,
                        %error,
                        "采集远端资源占用失败：无法解析连接凭据"
                    );
                    continue;
                }
            };
            let config = match terminal::terminal::ssh_terminal_config_from_connection(&connection)
            {
                Ok(config) => config,
                Err(error) => {
                    tracing::debug!(
                        connection_id,
                        %error,
                        "采集远端资源占用失败：无法构造 SSH 配置"
                    );
                    continue;
                }
            };

            self.host_metrics.in_flight.insert(connection_id);
            // SSH 建连依赖 Tokio 运行时（crates/ssh/src/ssh.rs 里的 tokio::time::timeout 等），
            // 必须走应用 Tokio runtime；cx.background_spawn 跑在 GPUI 自己的线程池上，
            // 没有 reactor，一旦开始建连就会 panic：there is no reactor running。
            let task = Tokio::spawn_result(cx, async move {
                terminal_view::probe_host_resource_usage(config.ssh_config).await
            });
            cx.spawn(async move |this, cx| {
                let usage = task.await;
                _ = this.update(cx, |this, cx| {
                    this.host_metrics.in_flight.remove(&connection_id);
                    match usage {
                        Ok(usage) if host_metrics_enabled(cx) => {
                            this.host_metrics.entries.insert(
                                connection_id,
                                HostMetricsEntry {
                                    usage,
                                    sampled_at: Instant::now(),
                                },
                            );
                            this.host_metrics.last_attempt.remove(&connection_id);
                        }
                        Ok(_) => {}
                        Err(error) => {
                            tracing::debug!(connection_id, %error, "采集远端资源占用失败");
                        }
                    }
                    this.refresh_host_metrics(cx);
                    cx.notify();
                });
            })
            .detach();
        }
    }

    fn next_host_metrics_candidate(&self) -> Option<(i64, one_core::storage::StoredConnection)> {
        self.connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .find_map(|connection| {
                let connection_id = connection.id?;
                if self.host_metrics.in_flight.contains(&connection_id) {
                    return None;
                }
                if self
                    .host_metrics
                    .entries
                    .get(&connection_id)
                    .is_some_and(|entry| entry.sampled_at.elapsed() < HOST_METRICS_TTL)
                {
                    return None;
                }
                if self
                    .host_metrics
                    .last_attempt
                    .get(&connection_id)
                    .is_some_and(|attempted_at| attempted_at.elapsed() < HOST_METRICS_TTL)
                {
                    return None;
                }
                if let Ok(params) = connection.to_ssh_params()
                    && (params.prompts_for_username()
                        || (params.prompts_for_password()
                            && params.auth_method.contains_password()))
                {
                    return None;
                }
                Some((connection_id, connection.clone()))
            })
    }

    /// 卡片、列表共用的资源标签（没有缓存快照或设置关闭时返回 None）。
    pub(crate) fn render_host_metrics_detail(
        &self,
        connection: &StoredConnection,
        id_prefix: &str,
        cx: &App,
    ) -> Option<AnyElement> {
        match self.host_metrics_state(connection, cx)? {
            HostMetricsState::Ready(usage) => {
                Some(render_host_metrics_detail(&usage, id_prefix, cx))
            }
            HostMetricsState::Pending => Some(render_host_metrics_status(
                id_prefix,
                t!("Home.host_metrics_collecting").to_string(),
                cx,
            )),
            HostMetricsState::Unavailable => Some(render_host_metrics_status(
                id_prefix,
                t!("Home.host_metrics_unavailable").to_string(),
                cx,
            )),
        }
    }

    /// 卡片宽度有限，只显示一行三项占用；系统和容量信息通过 tooltip 展开。
    pub(crate) fn render_host_metrics_card(
        &self,
        connection: &StoredConnection,
        id_prefix: &str,
        cx: &App,
    ) -> Option<AnyElement> {
        match self.host_metrics_state(connection, cx)? {
            HostMetricsState::Ready(usage) => Some(render_host_metrics_card(&usage, id_prefix, cx)),
            HostMetricsState::Pending => Some(render_host_metrics_card_status(
                id_prefix,
                t!("Home.host_metrics_collecting").to_string(),
                cx,
            )),
            HostMetricsState::Unavailable => Some(render_host_metrics_card_status(
                id_prefix,
                t!("Home.host_metrics_unavailable").to_string(),
                cx,
            )),
        }
    }

    /// 树行空间有限，用三条迷你进度条表示 CPU / 内存 / 磁盘，完整数值放 tooltip。
    pub(crate) fn render_host_metrics_compact(
        &self,
        connection: &StoredConnection,
        id_prefix: &str,
        cx: &App,
    ) -> Option<AnyElement> {
        match self.host_metrics_state(connection, cx)? {
            HostMetricsState::Ready(usage) => {
                Some(render_host_metrics_compact(&usage, id_prefix, cx))
            }
            HostMetricsState::Pending | HostMetricsState::Unavailable => None,
        }
    }

    pub(crate) fn shows_host_metrics_strip(&self, connection: &StoredConnection, cx: &App) -> bool {
        host_metrics_enabled(cx) && connection.connection_type == ConnectionType::SshSftp
    }

    fn host_metrics_state(
        &self,
        connection: &StoredConnection,
        cx: &App,
    ) -> Option<HostMetricsState> {
        if !self.shows_host_metrics_strip(connection, cx) {
            return None;
        }
        let connection_id = connection.id?;
        if let Some(entry) = self.host_metrics.entries.get(&connection_id) {
            return Some(HostMetricsState::Ready(entry.usage.clone()));
        }
        if self.host_metrics.in_flight.contains(&connection_id) {
            return Some(HostMetricsState::Pending);
        }
        if self.host_metrics.last_attempt.contains_key(&connection_id) {
            return Some(HostMetricsState::Unavailable);
        }
        Some(HostMetricsState::Pending)
    }
}

fn host_metrics_enabled(cx: &App) -> bool {
    AppSettings::global(cx).home_show_host_metrics
}

fn format_host_metrics_summary(usage: &terminal_view::HostResourceUsage) -> Option<String> {
    let mut parts = Vec::new();
    if let Some(cpu) = usage.cpu_percent {
        let mut text = t!("Home.host_metrics_cpu", percent = format!("{cpu:.0}")).to_string();
        if let Some(cores) = usage.cpu_cores {
            text.push_str(" · ");
            text.push_str(&t!("Home.host_metrics_cores", count = cores.to_string()).to_string());
        }
        parts.push(text);
    }
    if let Some(memory) = usage.memory_percent {
        let mut text = t!("Home.host_metrics_memory", percent = format!("{memory:.0}")).to_string();
        if let Some(total) = usage.memory_total_kib {
            text.push_str(" · ");
            text.push_str(&format_kib_compact(total));
        }
        parts.push(text);
    }
    if let Some(disk) = usage.disk_percent {
        let mut text = t!("Home.host_metrics_disk", percent = format!("{disk:.0}")).to_string();
        if let Some(total) = usage.disk_total_kib {
            text.push_str(" · ");
            text.push_str(&format_kib_compact(total));
        }
        parts.push(text);
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn render_host_metrics_detail(
    usage: &terminal_view::HostResourceUsage,
    id_prefix: &str,
    cx: &App,
) -> AnyElement {
    let metrics = host_metric_values(usage);
    h_flex()
        .min_w_0()
        .flex_wrap()
        .gap_1p5()
        .when_some(usage.os_name.as_ref(), |row, os_name| {
            row.child(render_host_info_chip(
                SharedString::from(format!("{id_prefix}-os")),
                os_name.clone(),
                cx,
            ))
        })
        .children(metrics.into_iter().map(|(kind, value)| {
            render_host_metric_chip(
                SharedString::from(format!("{id_prefix}-{}", kind.id_suffix())),
                kind.label(),
                value,
                host_metric_detail(usage, kind),
                cx,
            )
        }))
        .into_any_element()
}

fn render_host_metrics_status(id_prefix: &str, text: String, cx: &App) -> AnyElement {
    h_flex()
        .id(SharedString::from(format!("{id_prefix}-status")))
        .flex_shrink_0()
        .items_center()
        .gap_1p5()
        .px_2()
        .py_0p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .into_any_element()
}

fn render_host_metrics_card_status(id_prefix: &str, text: String, cx: &App) -> AnyElement {
    h_flex()
        .id(SharedString::from(format!("{id_prefix}-card-status")))
        .w_full()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .into_any_element()
}

fn render_host_metrics_card(
    usage: &terminal_view::HostResourceUsage,
    id_prefix: &str,
    cx: &App,
) -> AnyElement {
    let metrics = host_metric_values(usage);
    let tooltip: SharedString = format_host_metrics_summary(usage)
        .unwrap_or_default()
        .into();
    h_flex()
        .id(SharedString::from(format!("{id_prefix}-card")))
        .w_full()
        .min_w_0()
        .items_center()
        .gap_2()
        .px_2()
        .py_1()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .children(metrics.into_iter().map(|(kind, value)| {
            h_flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap_1()
                .child(
                    div()
                        .size_1p5()
                        .flex_shrink_0()
                        .rounded_full()
                        .bg(metric_color(value, cx)),
                )
                .child(
                    div()
                        .min_w_0()
                        .overflow_hidden()
                        .text_ellipsis()
                        .whitespace_nowrap()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(kind.label()),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .text_color(if value >= 75.0 {
                            metric_color(value, cx)
                        } else {
                            cx.theme().foreground
                        })
                        .child(format!("{value:.0}%")),
                )
        }))
        .into_any_element()
}

fn render_host_info_chip(id: SharedString, text: String, cx: &App) -> AnyElement {
    let tooltip: SharedString = text.clone().into();
    h_flex()
        .id(id)
        .flex_shrink_0()
        .max_w(gpui::rems(12.0))
        .px_2()
        .py_0p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            div()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(text),
        )
        .into_any_element()
}

fn render_host_metric_chip(
    id: SharedString,
    label: String,
    value: f64,
    detail: Option<String>,
    cx: &App,
) -> AnyElement {
    let tone = metric_color(value, cx);
    h_flex()
        .id(id)
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .px_2()
        .py_0p5()
        .rounded(cx.theme().radius)
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .child(div().size_1p5().flex_shrink_0().rounded_full().bg(tone))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if value >= 75.0 {
                    tone
                } else {
                    cx.theme().foreground
                })
                .child(format!("{value:.0}%")),
        )
        .when_some(detail, |chip, detail| {
            chip.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("· {detail}")),
            )
        })
        .into_any_element()
}

fn host_metric_detail(
    usage: &terminal_view::HostResourceUsage,
    kind: HostMetricKind,
) -> Option<String> {
    match kind {
        HostMetricKind::Cpu => usage
            .cpu_cores
            .map(|cores| t!("Home.host_metrics_cores", count = cores.to_string()).to_string()),
        HostMetricKind::Memory => usage.memory_total_kib.map(format_kib_compact),
        HostMetricKind::Disk => usage.disk_total_kib.map(format_kib_compact),
    }
}

fn format_kib_compact(kib: u64) -> String {
    let gib = kib as f64 / 1024.0 / 1024.0;
    if gib >= 1024.0 {
        format!("{:.1}T", gib / 1024.0)
    } else if gib >= 100.0 || (gib >= 10.0 && nearly_whole(gib)) {
        format!("{gib:.0}G")
    } else if gib >= 10.0 {
        format!("{gib:.1}G")
    } else if gib >= 1.0 {
        format!("{gib:.2}G")
    } else {
        let mib = kib as f64 / 1024.0;
        if mib >= 1.0 {
            format!("{mib:.0}M")
        } else {
            format!("{kib}K")
        }
    }
}

fn nearly_whole(value: f64) -> bool {
    (value - value.round()).abs() < 0.05
}

fn render_host_metrics_compact(
    usage: &terminal_view::HostResourceUsage,
    id_prefix: &str,
    cx: &App,
) -> AnyElement {
    let metrics = host_metric_values(usage);
    let tooltip: SharedString = format_host_metrics_summary(usage)
        .unwrap_or_default()
        .into();
    h_flex()
        .id(SharedString::from(format!("{id_prefix}-compact")))
        .flex_shrink_0()
        .items_center()
        .justify_center()
        .px_1p5()
        .py_1()
        .rounded_full()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().muted)
        .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
        .child(
            v_flex()
                .w_5()
                .gap_0p5()
                .children(metrics.into_iter().map(|(kind, value)| {
                    Progress::new(SharedString::from(format!(
                        "{id_prefix}-compact-{}",
                        kind.id_suffix()
                    )))
                    .with_size(Size::XSmall)
                    .color(metric_color(value, cx))
                    .value(value.clamp(0.0, 100.0) as f32)
                    .accessibility_label(format!("{} {value:.0}%", kind.label()))
                })),
        )
        .into_any_element()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HostMetricKind {
    Cpu,
    Memory,
    Disk,
}

impl HostMetricKind {
    fn id_suffix(self) -> &'static str {
        match self {
            Self::Cpu => "cpu",
            Self::Memory => "memory",
            Self::Disk => "disk",
        }
    }

    fn label(self) -> String {
        match self {
            Self::Cpu => t!("Home.host_metrics_cpu_label").to_string(),
            Self::Memory => t!("Home.host_metrics_memory_label").to_string(),
            Self::Disk => t!("Home.host_metrics_disk_label").to_string(),
        }
    }
}

fn host_metric_values(usage: &terminal_view::HostResourceUsage) -> Vec<(HostMetricKind, f64)> {
    let parts = [usage.cpu_percent, usage.memory_percent, usage.disk_percent].into_iter();
    [
        HostMetricKind::Cpu,
        HostMetricKind::Memory,
        HostMetricKind::Disk,
    ]
    .into_iter()
    .zip(parts)
    .filter_map(|(kind, value)| value.map(|value| (kind, value)))
    .collect()
}

fn metric_color(value: f64, cx: &App) -> Hsla {
    if value >= 90.0 {
        cx.theme().danger
    } else if value >= 75.0 {
        cx.theme().warning
    } else {
        cx.theme().primary
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn metric_values_skip_missing_metrics_and_keep_column_order() {
        assert_eq!(
            host_metric_values(&terminal_view::HostResourceUsage {
                os_name: None,
                cpu_cores: None,
                cpu_percent: Some(12.4),
                memory_total_kib: None,
                memory_percent: None,
                disk_total_kib: None,
                disk_percent: Some(51.6),
            })
            .iter()
            .map(|(kind, value)| (kind.id_suffix(), *value))
            .collect::<Vec<_>>(),
            vec![("cpu", 12.4), ("disk", 51.6)]
        );
        assert_eq!(
            host_metric_values(&terminal_view::HostResourceUsage {
                os_name: None,
                cpu_cores: None,
                cpu_percent: None,
                memory_total_kib: None,
                memory_percent: None,
                disk_total_kib: None,
                disk_percent: None,
            }),
            Vec::new()
        );
    }

    #[test]
    fn compact_capacity_uses_reference_style_units() {
        assert_eq!(format_kib_compact(27 * 1024 * 1024), "27G");
        assert_eq!(format_kib_compact(247 * 1024 * 1024), "247G");
        assert_eq!(format_kib_compact(512 * 1024), "512M");
    }
}
