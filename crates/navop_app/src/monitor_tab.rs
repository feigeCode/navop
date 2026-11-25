//! Dedicated multi-host SSH monitoring page.
//!
//! Each saved SSH connection owns one long-lived monitor session. The session
//! samples on the configured interval and updates a single retained card, so
//! the dashboard does not perform a fresh SSH handshake for every refresh.
//!
//! 只有用户按连接开启「资源监控」的主机才会建立后台会话；默认不采集，这份
//! 选择与首页连接列表共用（`AppSettings::monitored_connection_ids`）。

use std::collections::{BTreeSet, HashMap};
use std::time::Duration;

use chrono::Local;
use gpui::prelude::FluentBuilder as _;
use gpui::{
    Anchor, AnyElement, App, AppContext as _, Context, Entity, EventEmitter, FocusHandle,
    Focusable, Hsla, InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled,
    Task, Window, div, linear_color_stop, linear_gradient, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Sizable, StyledExt,
    button::{Button, DropdownButton},
    chart::{AreaChart, LineChart},
    h_flex,
    input::{Input, InputEvent, InputState},
    menu::PopupMenuItem,
    progress::Progress,
    scroll::ScrollableElement as _,
    spinner::Spinner,
    switch::Switch,
    v_flex,
};
use one_assets::IconName;
use one_core::gpui_tokio::Tokio;
use one_core::settings::AppSettings;
use one_core::storage::{
    ConnectionRepository, ConnectionType, GlobalStorageState, StoredConnection,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::{IconButton, IconButtonRole, IconSize};
use rust_i18n::t;
use terminal_view::{HostMonitorSession, HostMonitorSnapshot};

const MONITOR_HISTORY_LIMIT: usize = 30;
const MONITOR_CARD_MIN_WIDTH: gpui::Pixels = px(220.0);
const MONITOR_STAGGER_DELAY: Duration = Duration::from_millis(220);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RefreshInterval {
    FiveSeconds,
    FifteenSeconds,
    ThirtySeconds,
}

impl RefreshInterval {
    const ALL: [Self; 3] = [Self::FiveSeconds, Self::FifteenSeconds, Self::ThirtySeconds];

    fn seconds(self) -> u64 {
        match self {
            Self::FiveSeconds => 5,
            Self::FifteenSeconds => 15,
            Self::ThirtySeconds => 30,
        }
    }

    fn duration(self) -> Duration {
        Duration::from_secs(self.seconds())
    }

    fn label(self) -> String {
        t!(
            "Monitor.every_seconds",
            seconds = self.seconds().to_string()
        )
        .to_string()
    }
}

enum MonitorCardState {
    /// 用户未开启这台主机的资源监控：不建连、不采集。
    Idle,
    Sampling,
    Ready,
    Failed(String),
}

struct MonitorCard {
    connection: StoredConnection,
    title: String,
    address: String,
    snapshot: Option<HostMonitorSnapshot>,
    state: MonitorCardState,
    last_updated: Option<String>,
    cpu_history: Vec<f64>,
    rx_history: Vec<f64>,
    tx_history: Vec<f64>,
}

impl MonitorCard {
    fn new(connection: StoredConnection) -> Self {
        let params = connection.to_ssh_params().ok();
        let address = params
            .as_ref()
            .map(|params| format!("{}@{}:{}", params.username, params.host, params.port))
            .unwrap_or_else(|| connection.name.clone());
        Self {
            title: connection.name.clone(),
            address,
            connection,
            snapshot: None,
            state: MonitorCardState::Idle,
            last_updated: None,
            cpu_history: Vec::new(),
            rx_history: Vec::new(),
            tx_history: Vec::new(),
        }
    }

    fn apply_snapshot(&mut self, snapshot: HostMonitorSnapshot) {
        if let Some(cpu) = snapshot.cpu_percent {
            push_history(&mut self.cpu_history, cpu);
        }
        if let (Some(rx), Some(tx)) = (
            snapshot.network_rx_bytes_per_sec,
            snapshot.network_tx_bytes_per_sec,
        ) {
            push_history(&mut self.rx_history, rx);
            push_history(&mut self.tx_history, tx);
        }
        self.snapshot = Some(snapshot);
        self.state = MonitorCardState::Ready;
        self.last_updated = Some(Local::now().format("%H:%M:%S").to_string());
    }

    fn fail(&mut self, error: String) {
        self.state = MonitorCardState::Failed(error);
    }

    fn reset_to_idle(&mut self) {
        self.state = MonitorCardState::Idle;
        self.snapshot = None;
        self.last_updated = None;
        self.cpu_history.clear();
        self.rx_history.clear();
        self.tx_history.clear();
    }
}

pub(crate) struct MonitorTab {
    focus_handle: FocusHandle,
    search_input: Entity<InputState>,
    search_query: String,
    cards: Vec<MonitorCard>,
    interval: RefreshInterval,
    sampling_tasks: HashMap<i64, Task<()>>,
}

impl MonitorTab {
    pub(crate) fn new(
        connections: Vec<StoredConnection>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("Monitor.search_placeholder").to_string())
                .clean_on_escape()
        });
        cx.subscribe(&search_input, |this, input, event: &InputEvent, cx| {
            if matches!(event, InputEvent::Change) {
                this.search_query = input.read(cx).value().trim().to_lowercase();
                cx.notify();
            }
        })
        .detach();

        let mut cards = connections
            .into_iter()
            .filter(|connection| connection.connection_type == ConnectionType::SshSftp)
            .filter(|connection| connection.id.is_some())
            .map(MonitorCard::new)
            .collect::<Vec<_>>();
        cards.sort_by_cached_key(|card| card.title.to_lowercase());

        Self {
            focus_handle: cx.focus_handle(),
            search_input,
            search_query: String::new(),
            cards,
            interval: RefreshInterval::FiveSeconds,
            sampling_tasks: HashMap::new(),
        }
    }

    fn set_interval(&mut self, interval: RefreshInterval, cx: &mut Context<Self>) {
        if self.interval == interval {
            return;
        }
        self.interval = interval;
        self.restart_sampling(cx);
    }

    pub(crate) fn restart_sampling(&mut self, cx: &mut Context<Self>) {
        self.sampling_tasks.clear();
        let enabled = monitored_connection_ids(cx);
        for card in &mut self.cards {
            card.state = if card_is_monitored(card, &enabled) {
                MonitorCardState::Sampling
            } else {
                MonitorCardState::Idle
            };
        }
        let specs = monitored_cards(&self.cards, &enabled)
            .into_iter()
            .filter_map(|card| card.connection.id.map(|id| (id, card.connection.clone())))
            .collect::<Vec<_>>();
        for (index, (connection_id, connection)) in specs.into_iter().enumerate() {
            self.start_card_sampling(connection_id, connection, index as u32, cx);
        }
        cx.notify();
    }

    /// 只为单个连接建立/重建长连接会话；开启开关后立即生效。
    fn start_card_sampling(
        &mut self,
        connection_id: i64,
        connection: StoredConnection,
        stagger_index: u32,
        cx: &mut Context<Self>,
    ) {
        let repository = cx
            .global::<GlobalStorageState>()
            .storage
            .get::<ConnectionRepository>();
        let config = match build_monitor_config(
            repository.as_ref().map(|repo| repo.as_ref()),
            &connection,
        ) {
            Ok(config) => config,
            Err(error) => {
                if let Some(card) = self
                    .cards
                    .iter_mut()
                    .find(|card| card.connection.id == Some(connection_id))
                {
                    card.fail(error);
                }
                cx.notify();
                return;
            }
        };
        let interval = self.interval.duration();
        let task = cx.spawn(async move |this, cx| {
            if stagger_index > 0 {
                cx.background_executor()
                    .timer(MONITOR_STAGGER_DELAY * stagger_index)
                    .await;
            }

            let mut session = Some(HostMonitorSession::new(config.0, config.1));
            loop {
                let Some(active_session) = session.take() else {
                    break;
                };
                let sample = Tokio::spawn(cx, async move {
                    let mut active_session = active_session;
                    let result = active_session.sample().await;
                    (active_session, result)
                });
                let (next_session, result) = match sample.await {
                    Ok(value) => value,
                    Err(_) => break,
                };
                session = Some(next_session);

                let still_open = this
                    .update(cx, |this, cx| {
                        let Some(card) = this
                            .cards
                            .iter_mut()
                            .find(|card| card.connection.id == Some(connection_id))
                        else {
                            return false;
                        };
                        match result {
                            Ok(snapshot) => card.apply_snapshot(snapshot),
                            Err(error) => card.fail(format!("{error:#}")),
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !still_open {
                    break;
                }

                cx.background_executor().timer(interval).await;
            }
            if let Some(session) = session {
                let _ = Tokio::spawn(cx, async move {
                    session.disconnect().await;
                })
                .await;
            }
        });
        self.sampling_tasks.insert(connection_id, task);
    }

    /// 卡片开关：写入共享的按连接选择，并立即启停该连接的采集会话。
    fn set_card_monitoring(&mut self, connection_id: i64, enabled: bool, cx: &mut Context<Self>) {
        if AppSettings::global(cx).connection_monitoring_enabled(connection_id) == enabled {
            return;
        }
        AppSettings::update_and_save(cx, |settings| {
            settings.set_connection_monitoring(connection_id, enabled);
        });

        if !enabled {
            self.sampling_tasks.remove(&connection_id);
            if let Some(card) = self
                .cards
                .iter_mut()
                .find(|card| card.connection.id == Some(connection_id))
            {
                card.reset_to_idle();
            }
            cx.notify();
            return;
        }

        let Some(connection) = self
            .cards
            .iter()
            .find(|card| card.connection.id == Some(connection_id))
            .map(|card| card.connection.clone())
        else {
            return;
        };
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.connection.id == Some(connection_id))
        {
            card.state = MonitorCardState::Sampling;
        }
        self.start_card_sampling(connection_id, connection, 0, cx);
        cx.notify();
    }

    /// 单卡刷新：只重连这一台主机，不影响其它已开启的监控。
    fn restart_card_sampling(&mut self, connection_id: i64, cx: &mut Context<Self>) {
        if !AppSettings::global(cx).connection_monitoring_enabled(connection_id) {
            return;
        }
        self.sampling_tasks.remove(&connection_id);
        let Some(connection) = self
            .cards
            .iter()
            .find(|card| card.connection.id == Some(connection_id))
            .map(|card| card.connection.clone())
        else {
            return;
        };
        if let Some(card) = self
            .cards
            .iter_mut()
            .find(|card| card.connection.id == Some(connection_id))
        {
            card.state = MonitorCardState::Sampling;
        }
        self.start_card_sampling(connection_id, connection, 0, cx);
        cx.notify();
    }

    fn filtered_card_indices(&self) -> Vec<usize> {
        self.cards
            .iter()
            .enumerate()
            .filter(|(_, card)| {
                self.search_query.is_empty()
                    || card.title.to_lowercase().contains(&self.search_query)
                    || card.address.to_lowercase().contains(&self.search_query)
                    || card
                        .snapshot
                        .as_ref()
                        .and_then(|snapshot| snapshot.os_name.as_deref())
                        .is_some_and(|os| os.to_lowercase().contains(&self.search_query))
            })
            .map(|(index, _)| index)
            .collect()
    }

    fn render_header(&self, enabled: &BTreeSet<i64>, cx: &mut Context<Self>) -> AnyElement {
        let interval = self.interval;
        let interval_menu_view = cx.entity();
        let monitored_count = monitored_cards(&self.cards, enabled).len();
        let refresh_enabled = monitored_count > 0;
        h_flex()
            .w_full()
            .min_w_0()
            .flex_wrap()
            .items_center()
            .gap_3()
            .px_6()
            .py_4()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                v_flex()
                    .flex_1()
                    .min_w(px(240.0))
                    .gap_1()
                    .child(
                        div()
                            .text_xl()
                            .font_semibold()
                            .child(t!("Monitor.title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Monitor.subtitle").to_string()),
                    ),
            )
            .child(Input::new(&self.search_input).small().w(px(240.0)))
            .child(
                div()
                    .flex_shrink_0()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!(
                            "Monitor.enabled_summary",
                            enabled = monitored_count.to_string(),
                            total = self.cards.len().to_string()
                        )
                        .to_string(),
                    ),
            )
            .child(
                DropdownButton::new("monitor-interval")
                    .button(
                        Button::new("monitor-interval-button")
                            .outline()
                            .small()
                            .icon(
                                Icon::new(IconName::Calendar)
                                    .mono()
                                    .with_size(IconSize::Small),
                            )
                            .label(interval.label())
                            .on_click(cx.listener(|this, _, _, cx| {
                                let next = match this.interval {
                                    RefreshInterval::FiveSeconds => RefreshInterval::FifteenSeconds,
                                    RefreshInterval::FifteenSeconds => {
                                        RefreshInterval::ThirtySeconds
                                    }
                                    RefreshInterval::ThirtySeconds => RefreshInterval::FiveSeconds,
                                };
                                this.set_interval(next, cx);
                            })),
                    )
                    .dropdown_menu_with_anchor(Anchor::TopRight, move |menu, _, _| {
                        RefreshInterval::ALL.into_iter().fold(menu, |menu, option| {
                            let view = interval_menu_view.clone();
                            menu.item(
                                PopupMenuItem::new(option.label())
                                    .checked(option == interval)
                                    .on_click(move |_, _, cx| {
                                        view.update(cx, |this, cx| {
                                            this.set_interval(option, cx);
                                        });
                                    }),
                            )
                        })
                    }),
            )
            .child(
                Button::new("monitor-refresh-all")
                    .outline()
                    .small()
                    .disabled(!refresh_enabled)
                    .icon(
                        Icon::new(IconName::Refresh)
                            .mono()
                            .with_size(IconSize::Small),
                    )
                    .label(t!("Monitor.refresh").to_string())
                    .tooltip(t!("Monitor.refresh_all").to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.restart_sampling(cx))),
            )
            .into_any_element()
    }

    fn render_card(
        &self,
        index: usize,
        enabled: &BTreeSet<i64>,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let card = &self.cards[index];
        let card_id = card.connection.id.unwrap_or(index as i64);
        let monitored = card_is_monitored(card, enabled);
        let snapshot = card.snapshot.as_ref();
        let accent = cx.theme().chart_1;
        let accent_soft = accent.opacity(0.12);
        let status = self.render_status(card, cx);

        let card_body =
            v_flex()
                .w_full()
                .gap_4()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .bg(cx.theme().background)
                .p_5()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .items_center()
                        .gap_3()
                        .child(
                            div()
                                .size(px(42.0))
                                .flex_shrink_0()
                                .flex()
                                .items_center()
                                .justify_center()
                                .rounded_lg()
                                .bg(accent_soft)
                                .child(
                                    Icon::new(IconName::Server)
                                        .mono()
                                        .with_size(IconSize::Medium)
                                        .text_color(accent),
                                ),
                        )
                        .child(
                            v_flex()
                                .flex_1()
                                .min_w_0()
                                .gap_0p5()
                                .child(
                                    h_flex()
                                        .min_w_0()
                                        .gap_2()
                                        .child(
                                            div()
                                                .min_w_0()
                                                .overflow_hidden()
                                                .text_ellipsis()
                                                .whitespace_nowrap()
                                                .text_base()
                                                .font_semibold()
                                                .child(card.title.clone()),
                                        )
                                        .child(status),
                                )
                                .child(
                                    div()
                                        .min_w_0()
                                        .overflow_hidden()
                                        .text_ellipsis()
                                        .whitespace_nowrap()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(card.address.clone()),
                                )
                                .when_some(
                                    snapshot.and_then(|snapshot| snapshot.os_name.clone()),
                                    |this, os_name| {
                                        this.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(os_name),
                                        )
                                    },
                                ),
                        )
                        .when_some(card.last_updated.clone(), |this, updated| {
                            this.child(
                                div()
                                    .flex_shrink_0()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Monitor.updated_at", time = updated).to_string()),
                            )
                        })
                        .child(
                            h_flex()
                                .flex_shrink_0()
                                .items_center()
                                .gap_2()
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(t!("Monitor.card_toggle").to_string()),
                                )
                                .child(
                                    Switch::new(SharedString::from(format!(
                                        "monitor-card-toggle-{card_id}"
                                    )))
                                    .small()
                                    .checked(monitored)
                                    .tooltip(t!("Monitor.card_toggle_tooltip").to_string())
                                    .on_click(cx.listener(move |this, checked, _, cx| {
                                        this.set_card_monitoring(card_id, *checked, cx);
                                    })),
                                ),
                        )
                        .when(monitored, |this| {
                            this.child(
                                IconButton::new(
                                    SharedString::from(format!("monitor-card-refresh-{card_id}")),
                                    IconName::Refresh.mono(),
                                )
                                .role(IconButtonRole::Compact)
                                .tooltip(t!("Monitor.refresh_host"))
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.restart_card_sampling(card_id, cx);
                                    },
                                )),
                            )
                        }),
                )
                .when_some(card_error(card), |this, error| {
                    this.child(
                        div()
                            .w_full()
                            .rounded_md()
                            .bg(cx.theme().danger.opacity(0.08))
                            .px_3()
                            .py_2()
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(error),
                    )
                })
                .child(self.render_metrics(card, cx));

        div()
            .id(SharedString::from(format!("monitor-card-{card_id}")))
            .w_full()
            .min_w_0()
            .child(card_body)
            .into_any_element()
    }

    fn render_status(&self, card: &MonitorCard, cx: &Context<Self>) -> AnyElement {
        let (color, label) = match &card.state {
            MonitorCardState::Sampling => (
                cx.theme().info,
                if card.snapshot.is_some() {
                    t!("Monitor.refreshing").to_string()
                } else {
                    t!("Monitor.collecting").to_string()
                },
            ),
            MonitorCardState::Idle => (
                cx.theme().muted_foreground,
                t!("Monitor.not_monitoring").to_string(),
            ),
            MonitorCardState::Ready => (cx.theme().success, t!("Monitor.ready").to_string()),
            MonitorCardState::Failed(_) => {
                (cx.theme().danger, t!("Monitor.unavailable").to_string())
            }
        };
        h_flex()
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .rounded_full()
            .bg(color.opacity(0.1))
            .px_2()
            .py_0p5()
            .child(div().size_1p5().rounded_full().bg(color))
            .child(div().text_xs().text_color(color).child(label))
            .into_any_element()
    }

    fn render_metrics(&self, card: &MonitorCard, cx: &mut Context<Self>) -> AnyElement {
        if matches!(card.state, MonitorCardState::Idle) {
            return self.render_idle_state(card, cx);
        }
        let Some(snapshot) = card.snapshot.as_ref() else {
            if let MonitorCardState::Failed(_) = &card.state {
                return h_flex()
                    .w_full()
                    .min_h(px(120.0))
                    .items_center()
                    .justify_center()
                    .gap_2()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        Icon::new(IconName::TriangleAlert)
                            .mono()
                            .with_size(IconSize::Medium),
                    )
                    .child(t!("Monitor.unavailable").to_string())
                    .into_any_element();
            }
            return h_flex()
                .w_full()
                .min_h(px(120.0))
                .items_center()
                .justify_center()
                .gap_2()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(Spinner::new().small())
                .child(t!("Monitor.collecting").to_string())
                .into_any_element();
        };

        v_flex()
            .w_full()
            .gap_5()
            .child(
                h_flex()
                    .w_full()
                    .flex_wrap()
                    .items_stretch()
                    .gap_6()
                    .child(self.render_cpu_metric(card, snapshot, cx))
                    .child(self.render_memory_metric(card, snapshot, cx))
                    .child(self.render_network_metric(card, snapshot, cx)),
            )
            .child(self.render_disk_section(card, snapshot, cx))
            .into_any_element()
    }

    fn render_idle_state(&self, card: &MonitorCard, cx: &mut Context<Self>) -> AnyElement {
        let card_id = card.connection.id.unwrap_or_default();
        h_flex()
            .w_full()
            .min_h(px(120.0))
            .items_center()
            .justify_center()
            .gap_3()
            .child(
                Icon::new(IconName::Monitor)
                    .mono()
                    .with_size(IconSize::Medium)
                    .text_color(cx.theme().muted_foreground),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("Monitor.idle_title").to_string()),
                    )
                    .child(
                        div()
                            .max_w(px(420.0))
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Monitor.idle_description").to_string()),
                    ),
            )
            .child(
                Button::new(SharedString::from(format!("monitor-card-start-{card_id}")))
                    .outline()
                    .small()
                    .label(t!("Monitor.start_monitoring").to_string())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_card_monitoring(card_id, true, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_cpu_metric(
        &self,
        card: &MonitorCard,
        snapshot: &HostMonitorSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let percent = snapshot.cpu_percent.unwrap_or_default().clamp(0.0, 100.0);
        let color = usage_color(percent, cx);
        let core_count = snapshot.cpu_core_count();
        v_flex()
            .flex_1()
            .min_w(MONITOR_CARD_MIN_WIDTH)
            .gap_2()
            .child(metric_label(
                IconName::Monitor,
                t!("Monitor.cpu").to_string(),
                cx,
            ))
            .child(
                h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(color)
                            .child(format!("{percent:.1}%")),
                    )
                    .when_some(core_count, |this, cores| {
                        this.child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Monitor.cores", count = cores.to_string()).to_string()),
                        )
                    }),
            )
            .child(
                Progress::new(SharedString::from(format!(
                    "monitor-cpu-{:?}",
                    card.connection.id
                )))
                .color(color)
                .value(percent as f32),
            )
            .child(render_cpu_history(&card.cpu_history, cx))
            .into_any_element()
    }

    fn render_memory_metric(
        &self,
        card: &MonitorCard,
        snapshot: &HostMonitorSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let percent = snapshot
            .memory_percent()
            .unwrap_or_default()
            .clamp(0.0, 100.0);
        let color = usage_color(percent, cx);
        let used = snapshot
            .memory_used_kib
            .map(format_kib)
            .unwrap_or_else(|| "—".into());
        let total = snapshot
            .memory_total_kib
            .map(format_kib)
            .unwrap_or_else(|| "—".into());
        let cache = snapshot
            .memory_cached_kib
            .map(format_kib)
            .unwrap_or_else(|| "—".into());
        v_flex()
            .flex_1()
            .min_w(MONITOR_CARD_MIN_WIDTH)
            .gap_2()
            .child(metric_label(
                IconName::MemoryStick,
                t!("Monitor.memory").to_string(),
                cx,
            ))
            .child(
                h_flex()
                    .items_baseline()
                    .gap_2()
                    .child(
                        div()
                            .text_2xl()
                            .font_semibold()
                            .text_color(color)
                            .child(format!("{percent:.1}%")),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("{used} / {total}")),
                    ),
            )
            .child(
                Progress::new(SharedString::from(format!(
                    "monitor-memory-{:?}",
                    card.connection.id
                )))
                .color(color)
                .value(percent as f32),
            )
            .child(
                h_flex()
                    .gap_3()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Monitor.memory_cache", value = cache).to_string())
                    .when_some(snapshot.uptime_seconds, |this, uptime| {
                        this.child(t!("Monitor.uptime", value = format_uptime(uptime)).to_string())
                    }),
            )
            .into_any_element()
    }

    fn render_network_metric(
        &self,
        card: &MonitorCard,
        snapshot: &HostMonitorSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        let down = snapshot
            .network_rx_bytes_per_sec
            .map(format_bytes_per_sec)
            .unwrap_or_else(|| "—".into());
        let up = snapshot
            .network_tx_bytes_per_sec
            .map(format_bytes_per_sec)
            .unwrap_or_else(|| "—".into());
        v_flex()
            .flex_1()
            .min_w(MONITOR_CARD_MIN_WIDTH)
            .gap_2()
            .child(metric_label(
                IconName::Network,
                t!("Monitor.network").to_string(),
                cx,
            ))
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .gap_3()
                    .child(network_value(
                        IconName::ArrowDown,
                        t!("Monitor.download").to_string(),
                        down,
                        cx.theme().chart_2,
                        cx,
                    ))
                    .child(network_value(
                        IconName::ArrowUp,
                        t!("Monitor.upload").to_string(),
                        up,
                        cx.theme().chart_4,
                        cx,
                    )),
            )
            .child(render_network_history(
                &card.rx_history,
                &card.tx_history,
                cx,
            ))
            .into_any_element()
    }

    fn render_disk_section(
        &self,
        card: &MonitorCard,
        snapshot: &HostMonitorSnapshot,
        cx: &Context<Self>,
    ) -> AnyElement {
        if snapshot.disks.is_empty() {
            return div().into_any_element();
        }
        v_flex()
            .w_full()
            .gap_2()
            .pt_3()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .items_center()
                    .gap_2()
                    .child(
                        Icon::new(IconName::HardDrive)
                            .mono()
                            .with_size(IconSize::Small)
                            .text_color(cx.theme().muted_foreground),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(t!("Monitor.disk").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("Monitor.disk_hint").to_string()),
                    ),
            )
            .children(
                snapshot
                    .disks
                    .iter()
                    .take(6)
                    .enumerate()
                    .map(|(index, disk)| {
                        let color = usage_color(disk.percent, cx);
                        v_flex()
                            .w_full()
                            .gap_1()
                            .child(
                                h_flex()
                                    .w_full()
                                    .min_w_0()
                                    .justify_between()
                                    .gap_3()
                                    .child(
                                        h_flex()
                                            .min_w_0()
                                            .gap_2()
                                            .child(
                                                div()
                                                    .flex_shrink_0()
                                                    .text_xs()
                                                    .font_semibold()
                                                    .child(disk.mount.clone()),
                                            )
                                            .child(
                                                div()
                                                    .min_w_0()
                                                    .overflow_hidden()
                                                    .text_ellipsis()
                                                    .whitespace_nowrap()
                                                    .text_xs()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .child(disk.filesystem.clone()),
                                            ),
                                    )
                                    .child(
                                        div()
                                            .flex_shrink_0()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(format!(
                                                "{:.0}% · {} / {}",
                                                disk.percent,
                                                format_kib(
                                                    disk.size_kib
                                                        .saturating_sub(disk.available_kib)
                                                ),
                                                format_kib(disk.size_kib),
                                            )),
                                    ),
                            )
                            .child(
                                Progress::new(SharedString::from(format!(
                                    "monitor-disk-{:?}-{index}-{}",
                                    card.connection.id, disk.mount
                                )))
                                .color(color)
                                .value(disk.percent.clamp(0.0, 100.0) as f32),
                            )
                    }),
            )
            .into_any_element()
    }

    fn render_empty_state(&self, cx: &Context<Self>) -> AnyElement {
        let has_filter = !self.search_query.is_empty();
        v_flex()
            .w_full()
            .min_h(px(320.0))
            .items_center()
            .justify_center()
            .gap_3()
            .rounded_lg()
            .border_1()
            .border_dashed()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .size(px(52.0))
                    .flex()
                    .items_center()
                    .justify_center()
                    .rounded_full()
                    .bg(cx.theme().muted)
                    .child(
                        Icon::new(if has_filter {
                            IconName::Search
                        } else {
                            IconName::Monitor
                        })
                        .mono()
                        .with_size(IconSize::Large)
                        .text_color(cx.theme().muted_foreground),
                    ),
            )
            .child(
                div().text_base().font_semibold().child(
                    if has_filter {
                        t!("Monitor.no_results_title")
                    } else {
                        t!("Monitor.empty_title")
                    }
                    .to_string(),
                ),
            )
            .child(
                div()
                    .max_w(px(460.0))
                    .text_center()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        if has_filter {
                            t!("Monitor.no_results_description")
                        } else {
                            t!("Monitor.empty_description")
                        }
                        .to_string(),
                    ),
            )
            .into_any_element()
    }
}

