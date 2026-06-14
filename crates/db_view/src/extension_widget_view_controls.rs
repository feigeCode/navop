use std::collections::BTreeMap;

use extension_component::{SelectOption, UiFieldKind};
use gpui::{
    AnyElement, AppContext, Context, Entity, Focusable, IntoElement, ParentElement, SharedString,
    Styled, Subscription, Window, div, prelude::FluentBuilder, px,
};
use gpui_component::{
    ActiveTheme, IndexPath, Sizable,
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState},
    select::{SearchableVec, Select, SelectEvent, SelectItem, SelectState},
    v_flex,
};
use rust_i18n::t;

use crate::{
    extension_widget::{ExtensionWidgetField, ExtensionWidgetModel, field_source_label},
    extension_widget_view::ExtensionWidgetView,
};

pub type ExtensionSelectState = Entity<SelectState<SearchableVec<ExtensionSelectOption>>>;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExtensionSelectOption {
    value: String,
    label: String,
}

impl SelectItem for ExtensionSelectOption {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

pub fn render_field_row(
    view: &ExtensionWidgetView,
    field: ExtensionWidgetField,
    cx: &mut Context<ExtensionWidgetView>,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(render_field_label(&field, cx))
        .child(render_field_control(view, field, cx))
}

pub fn build_select_states(
    model: &ExtensionWidgetModel,
    window: &mut Window,
    cx: &mut Context<ExtensionWidgetView>,
) -> (BTreeMap<String, ExtensionSelectState>, Vec<Subscription>) {
    let mut states = BTreeMap::new();
    let mut subscriptions = Vec::new();
    for field in &model.fields {
        if field.kind != UiFieldKind::Select {
            continue;
        }
        let state = cx.new(|cx| {
            SelectState::new(
                SearchableVec::new(select_items(field)),
                selected_option_index(field),
                window,
                cx,
            )
            .searchable(true)
        });
        let field_id = field.id.clone();
        subscriptions.push(cx.subscribe_in(
            &state,
            window,
            move |view, _, event: &SelectEvent<SearchableVec<ExtensionSelectOption>>, _, cx| {
                let SelectEvent::Confirm(value) = event;
                update_select_value(view, &field_id, value.clone());
                cx.notify();
            },
        ));
        states.insert(field.id.clone(), state);
    }
    (states, subscriptions)
}

pub fn build_input_states(
    model: &ExtensionWidgetModel,
    window: &mut Window,
    cx: &mut Context<ExtensionWidgetView>,
) -> (BTreeMap<String, Entity<InputState>>, Vec<Subscription>) {
    let mut states = BTreeMap::new();
    let mut subscriptions = Vec::new();
    for field in model.fields.iter().filter(|field| is_input_field(field)) {
        let state = input_state_for_field(field, window, cx);
        let field_id = field.id.clone();
        subscriptions.push(cx.subscribe_in(
            &state,
            window,
            move |view, input, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    view.set_value(field_id.clone(), input.read(cx).value().to_string());
                    cx.notify();
                }
            },
        ));
        states.insert(field.id.clone(), state);
    }
    (states, subscriptions)
}

pub fn focus_first_field(
    model: &ExtensionWidgetModel,
    input_states: &BTreeMap<String, Entity<InputState>>,
    select_states: &BTreeMap<String, ExtensionSelectState>,
    window: &mut Window,
    cx: &mut Context<ExtensionWidgetView>,
) {
    for field in &model.fields {
        if let Some(state) = input_states.get(&field.id) {
            state.update(cx, |state, cx| state.focus(window, cx));
            return;
        }
        if let Some(state) = select_states.get(&field.id) {
            let focus_handle = { state.read(cx).focus_handle(cx) };
            focus_handle.focus(window, cx);
            return;
        }
    }
}

