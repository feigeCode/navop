use std::collections::HashSet;

use gpui::prelude::FluentBuilder;
use gpui::{
    App, Axis, Context, FocusHandle, Focusable, InteractiveElement as _, IntoElement,
    ParentElement, Render, Styled, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, Sizable, Size,
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
use one_assets::IconName;
use rust_i18n::t;

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
        // 页签由本表单托管,引擎只渲染当前页签字段(与 DB/中间件表单一致:
        // TabBar 在最上,名称作为首个页签的第一个字段)。
        self.fields.update(cx, |fields, _| {
            fields.host_content(self.active_tab, HashSet::new())
        });

        let tabs = &self.contribution.form.tabs;
        let mut content = v_flex().gap_4();
        if tabs.len() > 1 {
            content = content.child(self.render_tab_bar(cx));
        }
        if self.active_tab == 0 {
            content = content.child(
                div()
                    .id("extension-connection-name-row")
                    .debug_selector(|| "extension-connection-name-row".to_string())
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
                    ),
            );
        }
        content
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
                        field()
                            .label(t!("ConnectionForm.cloud_sync").to_string())
                            .items_center()
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
    }

    fn render_tab_bar(&self, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("extension-connection-tabs-bar")
            .debug_selector(|| "extension-connection-tabs-bar".to_string())
            .flex()
            .justify_center()
            .child(
                TabBar::new("extension-connection-tabs")
                    .with_size(Size::Large)
                    .underline()
                    .selected_index(self.active_tab)
                    .on_click(cx.listener(|this, index: &usize, _, cx| {
                        this.active_tab = *index;
                        cx.notify();
                    }))
                    .children(
                        self.contribution
                            .form
                            .tabs
                            .iter()
                            .map(|tab| Tab::new().label(tab.label.clone())),
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use extension_runtime::RegisteredResourceConnectionContribution;
    use extension_runtime::extension::manifest::{
        ResourceConnectionFieldType, ResourceConnectionForm, ResourceConnectionFormField,
        ResourceConnectionFormTab,
    };
    use gpui::{TestAppContext, VisualTestContext, px};
    use one_core::settings::AppSettings;

    use super::super::ExtensionConnectionFormConfig;
    use super::ExtensionConnectionForm;

    /// 两个页签 + 一个常规字段,复刻 MQTT 这类复合扩展的连接表单形状。
    fn two_tab_contribution() -> RegisteredResourceConnectionContribution {
        RegisteredResourceConnectionContribution {
            extension_id: "com.navop.middleware.mqtt".to_string(),
            extension_root: PathBuf::from("/tmp"),
            id: "mqtt".to_string(),
            label: "MQTT".to_string(),
            description: None,
            icon_path: None,
            runtime_id: "main".to_string(),
            resource_type: "middleware".to_string(),
            shell_view_id: None,
            form: ResourceConnectionForm {
                tabs: vec![
                    ResourceConnectionFormTab {
                        id: "general".to_string(),
                        label: "常规".to_string(),
                        fields: vec![ResourceConnectionFormField {
                            id: "host".to_string(),
                            label: "主机".to_string(),
                            field_type: ResourceConnectionFieldType::Text,
                            required: false,
                            default_value: None,
                            placeholder: None,
                            secret: false,
                            options: Vec::new(),
                            visible_when: Vec::new(),
                            rows: None,
                        }],
                    },
                    ResourceConnectionFormTab {
                        id: "session".to_string(),
                        label: "会话".to_string(),
                        fields: Vec::new(),
                    },
                ],
            },
        }
    }

    /// 真实布局:页签(宿主渲染)必须在名称之上,两者都有非零高度。
    #[gpui::test]
    fn tab_bar_sits_above_the_name_field(cx: &mut TestAppContext) {
        cx.update(|cx| {
            cx.set_global(AppSettings::default());
            gpui_component::init(cx);
        });
        let (_form, cx) = cx.add_window_view(|window, cx| {
            ExtensionConnectionForm::new(
                ExtensionConnectionFormConfig {
                    contribution: two_tab_contribution(),
                    editing_connection: None,
                    workspaces: Vec::new(),
                    teams: Vec::new(),
                },
                window,
                cx,
            )
        });
        let cx: &mut VisualTestContext = cx;

        let tab_bar = cx
            .debug_bounds("extension-connection-tabs-bar")
            .expect("页签应由宿主渲染");
        let name = cx
            .debug_bounds("extension-connection-name-row")
            .expect("名称字段应渲染");
        assert!(tab_bar.size.height > px(0.0), "页签高度应为正: {tab_bar:?}");
        assert!(name.size.height > px(0.0), "名称高度应为正: {name:?}");
        assert!(
            tab_bar.bottom() <= name.top(),
            "页签必须位于名称之上: tab_bar={tab_bar:?} name={name:?}"
        );
    }
}
