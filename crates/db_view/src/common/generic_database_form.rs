use crate::DatabaseFormEvent;
use crate::common::db_connection_form::FormSelectItem;
use crate::common::manifest_bridge::{
    default_select_value, field_visible, resolve_field_options, to_select_items, translate,
};
use db::plugin_manifest::{DatabaseFormField, DatabaseFormFieldType, DatabaseFormManifest};
use db::{GlobalDbState, plugin::DatabaseOperationRequest};
use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement, ParentElement, Render,
    Styled, Subscription, Window, prelude::*, px,
};
use gpui_component::form::h_form;
use gpui_component::{
    IndexPath, Sizable, Size,
    form::field,
    input::{Input, InputEvent, InputState},
    select::{Select, SelectEvent, SelectState},
    v_flex,
};
use one_core::storage::DatabaseType;
use std::collections::HashMap;

pub struct GenericDatabaseForm {
    database_type: DatabaseType,
    manifest: DatabaseFormManifest,
    focus_handle: FocusHandle,
    field_values: HashMap<String, Entity<String>>,
    field_inputs: HashMap<String, Entity<InputState>>,
    field_selects: HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
    is_edit_mode: bool,
    _subscriptions: Vec<Subscription>,
}

impl GenericDatabaseForm {
    pub fn new(
        database_type: DatabaseType,
        manifest: DatabaseFormManifest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let is_edit_mode = matches!(
            manifest.kind,
            db::plugin_manifest::DatabaseFormKind::EditDatabase
        );
        let plugin = cx
            .global::<GlobalDbState>()
            .get_plugin(&database_type)
            .expect("database plugin should exist");

        let all_fields = flatten_fields(&manifest);
        let mut initial_values = HashMap::new();
        for field in &all_fields {
            let initial = if matches!(field.field_type, DatabaseFormFieldType::Select) {
                let options = resolve_field_options(plugin.as_ref(), field, &initial_values);
                default_select_value(field, &options)
            } else {
                field.default_value.clone().unwrap_or_default()
            };
            initial_values.insert(field.id.clone(), initial);
        }

        let mut field_values = HashMap::new();
        let mut field_inputs = HashMap::new();
        let mut field_selects = HashMap::new();
        let mut subscriptions = Vec::new();

        for field in &all_fields {
            let current_value = initial_values.get(&field.id).cloned().unwrap_or_default();
            let value_entity = cx.new(|_| current_value.clone());
            field_values.insert(field.id.clone(), value_entity.clone());

            if matches!(field.field_type, DatabaseFormFieldType::Select) {
                let options = resolve_field_options(plugin.as_ref(), field, &initial_values);
                let items = to_select_items(options.clone());
                let selected_index = items
                    .iter()
                    .position(|item| item.value == current_value)
                    .or_else(|| (!items.is_empty()).then_some(0))
                    .map(IndexPath::new);
                let select = cx.new(|cx| SelectState::new(items, selected_index, window, cx));
                let field_id = field.id.clone();
                let subscription = cx.subscribe_in(
                    &select,
                    window,
                    move |this, _, event: &SelectEvent<Vec<FormSelectItem>>, window, cx| {
                        if let SelectEvent::Confirm(Some(selected)) = event {
                            if let Some(value) = this.field_values.get(&field_id) {
                                value.update(cx, |stored, cx| {
                                    *stored = selected.clone();
                                    cx.notify();
                                });
                            }
                            this.on_field_changed(&field_id, window, cx);
                        }
                    },
                );
                subscriptions.push(subscription);
                field_selects.insert(field.id.clone(), select);
            } else {
                let input = cx.new(|cx| build_input_state(field, &current_value, window, cx));
                let field_id = field.id.clone();
                let subscription =
                    cx.subscribe_in(&input, window, move |this, input, event, window, cx| {
                        if let InputEvent::Change = event {
                            let value = input.read(cx).text().to_string();
                            if let Some(stored) = this.field_values.get(&field_id) {
                                stored.update(cx, |current, cx| {
                                    *current = value;
                                    cx.notify();
                                });
                            }
                            this.on_field_changed(&field_id, window, cx);
                        }
                    });
                subscriptions.push(subscription);
                field_inputs.insert(field.id.clone(), input);
            }
        }

        Self {
            database_type,
            manifest,
            focus_handle,
            field_values,
            field_inputs,
            field_selects,
            is_edit_mode,
            _subscriptions: subscriptions,
        }
    }

