use crate::common::DatabaseUserFormEvent;
use crate::common::db_connection_form::FormSelectItem;
use crate::common::generic_database_form::{
    build_input_state, flatten_fields, render_database_field,
};
use crate::common::manifest_bridge::{
    default_select_value, field_visible, resolve_field_options, to_select_items, translate,
};
use db::GlobalDbState;
use db::plugin::DatabaseUserOperationRequest;
use db::plugin_manifest::{DatabaseFormField, DatabaseFormFieldType, DatabaseFormManifest};
use gpui::{
    App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Subscription, Window, px,
};
use gpui_component::{
    IndexPath, Sizable, Size,
    form::h_form,
    input::{InputEvent, InputState},
    select::{SelectEvent, SelectState},
    v_flex,
};
use one_core::storage::DatabaseType;
use std::collections::HashMap;
use std::rc::Rc;

pub struct GenericUserForm {
    database_type: DatabaseType,
    manifest: DatabaseFormManifest,
    focus_handle: FocusHandle,
    field_values: HashMap<String, Entity<String>>,
    field_inputs: HashMap<String, Entity<InputState>>,
    field_selects: HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
    text_resolver: Rc<dyn Fn(&str) -> String>,
    _subscriptions: Vec<Subscription>,
}

impl GenericUserForm {
    pub fn new(
        database_type: DatabaseType,
        manifest: DatabaseFormManifest,
        initial: Option<DatabaseUserOperationRequest>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::new_with_text_resolver(
            database_type,
            manifest,
            initial,
            Rc::new(translate),
            window,
            cx,
        )
    }

    pub fn new_with_text_resolver(
        database_type: DatabaseType,
        manifest: DatabaseFormManifest,
        initial: Option<DatabaseUserOperationRequest>,
        text_resolver: Rc<dyn Fn(&str) -> String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let plugin = cx
            .global::<GlobalDbState>()
            .get_plugin(&database_type)
            .expect("database plugin should exist");
        let fields = flatten_fields(&manifest);
        let initial_values = initial_user_values(&fields, initial, plugin.as_ref());
        let mut this = Self {
            database_type,
            manifest,
            focus_handle,
            field_values: HashMap::new(),
            field_inputs: HashMap::new(),
            field_selects: HashMap::new(),
            text_resolver,
            _subscriptions: Vec::new(),
        };
        this.build_field_state(fields, initial_values, window, cx);
        this.emit_form_changed(cx);
        this
    }

    fn build_field_state(
        &mut self,
        fields: Vec<DatabaseFormField>,
        initial_values: HashMap<String, String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for field in fields {
            let value = initial_values.get(&field.id).cloned().unwrap_or_default();
            let value_entity = cx.new(|_| value.clone());
            self.field_values.insert(field.id.clone(), value_entity);
            if matches!(field.field_type, DatabaseFormFieldType::Select) {
                self.build_select_state(field, &value, window, cx);
            } else {
                self.build_input_field_state(field, &value, window, cx);
            }
        }
    }

    fn build_select_state(
        &mut self,
        field: DatabaseFormField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let plugin = cx
            .global::<GlobalDbState>()
            .get_plugin(&self.database_type)
            .unwrap();
        let options = resolve_field_options(plugin.as_ref(), &field, &self.current_values(cx));
        let items = to_select_items(options, self.text_resolver.as_ref());
        let selected = selected_index(&items, value);
        let select = cx.new(|cx| SelectState::new(items, selected, window, cx));
        let field_id = field.id.clone();
        let subscription = cx.subscribe_in(
            &select,
            window,
            move |this, _, event: &SelectEvent<Vec<FormSelectItem>>, _window, cx| {
                if let SelectEvent::Confirm(Some(selected)) = event {
                    this.set_value(&field_id, selected, cx);
                    this.emit_form_changed(cx);
                }
            },
        );
        self._subscriptions.push(subscription);
        self.field_selects.insert(field.id, select);
    }

    fn build_input_field_state(
        &mut self,
        field: DatabaseFormField,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let input =
            cx.new(|cx| build_input_state(&field, value, self.text_resolver.as_ref(), window, cx));
        let field_id = field.id.clone();
        let subscription = cx.subscribe_in(&input, window, move |this, input, event, _, cx| {
            if let InputEvent::Change = event {
                let value = input.read(cx).text().to_string();
                this.set_value(&field_id, &value, cx);
                this.emit_form_changed(cx);
            }
        });
        self._subscriptions.push(subscription);
        self.field_inputs.insert(field.id, input);
    }

    fn set_value(&mut self, field_id: &str, value: &str, cx: &mut Context<Self>) {
        if let Some(stored) = self.field_values.get(field_id) {
            stored.update(cx, |current, cx| {
                *current = value.to_string();
                cx.notify();
            });
        }
    }

    fn current_values(&self, cx: &App) -> HashMap<String, String> {
        self.field_values
            .iter()
            .map(|(key, value)| (key.clone(), value.read(cx).clone()))
            .collect()
    }

    fn build_request(&self, cx: &App) -> DatabaseUserOperationRequest {
        let values = self.current_values(cx);
        DatabaseUserOperationRequest {
            user_name: values.get("name").cloned().unwrap_or_default(),
            host: non_empty_value(&values, "host"),
            database: non_empty_value(&values, "database"),
            field_values: values,
        }
    }

    pub fn current_request(&self, cx: &App) -> DatabaseUserOperationRequest {
        self.build_request(cx)
    }

    fn emit_form_changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(DatabaseUserFormEvent::FormChanged(self.build_request(cx)));
    }
}

impl EventEmitter<DatabaseUserFormEvent> for GenericUserForm {}

impl Focusable for GenericUserForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GenericUserForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let values = self.current_values(cx);
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
                        field_visible(field, &values).then(|| {
                            render_database_field(
                                field,
                                false,
                                self.text_resolver.as_ref(),
                                &self.field_inputs,
                                &self.field_selects,
                            )
                        })
                    }))
            }))
    }
}

fn selected_index(items: &[FormSelectItem], value: &str) -> Option<IndexPath> {
    items
        .iter()
        .position(|item| item.value == value)
        .or_else(|| (!items.is_empty()).then_some(0))
        .map(IndexPath::new)
}

fn initial_user_values(
    fields: &[DatabaseFormField],
    initial: Option<DatabaseUserOperationRequest>,
    plugin: &dyn db::plugin::DatabasePlugin,
) -> HashMap<String, String> {
    let mut state = HashMap::new();
    for field in fields {
        let value = initial_field_value(field, initial.as_ref(), plugin, &state);
        state.insert(field.id.clone(), value);
    }
    state
}

fn initial_field_value(
    field: &DatabaseFormField,
    initial: Option<&DatabaseUserOperationRequest>,
    plugin: &dyn db::plugin::DatabasePlugin,
    state: &HashMap<String, String>,
) -> String {
    match field.id.as_str() {
        "name" => initial.map(|request| request.user_name.clone()),
        "host" => initial.and_then(|request| request.host.clone()),
        "database" => initial.and_then(|request| request.database.clone()),
        key => initial.and_then(|request| request.field_values.get(key).cloned()),
    }
    .or_else(|| {
        if matches!(field.field_type, DatabaseFormFieldType::Select) {
            Some(default_select_value(
                field,
                &resolve_field_options(plugin, field, state),
            ))
        } else {
            field.default_value.clone()
        }
    })
    .unwrap_or_default()
}

fn non_empty_value(values: &HashMap<String, String>, key: &str) -> Option<String> {
    values
        .get(key)
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
