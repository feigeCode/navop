use gpui::prelude::FluentBuilder;
use gpui::{
    App, ColorExt, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable, Size,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    form::{field, v_form},
    h_flex,
    input::Input,
    scroll::ScrollableElement,
    select::Select,
    tab::{Tab, TabBar},
    v_flex,
};
use rust_i18n::t;

use super::ExtensionConnectionForm;

impl Focusable for ExtensionConnectionForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl ExtensionConnectionForm {
    /// manifest 声明的表单页签数量
    fn manifest_tab_count(&self, cx: &App) -> usize {
        self.fields.read(cx).tab_count()
    }

    /// 统一页签栏:manifest 页签(来自扩展声明,已中文化)+ 宿主"备注"页签
    fn render_tab_bar(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let tab_count = self.manifest_tab_count(cx);
        let mut bar = TabBar::new("extension-connection-tabs")
            .with_size(Size::Large)
            .underline()
            .selected_index(self.active_tab)
            .on_click(cx.listener(move |this, index: &usize, _window, cx| {
                this.active_tab = *index;
                let tab_count = this.fields.read(cx).tab_count();
                let fields_index = (*index).min(tab_count.saturating_sub(1));
                this.fields
                    .update(cx, |form, _| form.set_active_tab(fields_index));
                cx.notify();
            }));
        for index in 0..tab_count {
            if let Some(label) = self.fields.read(cx).tab_label(index) {
                bar = bar.child(Tab::new().label(label));
            }
        }
        div()
            .flex()
            .justify_center()
            .child(bar.child(Tab::new().label(t!("ConnectionForm.remark").to_string())))
    }

    fn render_tab_content(&mut self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.active_tab < self.manifest_tab_count(cx) {
            self.render_manifest_tab(cx).into_any_element()
        } else {
            self.render_remark_tab(cx).into_any_element()
        }
    }

    /// manifest 页签内容:首页签附带名称/工作空间/团队,其余页签仅扩展声明字段
    fn render_manifest_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let is_first_tab = self.active_tab == 0;
        v_flex()
            .gap_4()
            .min_h(px(250.))
            .when(is_first_tab, |this| {
                this.child(
                    v_form().columns(1).child(
                        field()
                            .label(t!("ConnectionForm.name").to_string())
                            .required(true)
                            .child(Input::new(&self.name).w_full()),
                    ),
                )
            })
            .child(self.fields.clone())
            .when(is_first_tab, |this| {
                this.child(
                    v_form().columns(1).child(
                        field()
                            .label(t!("ConnectionForm.workspace").to_string())
                            .child(Select::new(&self.workspace).w_full()),
                    ),
                )
            })
            .when(
                is_first_tab && connection_form::team::team_management_enabled(cx),
                |this| {
                    this.child(
                        v_form().columns(1).child(
                            field()
                                .label(connection_form::team::team_label())
                                .child(Select::new(&self.team).w_full()),
                        ),
                    )
                },
            )
    }

    /// 备注页:备注 + 云端同步(与数据库表单的备注页一致)
    fn render_remark_tab(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let show_sync = connection_form::team::connection_sync_controls_visible_in(cx);
        v_flex()
            .gap_4()
            .min_h(px(250.))
            .child(
                v_form().columns(1).child(
                    field()
                        .label(t!("ConnectionForm.remark").to_string())
                        .child(Input::new(&self.remark).w_full()),
                ),
            )
            .when(show_sync, |this| {
                this.child(
                    v_form().columns(1).child(
                        field()
                            .label(t!("ConnectionForm.cloud_sync").to_string())
                            .child(
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
            })
    }

    /// 测试结果横幅(样式对齐数据库连接窗口)
    fn render_test_banner(
        &mut self,
        status: Result<(), String>,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let is_success = status.is_ok();
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
                    .child(match status {
                        Ok(()) => t!("ConnectionForm.test_success").to_string(),
                        Err(error) => error,
                    }),
            )
            .child(
                Button::new("extension-clear-test-result")
                    .xsmall()
                    .ghost()
                    .icon(IconName::Close)
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.test_result.update(cx, |result, cx| {
                            *result = None;
                            cx.notify();
                        });
                    })),
            )
    }

    /// 页脚:取消/测试/确定(对齐数据库连接窗口)
    fn render_footer(&mut self, testing: bool, cx: &mut Context<Self>) -> impl IntoElement {
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
                    .on_click(cx.listener(|_, _, window, _cx| {
                        window.remove_window();
                    })),
            )
            .child(
                Button::new("extension-connection-test")
                    .small()
                    .outline()
                    .label(if testing {
                        t!("Connection.testing").to_string()
                    } else {
                        t!("Connection.test").to_string()
                    })
                    .loading(testing)
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
}

impl Render for ExtensionConnectionForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let testing = *self.is_testing.read(cx);
        let status = self.test_result.read(cx).clone();

        v_flex()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(self.render_tab_bar(cx))
                    .child(self.render_tab_content(cx)),
            )
            .when_some(status, |el, status| {
                el.child(self.render_test_banner(status, cx))
            })
            .child(self.render_footer(testing, cx))
    }
}
