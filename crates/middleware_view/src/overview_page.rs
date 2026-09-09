//! 中间件概览页:集群节点卡片 + 运行指标统计区。
//!
//! - `cluster_overview=false` 时集群区显示"不支持"占位
//! - `metrics=false` 时指标区显示"不支持"占位
//! - 两者皆不可用时整页占位

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AsyncApp, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Task, Window, div, px,
};
use gpui_component::{ActiveTheme, Icon, IconName, IconSize, Sizable, h_flex, v_flex};
use middleware_runtime::{ClusterOverview, MiddlewareCapabilities, MiddlewareMetrics};
use one_core::gpui_tokio::Tokio;
use one_core::tab_container::{TabContent, TabContentEvent};
use rust_i18n::t;
use tracing::warn;

use crate::MiddlewareAdminHandle;
use crate::common::{LoadState, notify_async, page_header, render_unsupported};

/// 中间件概览页
pub struct MiddlewareOverviewPage {
    /// 管理接口句柄
    handle: MiddlewareAdminHandle,
    /// 能力位快照(构造时读取)
    capabilities: MiddlewareCapabilities,
    /// 集群概览数据
    overview: Option<ClusterOverview>,
    /// 指标快照
    metrics: Option<MiddlewareMetrics>,
    /// 加载状态
    load_state: LoadState,
    /// 刷新代号(递增使旧在途刷新的回写失效)
    refresh_generation: u64,
    /// 焦点句柄
    focus_handle: FocusHandle,
}

impl MiddlewareOverviewPage {
    /// 创建概览页并立即加载
    pub fn new(handle: MiddlewareAdminHandle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let _ = window;
        let capabilities = handle.capabilities();
        let mut this = Self {
            handle,
            capabilities,
            overview: None,
            metrics: None,
            load_state: LoadState::Idle,
            refresh_generation: 0,
            focus_handle: cx.focus_handle(),
        };
        this.refresh(cx);
        this
    }

    /// 同时刷新集群概览与指标快照(两请求并发执行,回写带 generation 防护)
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.load_state = LoadState::Loading;
        // 递增代号使旧的在途刷新回写失效
        let generation = self.refresh_generation.wrapping_add(1);
        self.refresh_generation = generation;
        let handle = self.handle.clone();
        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 先创建两个任务再依次汇合:两请求在 Tokio 运行时并发执行(F6)
            // 两个数据源各自尽力加载,单个失败不影响另一个
            let metrics_task = if handle.capabilities().metrics {
                Some(Tokio::spawn_result(cx, {
                    let handle = handle.clone();
                    async move {
                        handle
                            .metrics_snapshot()
                            .await
                            .map(Some)
                            .map_err(anyhow::Error::new)
                    }
                }))
            } else {
                None
            };
            let overview_task = if handle.capabilities().cluster_overview {
                Some(Tokio::spawn_result(cx, {
                    let handle = handle.clone();
                    async move { handle.cluster_overview().await.map_err(anyhow::Error::new) }
                }))
            } else {
                None
            };
            let metrics_result = match metrics_task {
                Some(task) => task.await,
                None => Ok::<Option<MiddlewareMetrics>, anyhow::Error>(None),
            };
            let overview_result = match overview_task {
                Some(task) => task.await.map(Some),
                None => Ok::<Option<ClusterOverview>, anyhow::Error>(None),
            };

