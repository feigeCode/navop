use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, px,
};
use gpui_component::{
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    form::{field, v_form},
    h_flex,
    input::{Input, Textarea},
    select::Select,
    tab::{Tab, TabBar},
    v_flex,
};

use super::{DeclarativeFieldType, DeclarativeForm, DeclarativeFormField};

impl DeclarativeForm {
    fn render_field(
        &self,
        field_info: &DeclarativeFormField,
        cx: &mut Context<Self>,
    ) -> gpui_component::form::Field {
        let id = field_info.id.clone();
        let checkbox_id = id.clone();
        field()
            .label(field_info.label.clone())
            .required(field_info.required)
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
                                            let next =
                                                !value.read(cx).parse::<bool>().unwrap_or(false);
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
            )
    }
}

impl Focusable for DeclarativeForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DeclarativeForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tabs = self.config.tabs.clone();
        let active = self.active_tab.min(tabs.len().saturating_sub(1));
        let fields = tabs
            .get(active)
            .map(|tab| tab.fields.clone())
            .unwrap_or_default()
            .into_iter()
            .filter(|field| self.visible(field, cx))
            .collect::<Vec<_>>();
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
            .child(
                v_form()
                    .columns(1)
                    .label_width(px(120.))
                    .children(fields.iter().map(|field| self.render_field(field, cx))),
            )
    }
}
