use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    form::{field, v_form},
    h_flex,
    input::Input,
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
        let status = self.test_result.read(cx).clone();
        v_flex()
            .size_full()
            .gap_4()
            .p_4()
            .child(
                v_form().columns(1).child(
                    field()
                        .label("Name")
                        .required(true)
                        .child(Input::new(&self.name).w_full()),
                ),
            )
            .child(self.fields.clone())
            .child(
                v_form().columns(1).child(
                    field()
                        .label("Workspace")
                        .child(Select::new(&self.workspace).w_full()),
                ),
            )
            .when(connection_form::team::team_management_enabled(cx), |this| {
                this.child(
                    v_form().columns(1).child(
                        field()
                            .label(connection_form::team::team_label())
                            .child(Select::new(&self.team).w_full()),
                    ),
                )
            })
            .when(
                connection_form::team::connection_sync_controls_visible_in(cx),
                |this| {
                    this.child(
                        v_form().columns(1).child(
                            field()
                                .label("Remark")
                                .child(Input::new(&self.remark).w_full()),
                        ),
                    )
                },
            )
            .child(
                v_form().columns(1).child(
                    field().label("Sync").child(
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
            .when_some(status, |el, status| el.child(status_message(status, cx)))
            .child(action_buttons(testing, cx))
    }
}

fn status_message(status: Result<(), String>, cx: &App) -> impl IntoElement {
    div()
        .text_sm()
        .text_color(if status.is_ok() {
            cx.theme().success
        } else {
            cx.theme().danger
        })
        .child(match status {
            Ok(()) => "Connection successful".into(),
            Err(error) => error,
        })
}

fn action_buttons(testing: bool, cx: &mut Context<ExtensionConnectionForm>) -> impl IntoElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("extension-connection-test")
                .label("Test")
                .loading(testing)
                .disabled(testing)
                .on_click(cx.listener(|this, _, _, cx| this.on_test(cx))),
        )
        .child(
            Button::new("extension-connection-save")
                .primary()
                .label("Save")
                .disabled(testing)
                .on_click(cx.listener(|this, _, window, cx| this.on_save(window, cx))),
        )
}