    fn on_field_changed(
        &mut self,
        changed_field: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.refresh_dependents(changed_field, window, cx);
        self.emit_form_changed(cx);
    }

    fn refresh_dependents(
        &mut self,
        changed_field: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plugin = cx
            .global::<GlobalDbState>()
            .get_plugin(&self.database_type)
            .expect("database plugin should exist");
        let state = self.current_values(cx);

        for field in flatten_fields(&self.manifest) {
            let should_refresh = field
                .default_when
                .iter()
                .any(|rule| rule.when_field_changes == changed_field);
            if !should_refresh {
                continue;
            }

            let Some(select) = self.field_selects.get(&field.id) else {
                continue;
            };

            let options = resolve_field_options(plugin.as_ref(), &field, &state);
            let next_value = default_select_value(&field, &options);
            let items = to_select_items(options);
            let selected_index = items
                .iter()
                .position(|item| item.value == next_value)
                .or_else(|| (!items.is_empty()).then_some(0))
                .map(IndexPath::new);

            select.update(cx, |state, cx| {
                state.set_items(items, window, cx);
                state.set_selected_index(selected_index, window, cx);
            });

            if let Some(value) = self.field_values.get(&field.id) {
                value.update(cx, |current, cx| {
                    *current = next_value.clone();
                    cx.notify();
                });
            }
        }
    }

    fn current_values(&self, cx: &App) -> HashMap<String, String> {
        self.field_values
            .iter()
            .map(|(key, value)| (key.clone(), value.read(cx).clone()))
            .collect()
    }

    fn build_request(&self, cx: &App) -> DatabaseOperationRequest {
        let field_values = self.current_values(cx);
        let database_name = field_values.get("name").cloned().unwrap_or_default();
        DatabaseOperationRequest {
            database_name,
            field_values,
        }
    }

    fn emit_form_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(DatabaseFormEvent::FormChanged(self.build_request(cx)));
    }
}

impl EventEmitter<DatabaseFormEvent> for GenericDatabaseForm {}

impl Focusable for GenericDatabaseForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GenericDatabaseForm {
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
                        Some(render_database_field(
                            field,
                            self.is_edit_mode,
                            &self.field_inputs,
                            &self.field_selects,
                        ))
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
    if matches!(field.field_type, DatabaseFormFieldType::Password) {
        input = input.masked(true);
    }
    if matches!(field.field_type, DatabaseFormFieldType::TextArea) {
        input = input
            .multi_line(true)
            .rows(field.rows.unwrap_or(3) as usize);
    }
    input.set_value(value.to_string(), window, cx);
    input
}

fn render_database_field(
    field_info: &DatabaseFormField,
    is_edit_mode: bool,
    field_inputs: &HashMap<String, Entity<InputState>>,
    field_selects: &HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
) -> gpui_component::form::Field {
    let is_select = matches!(field_info.field_type, DatabaseFormFieldType::Select);
    let is_textarea = matches!(field_info.field_type, DatabaseFormFieldType::TextArea);
    let is_password = matches!(field_info.field_type, DatabaseFormFieldType::Password);
    let disabled = is_edit_mode && field_info.disabled_when_editing;

    field()
        .label(translate(&field_info.label_i18n_key))
        .required(field_info.required)
        .when(!is_textarea, |f| f.items_center())
        .when(is_textarea, |f| f.items_start())
        .label_justify_end()
        .child(if is_select {
            field_selects
                .get(&field_info.id)
                .map(|select| {
                    Select::new(select)
                        .w_full()
                        .disabled(disabled)
                        .into_any_element()
                })
                .unwrap_or_else(|| v_flex().into_any_element())
        } else {
            field_inputs
                .get(&field_info.id)
                .map(|input_state| {
                    let input = Input::new(input_state).w_full().disabled(disabled);
                    let input = if is_password {
                        input.mask_toggle()
                    } else {
                        input
                    };
                    input.into_any_element()
                })
                .unwrap_or_else(|| v_flex().into_any_element())
        })
}