            let metrics = match metrics_result {
                Ok(metrics) => metrics,
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "中间件指标加载失败");
                    notify_async(
                        cx,
                        gpui_component::notification::Notification::error(
                            t!("Middleware.load_failed", error = message).to_string(),
                        ),
                    );
                    None
                }
            };
            let overview = match overview_result {
                Ok(overview) => overview,
                Err(error) => {
                    let message = format!("{error:#}");
                    warn!(%message, "中间件集群概览加载失败");
                    notify_async(
                        cx,
                        gpui_component::notification::Notification::error(
                            t!("Middleware.load_failed", error = message).to_string(),
                        ),
                    );
                    None
                }
            };

            // generation 不匹配说明期间又发起了新刷新,过期数据直接丢弃(F2)
            _ = this.update(cx, |view, cx| {
                if view.refresh_generation != generation {
                    return;
                }
                view.metrics = metrics;
                view.overview = overview;
                view.load_state = LoadState::Loaded;
                cx.notify();
            });
        })
        .detach();
    }

    /// 指标统计卡片
    fn render_metric_card(
        id: &str,
        label: SharedString,
        value: SharedString,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        v_flex()
            .id(SharedString::from(format!("metric-{id}")))
            .flex_1()
            .min_w(px(140.0))
            .gap_1()
            .p_3()
            .rounded(cx.theme().geometry.radius.md)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(label),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(value),
            )
    }

    /// 指标统计区(卡片网格)
    fn render_metrics(&self, cx: &Context<Self>) -> impl IntoElement {
        let Some(metrics) = &self.metrics else {
            return render_unsupported(
                cx,
                t!("Middleware.metrics_unsupported").to_string().into(),
                t!("Middleware.metrics_unsupported_hint").to_string().into(),
            )
            .into_any_element();
        };
        let cards = vec![
            (
                "tps-in",
                t!("Middleware.tps_in").to_string(),
                format!("{:.1}", metrics.tps_in),
            ),
            (
                "tps-out",
                t!("Middleware.tps_out").to_string(),
                format!("{:.1}", metrics.tps_out),
            ),
            (
                "topic-count",
                t!("Middleware.topic_count").to_string(),
                metrics.topic_count.to_string(),
            ),
            (
                "connections",
                t!("Middleware.connection_count").to_string(),
                metrics.connection_count.to_string(),
            ),
            (
                "today",
                t!("Middleware.message_today").to_string(),
                metrics.message_count_today.to_string(),
            ),
        ]
        .into_iter()
        .map(|(id, label, value)| Self::render_metric_card(id, label.into(), value.into(), cx))
        .collect::<Vec<_>>();

        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(t!("Middleware.metrics_title").to_string()),
            )
            .child(h_flex().gap_2().children(cards))
            .when(!metrics.extras.is_empty(), |this| {
                this.child(
                    h_flex().gap_2().flex_wrap().children(
                        metrics
                            .extras
                            .iter()
                            .map(|(key, value)| {
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!("{key}: {value}"))
                            })
                            .collect::<Vec<_>>(),
                    ),
                )
            })
            .into_any_element()
    }

    /// 单个 Broker 卡片
    fn render_broker_card(
        cluster_name: &str,
        index: usize,
        broker: &middleware_runtime::BrokerInfo,
        cx: &Context<Self>,
    ) -> impl IntoElement {
        let rows = vec![
            (t!("Middleware.address").to_string(), broker.address.clone()),
            (
                t!("Middleware.version").to_string(),
                broker.version.clone().unwrap_or_else(|| "-".into()),
            ),
            (
                t!("Middleware.topics_label").to_string(),
                broker.topic_count.to_string(),
            ),
            (
                t!("Middleware.queues_label").to_string(),
                broker.queue_count.to_string(),
            ),
            (
                t!("Middleware.tps_in").to_string(),
                format!("{:.1}", broker.tps_in),
            ),
            (
                t!("Middleware.tps_out").to_string(),
                format!("{:.1}", broker.tps_out),
            ),
        ];
        v_flex()
            .id(SharedString::from(format!("broker-{cluster_name}-{index}")))
            .min_w(px(240.0))
            .max_w(px(320.0))
            .gap_1()
            .p_3()
            .rounded(cx.theme().geometry.radius.md)
            .border_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Server)
                            .with_size(IconSize::Small)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .truncate()
                            .child(broker.name.clone()),
                    ),
            )
            .children(
                rows.into_iter()
                    .map(|(label, value)| {
                        h_flex()
                            .gap_2()
                            .child(
                                div()
                                    .w(px(90.0))
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(label),
                            )
                            .child(div().flex_1().min_w_0().text_sm().truncate().child(value))
                    })
                    .collect::<Vec<_>>(),
            )
    }

    /// 集群节点卡片列表
    fn render_cluster(&self, cx: &Context<Self>) -> impl IntoElement {
        if !self.capabilities.cluster_overview {
            return render_unsupported(
                cx,
                t!("Middleware.cluster_unsupported").to_string().into(),
                t!("Middleware.cluster_unsupported_hint").to_string().into(),
            )
            .into_any_element();
        }
        let Some(overview) = &self.overview else {
            return v_flex()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Middleware.no_cluster_data").to_string()),
                )
                .into_any_element();
        };
        if overview.clusters.is_empty() {
            return v_flex()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Middleware.no_cluster_data").to_string()),
                )
                .into_any_element();
        }
        v_flex()
            .gap_2()
            .child(
                div()
                    .text_sm()
                    .font_weight(gpui::FontWeight::SEMIBOLD)
                    .child(t!("Middleware.cluster_title").to_string()),
            )
            .children(
                overview
                    .clusters
                    .iter()
                    .map(|cluster| {
                        v_flex()
                            .gap_2()
                            .child(
                                h_flex().gap_2().items_center().child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{}: {}",
                                            t!("Middleware.cluster_label"),
                                            cluster.name
                                        )),
                                ),
                            )
                            .child(
                                h_flex().gap_2().flex_wrap().children(
                                    cluster
                                        .brokers
                                        .iter()
                                        .enumerate()
                                        .map(|(index, broker)| {
                                            Self::render_broker_card(
                                                &cluster.name,
                                                index,
                                                broker,
                                                cx,
                                            )
                                        })
                                        .collect::<Vec<_>>(),
                                ),
                            )
                    })
                    .collect::<Vec<_>>(),
            )
            .into_any_element()
    }
}

impl Focusable for MiddlewareOverviewPage {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for MiddlewareOverviewPage {}

impl TabContent for MiddlewareOverviewPage {
    fn content_key(&self) -> &'static str {
        "middleware-overview"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("Middleware.overview_tab").to_string().into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(Icon::new(IconName::ChartPie).with_size(IconSize::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn try_close(
        &mut self,
        _tab_id: &str,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Task<bool> {
        Task::ready(true)
    }
}

impl Render for MiddlewareOverviewPage {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let border_color = cx.theme().border;
        v_flex()
            .id("middleware-overview-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(cx.theme().background)
            .child(page_header(
                "middleware-overview",
                IconName::ChartPie,
                t!("Middleware.overview_tab").to_string().into(),
                border_color,
                {
                    let view = cx.entity().clone();
                    move |_, _, cx| {
                        view.update(cx, |view, cx| view.refresh(cx));
                    }
                },
            ))
            .child(
                div()
                    .id("middleware-overview-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .p_3()
                    .child(
                        v_flex()
                            .gap_4()
                            .child(self.render_metrics(cx))
                            .child(div().h(px(0.0)).border_b_1().border_color(border_color))
                            .child(self.render_cluster(cx)),
                    ),
            )
    }
}