impl EventEmitter<TabContentEvent> for MonitorTab {}

impl Focusable for MonitorTab {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl TabContent for MonitorTab {
    fn content_key(&self) -> &'static str {
        "Monitor"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("Monitor.title").into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::Monitor.into())
    }
}

impl Render for MonitorTab {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let enabled = monitored_connection_ids(cx);
        let indices = self.filtered_card_indices();
        let cards = indices
            .into_iter()
            .map(|index| self.render_card(index, &enabled, cx))
            .collect::<Vec<_>>();

        v_flex()
            .id("monitor-page")
            .track_focus(&self.focus_handle)
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(cx.theme().background)
            .child(self.render_header(&enabled, cx))
            .child(
                div().flex_1().min_h_0().min_w_0().overflow_hidden().child(
                    v_flex()
                        .id("monitor-scroll")
                        .size_full()
                        .min_w_0()
                        .overflow_y_scrollbar()
                        .child(
                            v_flex()
                                .w_full()
                                .min_w_0()
                                .gap_4()
                                .p_6()
                                .when(self.cards.is_empty(), |this| {
                                    this.child(self.render_empty_state(cx))
                                })
                                .when(!self.cards.is_empty() && cards.is_empty(), |this| {
                                    this.child(self.render_empty_state(cx))
                                })
                                .children(cards),
                        ),
                ),
            )
    }
}

