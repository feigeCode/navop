use crate::common::manifest_bridge::{field_visible, translate};
use crate::common::{SchemaFormEvent, SchemaOperationRequest};
use db::plugin_manifest::{DatabaseFormField, DatabaseFormFieldType, DatabaseFormManifest};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, prelude::*, px,
};
use gpui_component::form::h_form;
use gpui_component::{
    Sizable, Size,
    form::field,
    input::{Input, InputEvent, InputState},
    v_flex,
};
use std::collections::HashMap;

pub struct GenericSchemaForm {
    manifest: DatabaseFormManifest,
    focus_handle: FocusHandle,
    field_values: HashMap<String, Entity<String>>,
    field_inputs: HashMap<String, Entity<InputState>>,
    _subscriptions: Vec<Subscription>,
}

impl GenericSchemaForm {
    pub fn new(
        manifest: DatabaseFormManifest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let mut field_values = HashMap::new();
        let mut field_inputs = HashMap::new();
        let mut subscriptions = Vec::new();

        for field in flatten_fields(&manifest) {
            let value = field.default_value.clone().unwrap_or_default();
            let value_entity = cx.new(|_| value.clone());
            let input = cx.new(|cx| build_input_state(&field, &value, window, cx));
            let field_id = field.id.clone();
            let subscription =
                cx.subscribe_in(&input, window, move |this, input, event, _window, cx| {
                    if let InputEvent::Change = event {
                        let next = input.read(cx).text().to_string();
                        if let Some(value) = this.field_values.get(&field_id) {
                            value.update(cx, |stored, cx| {
                                *stored = next;
                                cx.notify();
                            });
                        }
                        this.emit_form_changed(cx);
                    }
                });
            subscriptions.push(subscription);
            field_values.insert(field.id.clone(), value_entity);
            field_inputs.insert(field.id.clone(), input);
        }

        Self {
            manifest,
            focus_handle,
            field_values,
            field_inputs,
            _subscriptions: subscriptions,
        }
    }

    fn current_values(&self, cx: &App) -> HashMap<String, String> {
        self.field_values
            .iter()
            .map(|(key, value)| (key.clone(), value.read(cx).clone()))
            .collect()
    }

    fn build_request(&self, cx: &App) -> SchemaOperationRequest {
        let values = self.current_values(cx);
        let schema_name = values
            .get("name")
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string();
        let comment = values
            .get("comment")
            .cloned()
            .unwrap_or_default()
            .trim()
            .to_string();
        SchemaOperationRequest {
            schema_name,
            comment: (!comment.is_empty()).then_some(comment),
        }
    }

    fn emit_form_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(SchemaFormEvent::FormChanged(self.build_request(cx)));
    }
}

impl EventEmitter<SchemaFormEvent> for GenericSchemaForm {}

impl Focusable for GenericSchemaForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GenericSchemaForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let current_values = self.current_values(cx);
        v_flex()
            .gap_4()
            .p_4()
            .size_full()
            .children(self.manifest.tabs.iter().map(|tab| {
                h_form()
                    .with_size(Size::Small)
                    .columns(1)
                    .label_width(px(100.))
                    .children(tab.fields.iter().filter_map(|field| {
                        if !field_visible(field, &current_values) {
                            return None;
                        }
                        self.field_inputs
                            .get(&field.id)
                            .map(|input| render_schema_field(field, input))
                    }))
            }))
    }
}

fn flatten_fields(manifest: &DatabaseFormManifest) -> Vec<DatabaseFormField> {
    manifest
        .tabs
        .iter()
        .flat_map(|tab| tab.fields.iter().cloned())
        .collect()
}

fn build_input_state(
    field: &DatabaseFormField,
    value: &str,
    window: &mut Window,
    cx: &mut Context<InputState>,
) -> InputState {
    let placeholder = field
        .placeholder_i18n_key
        .as_deref()
        .map(translate)
        .unwrap_or_default();
    let mut input = InputState::new(window, cx).placeholder(placeholder);
    if matches!(field.field_type, DatabaseFormFieldType::TextArea) {
        input = input
            .multi_line(true)
            .rows(field.rows.unwrap_or(3) as usize);
    }
    input.set_value(value.to_string(), window, cx);
    input
}

fn render_schema_field(
    field_info: &DatabaseFormField,
    input_state: &Entity<InputState>,
) -> gpui_component::form::Field {
    let is_textarea = matches!(field_info.field_type, DatabaseFormFieldType::TextArea);
    field()
        .label(translate(&field_info.label_i18n_key))
        .required(field_info.required)
        .when(!is_textarea, |f| f.items_center())
        .when(is_textarea, |f| f.items_start())
        .label_justify_end()
        .child(Input::new(input_state).w_full())
}
