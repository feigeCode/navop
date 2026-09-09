use gpui::prelude::FluentBuilder as _;
use gpui::{
    ClipboardItem, FontWeight, IntoElement, ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, Icon, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use rust_i18n::t;
use ssh::KnownHost;

use crate::page::KnownHostsPage;

impl Render for KnownHostsPage {
    fn render(&mut self, _window: &mut Window, cx: &mut gpui::Context<Self>) -> impl IntoElement {
        let hosts = self.hosts.clone();
        v_flex()
            .size_full()
            .min_h_0()
            .overflow_hidden()
            .child(self.render_toolbar(cx))
            .child(
                v_flex()
                    .w_full()
                    .min_h_0()
                    .flex_1()
                    .overflow_hidden()
                    .child(self.render_header(hosts.len(), cx))
                    .child(self.render_content(hosts, cx)),
            )
    }
}

impl KnownHostsPage {
    fn render_toolbar(&self, cx: &gpui::Context<Self>) -> impl IntoElement {
        let busy = self.loading || self.importing;
        h_flex()
            .w_full()
            .min_w_0()
            .flex_shrink_0()
            .flex_wrap()
            .justify_between()
            .items_center()
            .gap_3()
            .px_4()
            .py_3()
            .border_b_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("known-hosts-scan-system")
                            .icon(IconName::Plus)
                            .label(if self.importing {
                                t!("KnownHosts.scanning").to_string()
                            } else {
                                t!("KnownHosts.scan").to_string()
                            })
                            .small()
                            .ghost()
                            .disabled(busy)
                            .on_click(cx.listener(|page, _, _, cx| page.scan_system(cx))),
                    )
                    .child(
                        Button::new("known-hosts-refresh")
                            .icon(IconName::Refresh)
                            .label(if self.loading {
                                t!("KnownHosts.loading").to_string()
                            } else {
                                t!("KnownHosts.refresh").to_string()
                            })
                            .small()
                            .ghost()
                            .disabled(busy)
                            .on_click(cx.listener(|page, _, _, cx| page.refresh(cx))),
                    ),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("KnownHosts.toolbar_help").to_string()),
            )
    }

    fn render_header(&self, count: usize, cx: &gpui::Context<Self>) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_shrink_0()
            .gap_1()
            .px_4()
            .pt_4()
            .pb_3()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::BOLD)
                            .child(t!("KnownHosts.title").to_string()),
                    )
                    .child(div().flex_1())
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("KnownHosts.count", count = count).to_string()),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("KnownHosts.source_help").to_string()),
            )
    }

    fn render_content(
        &mut self,
        hosts: Vec<KnownHost>,
        cx: &mut gpui::Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(error) = self.load_error.clone() {
            return self.render_error(error, cx).into_any_element();
        }
        if self.loading && hosts.is_empty() {
            return self.render_loading(cx).into_any_element();
        }
        if hosts.is_empty() {
            return self.render_empty(cx).into_any_element();
        }
        div()
            .w_full()
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(
                div().size_full().overflow_y_scrollbar().child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .p_2()
                        .children(hosts.into_iter().map(|host| self.render_host(host, cx))),
                ),
            )
            .into_any_element()
    }

    fn render_host(&self, host: KnownHost, cx: &gpui::Context<Self>) -> impl IntoElement {
        let identity = host.identity.clone();
        let copy_identity = identity.clone();
        let host_label = host.identity.host().to_owned();
        let port = host.identity.port();
        let key_summary = format!("{} · {}", host.algorithm, host.fingerprint);
        v_flex()
            .justify_center()
            .w_full()
            .h(gpui::rems(4.5))
            .flex_shrink_0()
            .rounded(px(12.0))
            .bg(cx.theme().background)
            .border_1()
            .border_color(cx.theme().border)
            .px_3()
            .child(
                h_flex()
                    .w_full()
                    .items_center()
                    .gap_2p5()
                    .child(
                        div()
                            .size(gpui::rems(2.375))
                            .rounded(px(9.0))
                            .border_1()
                            .border_color(cx.theme().border)
                            .bg(cx.theme().muted)
                            .flex()
                            .items_center()
                            .justify_center()
                            .child(
                                Icon::new(IconName::ServerLine)
                                    .text_color(cx.theme().muted_foreground),
                            ),
                    )
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_0p5()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_1_5()
                                    .items_center()
                                    .child(
                                        div()
                                            .text_sm()
                                            .font_weight(FontWeight::BOLD)
                                            .truncate()
                                            .child(host_label),
                                    )
                                    .when(port != 22, |row| {
                                        row.child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .child(format!(":{port}")),
                                        )
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .truncate()
                                    .child(key_summary),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .child(
                                Button::new(format!("known-host-copy-{}", host.fingerprint))
                                    .icon(IconName::Copy)
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(move |_, _, _, cx| {
                                        cx.write_to_clipboard(ClipboardItem::new_string(
                                            copy_identity.to_string(),
                                        ));
                                    })),
                            )
                            .child(
                                Button::new(format!("known-host-remove-{}", host.fingerprint))
                                    .icon(IconName::Remove)
                                    .small()
                                    .ghost()
                                    .on_click(cx.listener(move |page, _, _, cx| {
                                        page.remove_host(identity.clone(), cx);
                                    })),
                            ),
                    ),
            )
    }

    fn render_loading(&self, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_1()
            .items_center()
            .justify_center()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(t!("KnownHosts.loading").to_string())
    }

    fn render_error(&self, error: String, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_1()
            .gap_2()
            .items_center()
            .justify_center()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().warning)
                    .child(t!("KnownHosts.load_error").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(error),
            )
    }

    fn render_empty(&self, cx: &gpui::App) -> impl IntoElement {
        v_flex()
            .w_full()
            .flex_1()
            .gap_3()
            .items_center()
            .justify_center()
            .px_8()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("KnownHosts.empty").to_string()),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("KnownHosts.empty_help").to_string()),
            )
    }
}