fn monitored_connection_ids(cx: &App) -> BTreeSet<i64> {
    AppSettings::global(cx).monitored_connection_ids.clone()
}

fn card_is_monitored(card: &MonitorCard, enabled: &BTreeSet<i64>) -> bool {
    card.connection.id.is_some_and(|id| enabled.contains(&id))
}

fn monitored_cards<'a>(cards: &'a [MonitorCard], enabled: &BTreeSet<i64>) -> Vec<&'a MonitorCard> {
    cards
        .iter()
        .filter(|card| card_is_monitored(card, enabled))
        .collect()
}

fn build_monitor_config(
    repository: Option<&ConnectionRepository>,
    connection: &StoredConnection,
) -> Result<(terminal_view::SshTerminalConfig, bool), String> {
    let Some(repository) = repository else {
        return Err(t!("Monitor.error_storage").to_string());
    };
    let runtime = repository
        .resolve_runtime_connection(connection)
        .map_err(|error| format!("{error:#}"))?;
    let params = runtime
        .to_ssh_params()
        .map_err(|_| t!("Monitor.error_invalid_connection").to_string())?;
    if params.prompts_for_username()
        || (params.prompts_for_password() && params.auth_method.contains_password())
    {
        return Err(t!("Monitor.error_credentials").to_string());
    }
    let keyboard_interactive = params.keyboard_interactive_enabled();
    terminal::terminal::ssh_terminal_config_from_connection(&runtime)
        .map(|config| (config, keyboard_interactive))
        .map_err(|error| format!("{error:#}"))
}

