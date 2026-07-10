use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, div,
    px,
};
use gpui_component::{
    ActiveTheme, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    radio::Radio,
    scroll::ScrollableElement,
    select::Select,
    v_flex,
};
use rust_i18n::t;

use super::RemoteDesktopFormWindow;

impl RemoteDesktopFormWindow {
    fn render_form_row(&self, label: String, child: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(100.0)).text_sm().text_right().child(label))
            .child(div().flex_1().child(child))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_name").to_string(),
                Input::new(&self.name_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_host").to_string(),
                Input::new(&self.host_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_port").to_string(),
                Input::new(&self.port_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_username").to_string(),
                Input::new(&self.username_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_password").to_string(),
                Input::new(&self.password_input).mask_toggle(),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_domain").to_string(),
                Input::new(&self.domain_input),
            ))
            .child(self.render_proxy_section(cx))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_workspace").to_string(),
                Select::new(&self.workspace_select).w_full(),
            ))
            .child(
                self.render_form_row(
                    t!("RemoteDesktopForm.label_team").to_string(),
                    h_flex()
                        .gap_2()
                        .child(Select::new(&self.team_select).w_full())
                        .child(
                            Button::new("sync-remote-desktop-teams")
                                .icon(IconName::Refresh)
                                .ghost()
                                .tooltip(t!("Home.sync_tooltip"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.request_team_sync(window, cx);
                                })),
                        ),
                ),
            )
            .child(self.render_read_only_row(cx))
            .child(self.render_sync_row(cx))
    }

    fn render_proxy_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    t!("RemoteDesktopForm.label_proxy").to_string(),
                    Checkbox::new("remote-desktop-proxy-enabled")
                        .checked(self.proxy_enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_enabled = !this.proxy_enabled;
                            cx.notify();
                        })),
                ),
            )
            .when(self.proxy_enabled, |form| {
                form.child(self.render_proxy_type_row(cx))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_host").to_string(),
                        Input::new(&self.proxy_host_input),
                    ))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_port").to_string(),
                        Input::new(&self.proxy_port_input),
                    ))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_username").to_string(),
                        Input::new(&self.proxy_username_input),
                    ))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_password").to_string(),
                        Input::new(&self.proxy_password_input).mask_toggle(),
                    ))
            })
    }

    fn render_proxy_type_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_form_row(
            t!("RemoteDesktopForm.label_proxy_type").to_string(),
            h_flex()
                .gap_4()
                .child(
                    Radio::new("remote-desktop-proxy-socks5")
                        .label("SOCKS5")
                        .checked(self.proxy_type == one_core::storage::ProxyType::Socks5)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_type = one_core::storage::ProxyType::Socks5;
                            cx.notify();
                        })),
                )
                .child(
                    Radio::new("remote-desktop-proxy-http")
                        .label("HTTP CONNECT")
                        .checked(self.proxy_type == one_core::storage::ProxyType::Http)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_type = one_core::storage::ProxyType::Http;
                            cx.notify();
                        })),
                ),
        )
    }

    fn render_read_only_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_form_row(
            t!("RemoteDesktopForm.label_read_only").to_string(),
            Checkbox::new("remote-desktop-read-only")
                .checked(self.read_only)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.read_only = !this.read_only;
                    cx.notify();
                })),
        )
    }

    fn render_sync_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = t!("ConnectionForm.cloud_sync").to_string();
        let desc = t!("ConnectionForm.cloud_sync_desc").to_string();
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(100.0)).text_sm().text_right().child(label))
            .child(
                div().flex_1().child(
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new("remote-desktop-sync-enabled")
                                .checked(self.sync_enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sync_enabled = !this.sync_enabled;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(desc),
                        ),
                ),
            )
    }
}

impl Focusable for RemoteDesktopFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RemoteDesktopFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .justify_center()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .size_full()
                            .p_3()
                            .overflow_y_scrollbar()
                            .child(self.render_body(cx)),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    h_flex()
                        .justify_center()
                        .pb_2()
                        .child(div().text_sm().text_color(cx.theme().danger).child(error)),
                )
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .px_6()
                    .py_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel-remote-desktop")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(cx.listener(|_, _, window, _| window.remove_window())),
                    )
                    .child(
                        Button::new("save-remote-desktop")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
                    ),
            )
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_desktop_form_keeps_scrollbar_inside_explicit_flex_clip_boundary() {
        let source = include_str!("view.rs");
        let render_source = source.split("#[cfg(test)]").next().unwrap();

        assert!(render_source.contains(".min_h_0()"));
        assert!(render_source.contains(".min_w_0()"));
        assert!(render_source.contains(".overflow_hidden()"));
        assert!(render_source.matches(".size_full()").count() >= 2);
        assert!(render_source.contains(".overflow_y_scrollbar()"));
    }
}
