use connection_form::team::team_label;
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FocusHandle, Focusable, InteractiveElement, IntoElement, ParentElement, Render,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    select::Select,
    v_flex,
};
use one_core::storage::PortForwardingKind;
use rust_i18n::t;

use crate::form_window::PortForwardingFormWindow;

impl Render for PortForwardingFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let kind = self
            .kind_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or_default();

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(self.render_content(kind, cx))
            .when_some(self.validation_error.clone(), |this, error| {
                this.child(
                    h_flex()
                        .justify_center()
                        .pb_2()
                        .child(div().text_sm().text_color(cx.theme().danger).child(error)),
                )
            })
            .child(self.render_footer(cx))
    }
}

impl Focusable for PortForwardingFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl PortForwardingFormWindow {
    pub(super) fn render_row(
        &self,
        label: impl Into<String>,
        child: impl IntoElement,
    ) -> impl IntoElement {
        let label = label.into();
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(110.0)).text_sm().text_right().child(label))
            .child(div().flex_1().child(child))
    }

    fn render_content(&self, kind: PortForwardingKind, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("forwarding-port-form-content")
            .flex_1()
            .p_3()
            .overflow_y_scroll()
            .child(
                v_flex()
                    .gap_2()
                    .child(self.render_row(
                        t!("PortForwarding.name").to_string(),
                        Input::new(&self.name_input),
                    ))
                    .child(self.render_ssh_select())
                    .child(self.render_row(
                        t!("PortForwarding.kind").to_string(),
                        Select::new(&self.kind_select).w_full(),
                    ))
                    .child(self.render_row(
                        t!("PortForwarding.bind_host").to_string(),
                        Input::new(&self.bind_host_input),
                    ))
                    .child(self.render_row(
                        t!("PortForwarding.bind_port").to_string(),
                        Input::new(&self.bind_port_input),
                    ))
                    .when(kind == PortForwardingKind::Local, |form| {
                        form.child(self.render_row(
                            t!("PortForwarding.target_host").to_string(),
                            Input::new(&self.target_host_input),
                        ))
                        .child(self.render_row(
                            t!("PortForwarding.target_port").to_string(),
                            Input::new(&self.target_port_input),
                        ))
                    })
                    .child(self.render_row(
                        t!("PortForwarding.workspace").to_string(),
                        Select::new(&self.workspace_select).w_full(),
                    ))
                    .child(self.render_row(team_label(), Select::new(&self.team_select).w_full()))
                    .child(self.render_sync_row(cx))
                    .child(self.render_row(
                        t!("PortForwarding.remark").to_string(),
                        Input::new(&self.remark_input),
                    )),
            )
    }

    fn render_ssh_select(&self) -> impl IntoElement {
        self.render_row(
            t!("PortForwarding.ssh_connection").to_string(),
            Select::new(&self.ssh_select)
                .placeholder(t!("PortForwarding.ssh_connection_placeholder"))
                .w_full(),
        )
    }

    fn render_sync_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_row(
            t!("ConnectionForm.cloud_sync").to_string(),
            h_flex()
                .gap_2()
                .child(
                    Checkbox::new("forwarding-sync-enabled")
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
                        .child(t!("ConnectionForm.cloud_sync_desc").to_string()),
                ),
        )
    }

    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement {
        h_flex()
            .justify_end()
            .gap_2()
            .px_6()
            .py_4()
            .border_t_1()
            .border_color(cx.theme().border)
            .child(
                Button::new("forwarding-cancel")
                    .small()
                    .label(t!("Common.cancel").to_string())
                    .on_click(cx.listener(|_, _, window, _| window.remove_window())),
            )
            .child(
                Button::new("forwarding-ok")
                    .small()
                    .primary()
                    .label(t!("Common.ok").to_string())
                    .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
            )
    }
}