fn card_error(card: &MonitorCard) -> Option<String> {
    match &card.state {
        MonitorCardState::Failed(error) => Some(error.clone()),
        _ => None,
    }
}

fn metric_label(icon: IconName, label: String, cx: &Context<MonitorTab>) -> AnyElement {
    h_flex()
        .items_center()
        .gap_1p5()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(Icon::new(icon).mono().with_size(IconSize::Small))
        .child(div().font_semibold().child(label))
        .into_any_element()
}

fn network_value(
    icon: IconName,
    label: String,
    value: String,
    color: Hsla,
    cx: &Context<MonitorTab>,
) -> AnyElement {
    v_flex()
        .min_w_0()
        .gap_0p5()
        .child(
            h_flex()
                .items_center()
                .gap_1()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    Icon::new(icon)
                        .mono()
                        .with_size(IconSize::Small)
                        .text_color(color),
                )
                .child(label),
        )
        .child(
            div()
                .text_base()
                .font_semibold()
                .text_color(color)
                .child(value),
        )
        .into_any_element()
}

#[derive(Clone)]
struct ChartPoint {
    label: SharedString,
    value: f64,
    ceiling: f64,
}

#[derive(Clone)]
struct NetworkChartPoint {
    label: SharedString,
    rx: f64,
    tx: f64,
}