fn render_field_control(
    view: &ExtensionWidgetView,
    field: ExtensionWidgetField,
    cx: &mut Context<ExtensionWidgetView>,
) -> AnyElement {
    if let Some(state) = view.select_state(&field.id) {
        return select_row_control(&field, state).into_any_element();
    }
    if let Some(state) = view.input_state(&field.id) {
        return input_control(&field, state).into_any_element();
    }
    if field.kind == UiFieldKind::Checkbox {
        return checkbox_control(
            field.id.clone(),
            checkbox_checked(view.value_for(&field.id).map(String::as_str)),
            cx,
        )
        .into_any_element();
    }
    div()
        .h(px(34.0))
        .w_full()
        .flex()
        .items_center()
        .px_3()
        .border_1()
        .border_color(cx.theme().border)
        .rounded(cx.theme().radius)
        .text_sm()
        .text_color(cx.theme().muted_foreground)
        .child(field_source_label(&field))
        .into_any_element()
}

fn render_field_label(
    field: &ExtensionWidgetField,
    cx: &mut Context<ExtensionWidgetView>,
) -> impl IntoElement {
    h_flex()
        .gap_1()
        .child(div().text_sm().child(field.label.clone()))
        .when(field.required, |this| {
            this.child(div().text_sm().text_color(cx.theme().danger).child("*"))
        })
}

fn select_row_control(
    field: &ExtensionWidgetField,
    state: &ExtensionSelectState,
) -> impl IntoElement {
    Select::new(state)
        .placeholder(field_source_label(field))
        .search_placeholder(t!("DbObjectSelector.search", item = field.label.clone()).to_string())
        .w_full()
        .small()
}

fn input_control(field: &ExtensionWidgetField, state: &Entity<InputState>) -> impl IntoElement {
    let mut input = Input::new(state).w_full().small();
    if field.kind == UiFieldKind::Password {
        input = input.mask_toggle();
    }
    input
}

fn checkbox_control(
    field_id: String,
    checked: bool,
    cx: &mut Context<ExtensionWidgetView>,
) -> impl IntoElement {
    let view = cx.entity();
    Checkbox::new(format!("extension-checkbox-{field_id}"))
        .checked(checked)
        .on_click(move |checked, _, cx| {
            view.update(cx, |view, cx| {
                view.set_value(field_id.clone(), checked.to_string());
                cx.notify();
            });
        })
}

fn input_state_for_field(
    field: &ExtensionWidgetField,
    window: &mut Window,
    cx: &mut Context<ExtensionWidgetView>,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).placeholder(field_source_label(field));
        if field.kind == UiFieldKind::Password {
            state = state.masked(true);
        }
        if field.kind == UiFieldKind::TextArea {
            state = state.auto_grow(3, 8);
        }
        if let Some(value) = &field.value {
            state = state.default_value(value);
        }
        state
    })
}

fn update_select_value(view: &mut ExtensionWidgetView, field_id: &str, value: Option<String>) {
    match value {
        Some(value) => view.set_value(field_id.to_string(), value),
        None => view.remove_value(field_id),
    }
}

fn is_input_field(field: &ExtensionWidgetField) -> bool {
    field.options.is_empty()
        && matches!(
            field.kind,
            UiFieldKind::Text | UiFieldKind::TextArea | UiFieldKind::Password
        )
}

fn checkbox_checked(value: Option<&str>) -> bool {
    value
        .map(|value| {
            matches!(
                value.trim().to_ascii_lowercase().as_str(),
                "true" | "1" | "yes" | "on"
            )
        })
        .unwrap_or(false)
}

fn selected_option_index(field: &ExtensionWidgetField) -> Option<IndexPath> {
    let selected = field
        .value
        .as_ref()
        .or_else(|| field.options.first().map(|option| &option.value))?;
    field
        .options
        .iter()
        .position(|option| &option.value == selected)
        .map(IndexPath::new)
}

fn select_items(field: &ExtensionWidgetField) -> Vec<ExtensionSelectOption> {
    field.options.iter().map(option_item).collect()
}

fn option_item(option: &SelectOption) -> ExtensionSelectOption {
    ExtensionSelectOption {
        value: option.value.clone(),
        label: option.label.clone(),
    }
}
