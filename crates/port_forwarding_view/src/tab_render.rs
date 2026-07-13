use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, Styled, Window, div,
    prelude::FluentBuilder as _, px,
};
use gpui_component::button::{Button, ButtonVariants};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt, h_flex, scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;

use crate::tab::PortForwardingTab;
use crate::tab_state::PortForwardingTabState;

const ACTIVITY_MAX_HEIGHT: f32 = 220.0;

pub(crate) fn render_tab(
    tab: &mut PortForwardingTab,
    _window: &mut Window,
    cx: &mut Context<PortForwardingTab>,
) -> impl IntoElement {
    let status = status_label(&tab.state);
    let running = matches!(tab.state, PortForwardingTabState::Running { .. });
    let retryable = tab.state.can_retry_start();
    let stop_failed = tab.state.tunnel_may_be_running();
    v_flex()
        .track_focus(&tab.focus_handle)
        .size_full()
        .overflow_hidden()
        .bg(cx.theme().background)
        .child(
            div().flex_1().min_h_0().p_6().overflow_y_scrollbar().child(
                v_flex()
                    .w_full()
                    .max_w(px(980.0))
                    .mx_auto()
                    .gap_4()
                    .child(render_header(
                        tab,
                        status,
                        running || stop_failed,
                        retryable,
                        cx.entity(),
                        cx,
                    ))
                    .child(render_route(tab, cx))
                    .child(render_info(tab, cx))
                    .child(render_events(tab, cx)),
            ),
        )
}

fn render_header(
    tab: &PortForwardingTab,
    status: String,
    running: bool,
    retryable: bool,
    view: gpui::Entity<PortForwardingTab>,
    cx: &mut Context<PortForwardingTab>,
) -> impl IntoElement {
    let stop_view = view.clone();
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .child(
            h_flex()
                .gap_3()
                .child(
                    div()
                        .size(px(44.0))
                        .rounded_lg()
                        .bg(cx.theme().primary.opacity(0.14))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(IconName::PortForwardingColor.color().with_size(Size::Large)),
                )
                .child(
                    v_flex()
                        .child(div().text_lg().font_semibold().child(tab.name.clone()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(kind_label(tab)),
                        ),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .child(
                    div()
                        .px_3()
                        .py_1()
                        .rounded_full()
                        .bg(status_color(&tab.state, cx))
                        .text_sm()
                        .child(status),
                )
                .when(running, |this| {
                    this.child(
                        Button::new("stop-port-forwarding")
                            .label(t!("PortForwardingTab.stop").to_string())
                            .danger()
                            .on_click(move |_, _, cx| {
                                stop_view.update(cx, |tab, cx| tab.stop_forwarding(cx));
                            }),
                    )
                })
                .when(retryable, |this| {
                    this.child(
                        Button::new("retry-port-forwarding")
                            .label(t!("PortForwardingTab.retry").to_string())
                            .primary()
                            .on_click(move |_, _, cx| {
                                view.update(cx, |tab, cx| tab.retry_forwarding(cx));
                            }),
                    )
                }),
        )
}

fn render_route(tab: &PortForwardingTab, cx: &mut Context<PortForwardingTab>) -> impl IntoElement {
    v_flex()
        .p_5()
        .gap_4()
        .rounded_xl()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("PortForwardingTab.route").to_string()),
        )
        .child(
            h_flex()
                .w_full()
                .justify_between()
                .items_center()
                .child(endpoint(
                    &t!("PortForwardingTab.local_endpoint"),
                    &tab.bind_label,
                    false,
                    cx,
                ))
                .child(Icon::new(IconName::ArrowRight).with_size(Size::Medium))
                .child(endpoint(
                    &t!("PortForwardingTab.ssh_tunnel"),
                    &tab.ssh_label,
                    true,
                    cx,
                ))
                .child(Icon::new(IconName::ArrowRight).with_size(Size::Medium))
                .child(endpoint(
                    &t!("PortForwardingTab.remote_target"),
                    &tab.target_label,
                    false,
                    cx,
                )),
        )
}