fn render_cpu_history(values: &[f64], cx: &Context<MonitorTab>) -> AnyElement {
    if values.len() < 2 {
        return div()
            .h(px(58.0))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t!("Monitor.awaiting_history").to_string())
            .into_any_element();
    }
    let points = values
        .iter()
        .enumerate()
        .map(|(index, value)| ChartPoint {
            label: SharedString::from(index.to_string()),
            value: *value,
            ceiling: 100.0,
        })
        .collect::<Vec<_>>();
    div()
        .h(px(58.0))
        .child(
            AreaChart::new(points)
                .x(|point| point.label.clone())
                .y(|point| point.value)
                .stroke(cx.theme().chart_1)
                .fill(linear_gradient(
                    0.0,
                    linear_color_stop(cx.theme().chart_1.opacity(0.28), 1.0),
                    linear_color_stop(cx.theme().background.opacity(0.02), 0.0),
                ))
                .y(|point| point.ceiling)
                .stroke(cx.theme().chart_1.opacity(0.0))
                .fill(cx.theme().chart_1.opacity(0.0))
                .tick_margin(history_tick_margin(values.len())),
        )
        .into_any_element()
}

fn render_network_history(rx: &[f64], tx: &[f64], cx: &Context<MonitorTab>) -> AnyElement {
    if rx.len() < 2 || tx.len() < 2 {
        return div()
            .h(px(42.0))
            .flex()
            .items_center()
            .justify_center()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t!("Monitor.awaiting_history").to_string())
            .into_any_element();
    }
    let points = rx
        .iter()
        .zip(tx.iter())
        .enumerate()
        .map(|(index, (rx, tx))| NetworkChartPoint {
            label: SharedString::from(index.to_string()),
            rx: *rx,
            tx: *tx,
        })
        .collect::<Vec<_>>();
    div()
        .h(px(42.0))
        .child(
            LineChart::new(points)
                .x(|point| point.label.clone())
                .y(|point| point.rx)
                .stroke(cx.theme().chart_2)
                .y(|point| point.tx)
                .stroke(cx.theme().chart_4)
                .tick_margin(history_tick_margin(rx.len())),
        )
        .into_any_element()
}

