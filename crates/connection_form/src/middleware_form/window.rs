//! 通用中间件连接表单窗口外壳
//!
//! 布局与 `db_view::ConnectionFormWindow` 一致:滚动表单区 + 内联测试
//! 结果条 + 取消/测试/确定按钮。各中间件通过 `MiddlewareFormWindowConfig`
//! 提供适配器与标签页配置。

use std::sync::Arc;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, ColorExt, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IconName, Sizable,
    button::{Button, ButtonVariants as _},
    h_flex,
    scroll::ScrollableElement,
    v_flex,
};
use one_core::cloud_sync::TeamOption;
use one_core::connection_notifier::{ConnectionDataEvent, emit_connection_event};
use one_core::storage::StoredConnection;
use rust_i18n::t;

use super::adapter::{MiddlewareFormAdapter, MiddlewareFormSavedCallback};
use super::declarative::TabGroup;
use super::form::{MiddlewareConnectionForm, MiddlewareFormConfig, MiddlewareFormEvent};

/// 中间件表单窗口配置
pub struct MiddlewareFormWindowConfig {
    /// 中间件适配器(决定参数映射与测试连接)
    pub adapter: Arc<dyn MiddlewareFormAdapter>,
    /// 声明式标签页配置
    pub tab_groups: Vec<TabGroup>,
    /// 正在编辑的连接(`None` 表示新建)
    pub editing_connection: Option<StoredConnection>,
    /// 预填连接(不进入编辑模式)
    pub initial_connection: Option<StoredConnection>,
    /// 保存成功回调
    pub on_saved: Option<MiddlewareFormSavedCallback>,
    pub workspaces: Vec<one_core::storage::Workspace>,
    pub teams: Vec<TeamOption>,
    pub ssh_connections: Vec<StoredConnection>,
}

impl MiddlewareFormWindowConfig {
    pub fn is_editing(&self) -> bool {
        self.editing_connection.is_some()
    }

    fn connection_to_load(&self) -> Option<&StoredConnection> {
        self.editing_connection
            .as_ref()
            .or(self.initial_connection.as_ref())
    }
}

/// 中间件连接表单窗口
pub struct MiddlewareFormWindow {
    focus_handle: FocusHandle,
    form: Entity<MiddlewareConnectionForm>,
    is_edit: bool,
    on_saved: Option<MiddlewareFormSavedCallback>,
}

impl MiddlewareFormWindow {
    pub fn new(
        config: MiddlewareFormWindowConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let is_edit = config.is_editing();
        let on_saved = config.on_saved.clone();

        let form = cx.new(|cx| {
            MiddlewareConnectionForm::new(
                config.adapter.clone(),
                MiddlewareFormConfig {
                    tab_groups: config.tab_groups.clone(),
                },
                window,
                cx,
            )
        });

        form.update(cx, |form, cx| {
            form.set_workspaces(config.workspaces.clone(), window, cx);
            form.set_teams(config.teams.clone(), window, cx);
            form.set_ssh_connections(config.ssh_connections.clone(), window, cx);
        });

        if let Some(conn) = config.connection_to_load() {
            let is_editing = config.is_editing();
            form.update(cx, |form, cx| {
                if is_editing {
                    form.load_connection(conn, window, cx);
                } else {
                    form.load_initial_connection(conn, window, cx);
                }
            });
        }

        let on_saved_callback = on_saved.clone();
        cx.subscribe_in(
            &form,
            window,
            move |_window, _form, event: &MiddlewareFormEvent, window, cx| match event {
                MiddlewareFormEvent::Saved(conn) => {
                    let event = if is_edit {
                        ConnectionDataEvent::ConnectionUpdated {
                            connection: conn.as_ref().clone(),
                        }
                    } else {
                        ConnectionDataEvent::ConnectionCreated {
                            connection: conn.as_ref().clone(),
                        }
                    };
                    emit_connection_event(event, cx);
                    if let Some(callback) = on_saved_callback.as_ref() {
                        callback(conn.as_ref().clone(), cx);
                    }
                    window.remove_window();
                }
                MiddlewareFormEvent::SaveError(_) => {}
            },
        )
        .detach();

        Self {
            focus_handle: cx.focus_handle(),
            form,
            is_edit,
            on_saved,
        }
    }

    fn on_test(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.trigger_test_connection(cx);
        });
    }

    fn on_save(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.save_connection(cx);
        });
    }

    fn on_clear_test_result(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.clear_test_result(cx);
        });
    }

    fn on_cancel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.form.update(cx, |form, cx| {
            form.trigger_cancel(cx);
        });
        window.remove_window();
    }

    /// 支持保存并继续(由调用方通过 `on_saved` 启用)
    fn supports_save_and_continue(&self) -> bool {
        self.on_saved.is_some()
    }

    fn on_save_and_continue(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        // 中间件表单保存成功后窗口随 Saved 事件关闭;
        // on_saved 回调可用于串联后续流程,此处复用保存入口。
        self.form.update(cx, |form, cx| {
            form.save_connection(cx);
        });
    }

    #[allow(dead_code)] // 预留给需要感知编辑态的调用方
    fn is_editing(&self) -> bool {
        self.is_edit
    }
}

impl Focusable for MiddlewareFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for MiddlewareFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let is_testing = self.form.read(cx).is_testing(cx);
        let test_result_msg = self.form.read(cx).test_result_msg(cx);

        v_flex()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .p_4()
                    .overflow_y_scrollbar()
                    .child(self.form.clone()),
            )
            .when_some(test_result_msg, |this, msg| {
                let is_success = msg.starts_with('✓');
                this.child(
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
                            Button::new("middleware-clear-test-result")
                                .xsmall()
                                .ghost()
                                .icon(IconName::Close)
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_clear_test_result(window, cx);
                                })),
                        ),
                )
            })
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("test")
                            .small()
                            .outline()
                            .label(if is_testing {
                                t!("MiddlewareForm.testing").to_string()
                            } else {
                                t!("MiddlewareForm.test").to_string()
                            })
                            .disabled(is_testing)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_test(window, cx);
                            })),
                    )
                    .when(self.supports_save_and_continue(), |this| {
                        this.child(
                            Button::new("middleware-save-continue")
                                .small()
                                .outline()
                                .label(t!("MiddlewareForm.save_and_continue").to_string())
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.on_save_and_continue(window, cx);
                                })),
                        )
                    })
                    .child(
                        Button::new("ok")
                            .small()
                            .primary()
                            .label(t!("Common.ok").to_string())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.on_save(window, cx);
                            })),
                    ),
            )
    }
}
