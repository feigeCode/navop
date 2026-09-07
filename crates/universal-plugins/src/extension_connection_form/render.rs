use gpui::prelude::FluentBuilder;
use gpui::{
    App, Axis, ColorExt, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled,
    Window, div, px,
};
use rust_i18n::t;
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    form::{field, v_form},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    select::Select,
    v_flex,
};

use super::ExtensionConnectionForm;

impl Focusable for ExtensionConnectionForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ExtensionConnectionForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let testing = *self.is_testing.read(cx);
        let result_msg = self.test_result_msg(cx);
        v_flex()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(self.render_fields(cx)),
            )
            .when_some(result_msg, |this, msg| this.child(result_bar(msg, cx)))
            .child(action_buttons(testing, cx))
    }
}

impl ExtensionConnectionForm {
    fn render_fields(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                v_form()
                    .layout(Axis::Horizontal)
                    .columns(1)
                    .label_width(px(120.))
                    .child(
                        field()
                            .label(t!("ExtensionConnectionForm.name").to_string())
                            .required(true)
                            .items_center()
                            .child(Input::new(&self.name).w_full()),
                    ),
            )
            .child(self.fields.clone())
            .child(
                v_form()
                    .layout(Axis::Horizontal)
                    .columns(1)
                    .label_width(px(120.))
                    .child(
                        field()
                            .label(t!("ConnectionForm.workspace").to_string())
                            .items_center()
                            .child(Select::new(&self.workspace).w_full()),
                    ),
            )
            .when(connection_form::team::team_management_enabled(cx), |this| {
                this.child(
                    v_form()
                        .layout(Axis::Horizontal)
                        .columns(1)
                        .label_width(px(120.))
                        .child(
                            field()
                                .label(connection_form::team::team_label())
                                .items_center()
                                .child(Select::new(&self.team).w_full()),
                        ),
                )
            })
            .when(
                connection_form::team::connection_sync_controls_visible_in(cx),
                |this| {
                    this.child(
                        v_form()
                            .layout(Axis::Horizontal)
                            .columns(1)
                            .label_width(px(120.))
                            .child(
                                field()
                                    .label(t!("ConnectionForm.remark").to_string())
                                    .items_center()
                                    .child(Input::new(&self.remark).w_full()),
                            ),
                    )
                },
            )
            .child(
                v_form()
                    .layout(Axis::Horizontal)
                    .columns(1)
                    .label_width(px(120.))
                    .child(
                        field().label(t!("ConnectionForm.cloud_sync").to_string()).items_center().child(
                            Checkbox::new("extension-connection-sync")
                                .checked(*self.sync_enabled.read(cx))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sync_enabled.update(cx, |enabled, cx| {
                                        *enabled = !*enabled;
                                        cx.notify();
                                    });
                                })),
                        ),
                    ),
            )
    }
}

fn result_bar(msg: String, cx: &mut Context<ExtensionConnectionForm>) -> impl IntoElement {
    let is_success = msg.starts_with('✓');
    h_flex()
        .items_start()
        .gap_2()
        .mx_4()
        .mb_2()
        .px_3()
        .py_2()
        .rounded_md()
        .bg(if is_success {
            cx.theme().success.opacity(0.12)
        } else {
            cx.theme().danger.opacity(0.12)
        })
        .text_color(if is_success {
            cx.theme().success
        } else {
            cx.theme().danger
        })
        .child(
            div()
                .flex_1()
                .min_w_0()
                .max_h(px(96.0))
                .overflow_y_scrollbar()
                .text_sm()
                .child(msg),
        )
        .child(
            Button::new("extension-connection-clear-test-result")
                .xsmall()
                .ghost()
                .icon(IconName::Close)
                .on_click(cx.listener(|this, _, _, cx| this.on_clear_test_result(cx))),
        )
}

fn action_buttons(testing: bool, cx: &mut Context<ExtensionConnectionForm>) -> impl IntoElement {
    h_flex()
        .flex_shrink_0()
        .justify_end()
        .gap_2()
        .p_4()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            Button::new("extension-connection-cancel")
                .small()
                .label(t!("Common.cancel").to_string())
                .on_click(cx.listener(|this, _, window, cx| this.on_cancel(window, cx))),
        )
        .child(
            Button::new("extension-connection-test")
                .small()
                .outline()
                .label(if testing {
                    t!("ConnectionForm.testing").to_string()
                } else {
                    t!("ConnectionForm.test").to_string()
                })
                .disabled(testing)
                .on_click(cx.listener(|this, _, _, cx| this.on_test(cx))),
        )
        .child(
            Button::new("extension-connection-save")
                .small()
                .primary()
                .label(t!("Common.ok").to_string())
                .disabled(testing)
                .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
        )
}