fn history_tick_margin(point_count: usize) -> usize {
    (point_count / 6).max(1)
}

fn push_history(values: &mut Vec<f64>, value: f64) {
    values.push(value);
    if values.len() > MONITOR_HISTORY_LIMIT {
        let excess = values.len() - MONITOR_HISTORY_LIMIT;
        values.drain(0..excess);
    }
}

fn usage_color(percent: f64, cx: &Context<MonitorTab>) -> Hsla {
    if percent >= 90.0 {
        cx.theme().danger
    } else if percent >= 75.0 {
        cx.theme().warning
    } else {
        cx.theme().chart_1
    }
}

fn format_kib(value: u64) -> String {
    format_bytes(value.saturating_mul(1024))
}

fn format_bytes(value: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let value = value as f64;
    if value >= GIB {
        format!("{:.1} GiB", value / GIB)
    } else if value >= MIB {
        format!("{:.1} MiB", value / MIB)
    } else if value >= KIB {
        format!("{:.1} KiB", value / KIB)
    } else {
        format!("{value:.0} B")
    }
}

fn format_bytes_per_sec(value: f64) -> String {
    format!("{}/s", format_bytes(value.max(0.0) as u64))
}

fn format_uptime(seconds: f64) -> String {
    let seconds = seconds.max(0.0) as u64;
    let days = seconds / 86_400;
    let hours = (seconds % 86_400) / 3_600;
    let minutes = (seconds % 3_600) / 60;
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use one_core::storage::SshParams;

    #[test]
    fn history_keeps_only_the_latest_window() {
        let mut values = Vec::new();
        for value in 0..(MONITOR_HISTORY_LIMIT + 4) {
            push_history(&mut values, value as f64);
        }

        assert_eq!(values.len(), MONITOR_HISTORY_LIMIT);
        assert_eq!(values[0], 4.0);
        assert_eq!(values[values.len() - 1], 33.0);
    }

    #[test]
    fn monitored_cards_only_includes_enabled() {
        let cards = vec![
            monitor_card_for_test(1),
            monitor_card_for_test(2),
            monitor_card_for_test(3),
        ];
        let enabled = [2, 3]
            .into_iter()
            .collect::<std::collections::BTreeSet<_>>();

        let ids = monitored_cards(&cards, &enabled)
            .iter()
            .map(|card| card.connection.id.expect("card id"))
            .collect::<Vec<_>>();
        assert_eq!(ids, vec![2, 3]);
        assert!(monitored_cards(&cards, &std::collections::BTreeSet::new()).is_empty());
    }

    fn monitor_card_for_test(id: i64) -> MonitorCard {
        let params: SshParams = serde_json::from_value(serde_json::json!({
            "host": "example.com",
            "port": 22,
            "username": "root",
            "auth_method": "None",
        }))
        .expect("ssh params");
        let mut connection = StoredConnection::new_ssh(format!("host-{id}"), params, None);
        connection.id = Some(id);
        MonitorCard::new(connection)
    }

    #[test]
    fn dashboard_uses_long_lived_sessions_and_manual_refresh() {
        let source = include_str!("monitor_tab.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production source");
        assert!(source.contains("HostMonitorSession::new("));
        assert!(source.contains("session.sample().await"));
        assert!(source.contains("session.disconnect().await"));
        assert!(source.contains("this.restart_sampling(cx)"));
        assert!(!source.contains("probe_host_resource_usage("));
    }

    #[test]
    fn interval_presets_are_stable_and_ordered() {
        assert_eq!(
            RefreshInterval::ALL.map(RefreshInterval::seconds),
            [5, 15, 30]
        );
    }
}