fn endpoint(
    label: &str,
    value: &str,
    emphasized: bool,
    cx: &mut Context<PortForwardingTab>,
) -> impl IntoElement {
    v_flex()
        .min_w(px(210.0))
        .p_4()
        .gap_1()
        .rounded_lg()
        .when(emphasized, |this| this.bg(cx.theme().primary.opacity(0.1)))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().font_semibold().child(value.to_string()))
}

fn render_info(tab: &PortForwardingTab, cx: &mut Context<PortForwardingTab>) -> impl IntoElement {
    h_flex()
        .w_full()
        .gap_3()
        .child(info_card(
            &t!("PortForwardingTab.ssh_server"),
            &tab.ssh_label,
            cx,
        ))
        .child(info_card(
            &t!("PortForwardingTab.bind_address"),
            &actual_bind(tab),
            cx,
        ))
        .child(info_card(&t!("PortForwardingTab.uptime"), &uptime(tab), cx))
        .child(info_card(
            &t!("PortForwardingTab.status"),
            &state_detail(&tab.state),
            cx,
        ))
}

fn info_card(label: &str, value: &str, cx: &mut Context<PortForwardingTab>) -> impl IntoElement {
    v_flex()
        .flex_1()
        .p_4()
        .gap_1()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().secondary)
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.to_string()),
        )
        .child(div().font_medium().child(value.to_string()))
}

fn render_events(tab: &PortForwardingTab, cx: &mut Context<PortForwardingTab>) -> impl IntoElement {
    v_flex()
        .p_4()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .text_sm()
                .font_semibold()
                .child(t!("PortForwardingTab.activity").to_string()),
        )
        .child(
            v_flex()
                .gap_2()
                .max_h(px(ACTIVITY_MAX_HEIGHT))
                .overflow_y_scrollbar()
                .children(tab.events.iter().map(|event| {
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(event.clone())
                })),
        )
}

fn actual_bind(tab: &PortForwardingTab) -> String {
    match &tab.state {
        PortForwardingTabState::Running { local_addr } => local_addr.to_string(),
        _ => tab.bind_label.clone(),
    }
}

fn uptime(tab: &PortForwardingTab) -> String {
    tab.started_at
        .map(|started| format!("{}s", started.elapsed().as_secs()))
        .unwrap_or_else(|| "--".to_string())
}

fn status_label(state: &PortForwardingTabState) -> String {
    match state {
        PortForwardingTabState::Starting => t!("PortForwardingTab.starting"),
        PortForwardingTabState::Running { .. } => t!("PortForwardingTab.running"),
        PortForwardingTabState::Stopping => t!("PortForwardingTab.stopping"),
        PortForwardingTabState::Failed { .. } => t!("PortForwardingTab.failed"),
        PortForwardingTabState::Stopped => t!("PortForwardingTab.stopped"),
    }
    .to_string()
}

fn kind_label(tab: &PortForwardingTab) -> String {
    match tab.kind {
        one_core::storage::PortForwardingKind::Local => t!("PortForwardingTab.local_title"),
        one_core::storage::PortForwardingKind::Dynamic => t!("PortForwardingTab.dynamic_title"),
    }
    .to_string()
}

fn state_detail(state: &PortForwardingTabState) -> String {
    match state {
        PortForwardingTabState::Failed { error, .. } => error.clone(),
        _ => status_label(state),
    }
}

fn status_color(state: &PortForwardingTabState, cx: &Context<PortForwardingTab>) -> gpui::Hsla {
    match state {
        PortForwardingTabState::Running { .. } => cx.theme().success.opacity(0.16),
        PortForwardingTabState::Failed { .. } => cx.theme().danger.opacity(0.16),
        _ => cx.theme().primary.opacity(0.14),
    }
}
