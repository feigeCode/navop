//! 首页连接卡片的远端资源占用（#279）。
//!
//! 用一次性的后台 SSH 会话采集 CPU、内存、磁盘与网络占用，不打开终端页签；
//! 结果按连接缓存，过期后再采集。只对在监控页开启「资源监控」的连接生效，
//! 默认不采集；任何失败都只记 debug 日志，不影响首页交互。
//!
//! 资源条只在列表布局显示，并压成与地址同一行的紧凑形式：行高不因显示资源而变高。

use std::collections::{BTreeSet, HashMap, HashSet};
use std::time::{Duration, Instant};

use gpui::prelude::FluentBuilder as _;
use gpui::{
    AnyElement, App, Context, Hsla, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div,
};
use gpui_component::{ActiveTheme, Sizable, Size, h_flex, progress::Progress, tooltip::Tooltip};
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection,
};
use rust_i18n::t;

use super::{ConnectionLayout, HomePage};

/// 同一连接资源快照的缓存时间。
const HOST_METRICS_TTL: Duration = Duration::from_secs(300);
/// 定时检查过期快照的间隔；实际采集仍受 [`HOST_METRICS_TTL`] 控制。
const HOST_METRICS_REFRESH_INTERVAL: Duration = Duration::from_secs(60);
/// 同时进行的后台采集上限：首页一次可能渲染几十个连接，不能同时拉起几十个 SSH 会话。
const HOST_METRICS_MAX_IN_FLIGHT: usize = 2;
/// 资源轨道宽度：单行内依次放下「标签 · 容量」、比例条与百分比，四项在常见窗口宽度内保持同一行。
const HOST_METRIC_TRACK_WIDTH_REMS: f32 = 9.0;
/// 单行轨道里的比例条宽度：只用来表达饱和程度，不再单独占一行。
const HOST_METRIC_BAR_WIDTH_REMS: f32 = 1.5;
/// 网络轨道比资源轨道宽：一行要同时容纳「网速」标签与上下行两个速率。
const HOST_NETWORK_TRACK_WIDTH_REMS: f32 = 10.5;

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
        if self.connection_layout != ConnectionLayout::List
            || !host_metrics_enabled(cx)
            || self.saved_connections_locked()
        {
            return;
        }

        // 每个任务完成后都会再次调用本方法，把超出并发上限的连接继续排空。
        let enabled = AppSettings::global(cx).monitored_connection_ids.clone();
        while self.host_metrics.in_flight.len() < HOST_METRICS_MAX_IN_FLIGHT {
            let Some((connection_id, connection)) = self.next_host_metrics_candidate(&enabled)
            else {
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
            let monitored = enabled.clone();
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
                        Ok(usage)
                            if host_metrics_enabled(cx) && monitored.contains(&connection_id) =>
                        {
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

    fn next_host_metrics_candidate(
        &self,
        enabled: &BTreeSet<i64>,
    ) -> Option<(i64, one_core::storage::StoredConnection)> {
        self.connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .find_map(|connection| host_metrics_candidate(connection, &self.host_metrics, enabled))
    }

    /// 列表使用的资源标签（没有缓存快照或设置关闭时返回 None）。
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

    pub(crate) fn shows_host_metrics_strip(&self, connection: &StoredConnection, cx: &App) -> bool {
        let Some(connection_id) = connection.id else {
            return false;
        };
        self.connection_layout == ConnectionLayout::List
            && host_metrics_enabled(cx)
            && AppSettings::global(cx).connection_monitoring_enabled(connection_id)
            && connection.connection_type == ConnectionType::SshSftp
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

/// 只对用户开启了资源监控、且已过缓存/退避窗口的 SSH 连接排队。
fn host_metrics_candidate(
    connection: &StoredConnection,
    cache: &HostMetricsCache,
    enabled: &BTreeSet<i64>,
) -> Option<(i64, StoredConnection)> {
    let connection_id = connection.id?;
    if !enabled.contains(&connection_id) {
        return None;
    }
    if cache.in_flight.contains(&connection_id) {
        return None;
    }
    if cache
        .entries
        .get(&connection_id)
        .is_some_and(|entry| entry.sampled_at.elapsed() < HOST_METRICS_TTL)
    {
        return None;
    }
    if cache
        .last_attempt
        .get(&connection_id)
        .is_some_and(|attempted_at| attempted_at.elapsed() < HOST_METRICS_TTL)
    {
        return None;
    }
    if let Ok(params) = connection.to_ssh_params()
        && (params.prompts_for_username()
            || (params.prompts_for_password() && params.auth_method.contains_password()))
    {
        return None;
    }
    Some((connection_id, connection.clone()))
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
        .items_center()
        .gap_3()
        .when_some(usage.os_name.as_ref(), |row, os_name| {
            let tooltip: SharedString = os_name.clone().into();
            row.child(
                div()
                    .id(SharedString::from(format!("{id_prefix}-os")))
                    .flex_shrink_0()
                    .max_w(gpui::rems(10.0))
                    .overflow_hidden()
                    .text_ellipsis()
                    .whitespace_nowrap()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .tooltip(move |window, cx| Tooltip::new(tooltip.clone()).build(window, cx))
                    .child(os_name.clone()),
            )
        })
        .children(metrics.into_iter().map(|(kind, value)| {
            render_host_metric_track(
                SharedString::from(format!("{id_prefix}-{}", kind.id_suffix())),
                kind.label(),
                value,
                host_metric_detail(usage, kind),
                cx,
            )
        }))
        .when_some(host_network_rates(usage), |row, (rx, tx)| {
            row.child(render_host_network_track(
                SharedString::from(format!("{id_prefix}-network")),
                rx,
                tx,
                cx,
            ))
        })
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

fn render_host_metric_track(
    id: SharedString,
    label: String,
    value: f64,
    detail: Option<String>,
    cx: &App,
) -> AnyElement {
    let tone = metric_color(value, cx);
    let percent = format!("{value:.1}%");
    let caption = match detail.as_ref() {
        Some(detail) => format!("{label} · {detail}"),
        None => label.clone(),
    };
    let tooltip_text: SharedString = match detail.as_ref() {
        Some(detail) => format!("{label} · {detail} · {percent}"),
        None => format!("{label} · {percent}"),
    }
    .into();
    let accessibility_label = tooltip_text.clone();
    let progress_id = SharedString::from(format!("{}-progress", id.as_ref()));

    // 单行轨道：标签、比例条与百分比并排，行高与普通说明文字一致，
    // 资源信息不再额外撑高连接行（redesign §10.2 的紧凑列表）。
    h_flex()
        .id(id)
        .w(gpui::rems(HOST_METRIC_TRACK_WIDTH_REMS))
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .overflow_hidden()
                .text_ellipsis()
                .whitespace_nowrap()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(caption),
        )
        .child(
            Progress::new(progress_id)
                .with_size(Size::XSmall)
                .color(tone)
                .value(value.clamp(0.0, 100.0) as f32)
                .w(gpui::rems(HOST_METRIC_BAR_WIDTH_REMS))
                .flex_shrink_0()
                .accessibility_label(accessibility_label),
        )
        .child(
            div()
                .flex_shrink_0()
                .w_10()
                .text_right()
                .whitespace_nowrap()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .text_color(if value >= 75.0 {
                    tone
                } else {
                    cx.theme().foreground
                })
                .child(percent),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
        .into_any_element()
}

fn render_host_network_track(id: SharedString, rx: String, tx: String, cx: &App) -> AnyElement {
    let label = t!("Home.host_metrics_network_label").to_string();
    let up = t!("Home.host_metrics_network_up", value = tx.clone()).to_string();
    let down = t!("Home.host_metrics_network_down", value = rx.clone()).to_string();
    let tooltip_text: SharedString = format!("{label} · {up} · {down}").into();

    // 上下行并排成一行：网速只是参考值，不需要单独占一行高度。
    h_flex()
        .id(id)
        .w(gpui::rems(HOST_NETWORK_TRACK_WIDTH_REMS))
        .flex_shrink_0()
        .items_center()
        .gap_1()
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("↑"),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(tx),
        )
        .child(
            div()
                .flex_shrink_0()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child("↓"),
        )
        .child(
            div()
                .flex_shrink_0()
                .whitespace_nowrap()
                .text_xs()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(rx),
        )
        .tooltip(move |window, cx| Tooltip::new(tooltip_text.clone()).build(window, cx))
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

fn host_network_rates(usage: &terminal_view::HostResourceUsage) -> Option<(String, String)> {
    let rx = usage.network_rx_bytes_per_sec?;
    let tx = usage.network_tx_bytes_per_sec?;
    Some((format_network_rate(rx), format_network_rate(tx)))
}

fn format_network_rate(bytes_per_sec: f64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let value = bytes_per_sec.max(0.0);
    if value >= GB {
        format!("{:.1} GB/s", value / GB)
    } else if value >= MB {
        format!("{:.1} MB/s", value / MB)
    } else if value >= KB {
        format!("{:.1} KB/s", value / KB)
    } else {
        format!("{value:.0} B/s")
    }
}

fn nearly_whole(value: f64) -> bool {
    (value - value.round()).abs() < 0.05
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
                network_rx_bytes_per_sec: None,
                network_tx_bytes_per_sec: None,
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
                network_rx_bytes_per_sec: None,
                network_tx_bytes_per_sec: None,
            }),
            Vec::new()
        );
    }

    #[test]
    fn network_rate_formatter_uses_compact_binary_units() {
        assert_eq!(format_network_rate(830.0), "830 B/s");
        assert_eq!(format_network_rate(2.9 * 1024.0), "2.9 KB/s");
        assert_eq!(format_network_rate(3.25 * 1024.0 * 1024.0), "3.2 MB/s");
    }

    #[test]
    fn host_metrics_candidate_requires_per_connection_monitoring() {
        let connection = ssh_connection_for_test(42);
        let cache = HostMetricsCache::default();
        let enabled = [42].into_iter().collect::<std::collections::BTreeSet<_>>();

        assert_eq!(
            host_metrics_candidate(&connection, &cache, &enabled).map(|(id, _)| id),
            Some(42)
        );
        assert!(
            host_metrics_candidate(&connection, &cache, &std::collections::BTreeSet::new())
                .is_none()
        );
    }

    fn ssh_connection_for_test(id: i64) -> StoredConnection {
        let params: one_core::storage::SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "None",
        }))
        .expect("ssh params");
        let mut connection = StoredConnection::new_ssh(format!("host-{id}"), params, None);
        connection.id = Some(id);
        connection
    }

    #[test]
    fn compact_capacity_uses_reference_style_units() {
        assert_eq!(format_kib_compact(27 * 1024 * 1024), "27G");
        assert_eq!(format_kib_compact(247 * 1024 * 1024), "247G");
        assert_eq!(format_kib_compact(512 * 1024), "512M");
    }
}
