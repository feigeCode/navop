use gpui::prelude::FluentBuilder;
use gpui::{
    App, Axis, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window,
    div, px,
};
use gpui_component::{button::{Button, ButtonVariants as _}, checkbox::Checkbox, form::{field, v_form}, h_flex, input::{Input, Textarea}, select::Select, tab::{Tab, TabBar}, v_flex};
use one_assets::IconName;
use rust_i18n::t;
use std::collections::HashSet;

use super::{DeclarativeFieldType, DeclarativeForm, DeclarativeFormField, auth_subkey};

impl DeclarativeForm {
    fn render_field(
        &self,
        field_info: &DeclarativeFormField,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_component::form::Field> {
        if field_info.field_type == DeclarativeFieldType::Auth {
            return self.render_auth_field(field_info, cx);
        }
        let id = field_info.id.clone();
        let checkbox_id = id.clone();
        vec![
            field()
                .label(field_info.label.clone())
                .required(field_info.required)
                .when(
                    field_info.field_type == DeclarativeFieldType::TextArea,
                    |field| field.items_start(),
                )
                .when(
                    field_info.field_type != DeclarativeFieldType::TextArea,
                    |field| field.items_center(),
                )
                .child(
                    h_flex()
                        .w_full()
                        .when(
                            field_info.field_type == DeclarativeFieldType::Select,
                            |el| {
                                if let Some(state) = self.selects.get(&id) {
                                    el.child(Select::new(state).w_full())
                                } else {
                                    el
                                }
                            },
                        )
                        .when(
                            field_info.field_type == DeclarativeFieldType::Checkbox,
                            |el| {
                                let checked = self.value(&id, cx).parse::<bool>().unwrap_or(false);
                                el.child(
                                    Checkbox::new(format!("{id}-checkbox"))
                                        .checked(checked)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(value) = this.values.get(&checkbox_id) {
                                                let next = !value
                                                    .read(cx)
                                                    .parse::<bool>()
                                                    .unwrap_or(false);
                                                value.update(cx, |value, cx| {
                                                    *value = next.to_string();
                                                    cx.notify();
                                                });
                                            }
                                        })),
                                )
                            },
                        )
                        .when(
                            !matches!(
                                field_info.field_type,
                                DeclarativeFieldType::Select
                                    | DeclarativeFieldType::Checkbox
                                    | DeclarativeFieldType::TextArea
                            ),
                            |el| {
                                if let Some(state) = self.inputs.get(&id) {
                                    let input = Input::new(state).w_full();
                                    el.child(
                                        if field_info.field_type == DeclarativeFieldType::Password {
                                            input.mask_toggle()
                                        } else {
                                            input
                                        },
                                    )
                                } else {
                                    el
                                }
                            },
                        )
                        .when(
                            field_info.field_type == DeclarativeFieldType::TextArea,
                            |el| {
                                if let Some(state) = self.textareas.get(&id) {
                                    el.child(Textarea::new(state).w_full())
                                } else {
                                    el
                                }
                            },
                        )
                        .when(
                            field_info.field_type == DeclarativeFieldType::FilePath,
                            |el| {
                                let file_field = field_info.id.clone();
                                el.child(
                                    Button::new(format!("{file_field}-browse-file"))
                                        .icon(IconName::FolderOpen)
                                        .ghost()
                                        .on_click(cx.listener(move |this, _, _window, cx| {
                                            this.browse_file_path(&file_field, cx);
                                        })),
                                )
                            },
                        )
                        .when(field_info.secret, |el| {
                            let field_id = id.clone();
                            el.child(
                                Button::new(format!("{id}-clear-secret"))
                                    .ghost()
                                    .label("Clear")
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.clear_secret(&field_id, window, cx);
                                    })),
                            )
                        }),
                ),
        ]
    }

    fn render_auth_field(
        &self,
        field_info: &DeclarativeFormField,
        cx: &mut Context<Self>,
    ) -> Vec<gpui_component::form::Field> {
        let username_id = auth_subkey(&field_info.id, "username");
        let password_id = auth_subkey(&field_info.id, "password");
        let reference_selected = self.auth_has_reference(&field_info.id, cx);
        let mut fields = vec![
            field()
                .label(t!("Credential.keychain").to_string())
                .items_center()
                .child(
                    self.auth_pickers
                        .get(&field_info.id)
                        .map(|picker| div().w_full().child(picker.clone()))
                        .unwrap_or_else(|| div().w_full()),
                ),
        ];
        if !reference_selected {
            fields.push(
                field()
                    .label(t!("Credential.username").to_string())
                    .items_center()
                    .child(self.render_auth_input(&username_id, false, cx)),
            );
            fields.push(
                field()
                    .label(t!("Credential.password").to_string())
                    .items_center()
                    .child(self.render_auth_input(&password_id, true, cx)),
            );
        }
        fields
    }

    fn render_auth_input(
        &self,
        id: &str,
        masked: bool,
        _cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        if let Some(state) = self.inputs.get(id) {
            let input = Input::new(state).w_full();
            let input = if masked { input.mask_toggle() } else { input };
            input.into_any_element()
        } else {
            div().w_full().into_any_element()
        }
    }
}

impl Focusable for DeclarativeForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DeclarativeForm {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.config.tabs.clone();
        let active = self.active_tab.min(tabs.len().saturating_sub(1));
        let fields = tabs
            .get(active)
            .map(|tab| tab.fields.clone())
            .unwrap_or_default();
        let hosted = self.host_supplies_tab_bar;
        if hosted {
            let hidden = self.host_hidden.clone();
            return v_flex()
                .size_full()
                .child(self.render_fields(&fields, &hidden, window, cx))
                .into_any_element();
        }
        v_flex()
            .size_full()
            .gap_4()
            .when(tabs.len() > 1, |el| {
                el.child(
                    TabBar::new("declarative-connection-tabs")
                        .selected_index(active)
                        .on_click(cx.listener(|this, index: &usize, _, cx| {
                            this.active_tab = *index;
                            cx.notify();
                        }))
                        .children(tabs.iter().map(|tab| Tab::new().label(tab.label.clone()))),
                )
            })
            .child(self.render_fields(&fields, &HashSet::new(), window, cx))
            .into_any_element()
    }
}

impl DeclarativeForm {
    /// 渲染单个 tab 的声明字段(不含 TabBar),供宿主在自绘页内复用统一引擎。
    ///
    /// `hidden`:需隐藏的字段 id 集合(如选中钥匙串引用后的 username/password)。
    pub fn render_tab_fields(
        &mut self,
        tab_index: usize,
        hidden: &HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let fields = self
            .config
            .tabs
            .get(tab_index)
            .map(|tab| tab.fields.clone())
            .unwrap_or_default();
        self.render_fields(&fields, hidden, window, cx)
    }

    fn render_fields(
        &mut self,
        fields: &[DeclarativeFormField],
        hidden: &HashSet<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        self.apply_pending_file_path(window, cx);
        let selected = fields
            .iter()
            .filter(|field| !hidden.contains(&field.id) && self.visible(field, cx))
            .collect::<Vec<_>>();
        let rendered = selected
            .into_iter()
            .flat_map(|field| self.render_field(field, cx))
            .collect::<Vec<_>>();
        v_form()
            .layout(Axis::Horizontal)
            .columns(1)
            .label_width(px(120.))
            .children(rendered)
            .into_any_element()
    }
}
