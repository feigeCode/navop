//! 首页连接卡片的远端资源占用（#279）。
//!
//! 用一次性的后台 SSH 会话采集 CPU、内存与磁盘占用，不打开终端页签；
//! 结果按连接缓存，过期后再采集。任何失败都只记 debug 日志，不影响首页交互。

use std::time::{Duration, Instant};

use gpui::{AppContext, Context};
use one_core::settings::AppSettings;
use one_core::storage::models::ConnectionType;

use super::HomePage;

/// 同一连接资源快照的缓存时间。
const HOST_METRICS_TTL: Duration = Duration::from_secs(300);
/// 同时进行的后台采集上限：首页一次可能渲染几十个连接，不能同时拉起几十个 SSH 会话。
const HOST_METRICS_MAX_IN_FLIGHT: usize = 2;

pub(crate) struct HostMetricsEntry {
    usage: terminal_view::HostResourceUsage,
    sampled_at: Instant,
}

#[derive(Default)]
pub(crate) struct HostMetricsCache {
    entries: std::collections::HashMap<i64, HostMetricsEntry>,
    in_flight: std::collections::HashSet<i64>,
}

impl HomePage {
    /// 为首页的 SSH 连接排队后台采集（设置关闭或主密钥未解锁时直接跳过）。
    pub(super) fn refresh_host_metrics(&mut self, cx: &mut Context<Self>) {
        if !AppSettings::global(cx).home_show_host_metrics || self.saved_connections_locked() {
            return;
        }
        let pending = self
            .connections
            .iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .filter_map(|connection| {
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
                Some((connection_id, connection.clone()))
            })
            .collect::<Vec<_>>();

        for (connection_id, connection) in pending {
            if self.host_metrics.in_flight.len() >= HOST_METRICS_MAX_IN_FLIGHT {
                break;
            }
            let Ok(config) = terminal::terminal::ssh_terminal_config_from_connection(&connection)
            else {
                continue;
            };
            self.host_metrics.in_flight.insert(connection_id);
            let task = cx.background_spawn(async move {
                terminal_view::probe_host_resource_usage(config.ssh_config).await
            });
            cx.spawn(async move |this, cx| {
                let usage = task.await;
                _ = this.update(cx, |this, cx| {
                    this.host_metrics.in_flight.remove(&connection_id);
                    match usage {
                        Ok(usage) => {
                            this.host_metrics.entries.insert(
                                connection_id,
                                HostMetricsEntry {
                                    usage,
                                    sampled_at: Instant::now(),
                                },
                            );
                        }
                        Err(error) => {
                            tracing::debug!(connection_id, %error, "采集远端资源占用失败");
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
        }
    }

    /// 卡片上显示的一行资源摘要（没有缓存快照时返回 None）。
    pub(crate) fn host_metrics_summary(&self, connection_id: Option<i64>) -> Option<String> {
        let connection_id = connection_id?;
        self.host_metrics
            .entries
            .get(&connection_id)?
            .usage
            .summary()
    }
}
