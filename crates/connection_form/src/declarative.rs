mod render;
mod types;

use std::collections::{HashMap, HashSet};

use gpui::{App, AppContext, Context, Entity, FocusHandle, Window};
use gpui_component::{
    input::{InputEvent, InputState, TextareaState},
    select::{SelectEvent, SelectState},
};
use serde_json::{Map, Value};

pub use types::*;

pub struct DeclarativeForm {
    pub(super) config: DeclarativeFormConfig,
    pub(super) active_tab: usize,
    pub(super) focus_handle: FocusHandle,
    pub(super) values: HashMap<String, Entity<String>>,
    pub(super) inputs: HashMap<String, Entity<InputState>>,
    pub(super) textareas: HashMap<String, Entity<TextareaState>>,
    pub(super) selects: HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
    pub(super) cleared_secrets: HashSet<String>,
}

impl DeclarativeForm {
    pub fn new(
        config: DeclarativeFormConfig,
        initial: &Map<String, Value>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut values = HashMap::new();
        let mut inputs = HashMap::new();
        let mut textareas = HashMap::new();
        let mut selects = HashMap::new();
        for field in config.tabs.iter().flat_map(|tab| &tab.fields) {
            let initial_value = initial
                .get(&field.id)
                .map(value_text)
                .or_else(|| field.default_value.clone())
                .unwrap_or_default();
            let value = cx.new(|_| initial_value.clone());
            values.insert(field.id.clone(), value.clone());
            match field.field_type {
                DeclarativeFieldType::Select => {
                    let items = field
                        .options
                        .iter()
                        .map(|item| FormSelectItem {
                            value: item.value.clone(),
                            label: item.label.clone().into(),
                        })
                        .collect::<Vec<_>>();
                    let selected = items
                        .iter()
                        .position(|item| item.value == initial_value)
                        .map(gpui_component::IndexPath::new)
                        .or(Some(Default::default()));
                    let select = cx.new(|cx| SelectState::new(items, selected, window, cx));
                    cx.subscribe_in(&select, window, move |_, _, event, _, cx| {
                        let SelectEvent::Confirm(selected) = event;
                        if let Some(selected) = selected {
                            value.update(cx, |value, cx| {
                                *value = selected.clone();
                                cx.notify();
                            });
                        }
                    })
                    .detach();
                    selects.insert(field.id.clone(), select);
                }
                DeclarativeFieldType::Checkbox => {}
                DeclarativeFieldType::TextArea => {
                    let placeholder = field.placeholder.clone().unwrap_or_default();
                    let input = cx.new(|cx| {
                        let mut state = TextareaState::new(window, cx)
                            .placeholder(placeholder)
                            .auto_grow(4, 12);
                        state.set_value(initial_value, window, cx);
                        state
                    });
                    cx.subscribe_in(&input, window, move |_, input, event, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            let text = input.read(cx).text().to_string();
                            value.update(cx, |value, _| {
                                *value = text;
                            });
                            cx.notify();
                        }
                    })
                    .detach();
                    textareas.insert(field.id.clone(), input);
                }
                _ => {
                    let placeholder = field.placeholder.clone().unwrap_or_default();
                    let masked = field.field_type == DeclarativeFieldType::Password;
                    let input = cx.new(|cx| {
                        let mut state = InputState::new(window, cx)
                            .placeholder(placeholder)
                            .masked(masked);
                        state.set_value(initial_value, window, cx);
                        state
                    });
                    cx.subscribe_in(&input, window, move |_, input, event, _, cx| {
                        if matches!(event, InputEvent::Change) {
                            value.update(cx, |value, cx| {
                                *value = input.read(cx).text().to_string();
                            });
                            cx.notify();
                        }
                    })
                    .detach();
                    inputs.insert(field.id.clone(), input);
                }
            }
        }
        Self {
            config,
            active_tab: 0,
            focus_handle: cx.focus_handle(),
            values,
            inputs,
            textareas,
            selects,
            cleared_secrets: HashSet::new(),
        }
    }

    pub fn collect(
        &self,
        cx: &App,
    ) -> Result<(Map<String, Value>, HashMap<String, String>), String> {
        self.collect_with_preserved_secrets(cx, &std::collections::HashSet::new())
    }

    pub fn collect_with_preserved_secrets(
        &self,
        cx: &App,
        preserved: &std::collections::HashSet<String>,
    ) -> Result<(Map<String, Value>, HashMap<String, String>), String> {
        let mut config = Map::new();
        let mut secrets = HashMap::new();
        for field in self.config.tabs.iter().flat_map(|tab| &tab.fields) {
            if !self.visible(field, cx) {
                continue;
            }
            let value = self.value(&field.id, cx);
            if field.required
                && value.trim().is_empty()
                && !(field.secret && preserved.contains(&field.id))
            {
                return Err(format!("{} is required", field.label));
            }
            if field.secret {
                if !value.is_empty() {
                    secrets.insert(field.id.clone(), value);
                }
            } else {
                config.insert(field.id.clone(), typed_value(field.field_type, value)?);
            }
        }
        Ok((config, secrets))
    }

    pub fn visible_secret_ids(&self, cx: &App) -> std::collections::HashSet<String> {
        self.config
            .tabs
            .iter()
            .flat_map(|tab| &tab.fields)
            .filter(|field| field.secret && self.visible(field, cx))
            .map(|field| field.id.clone())
            .collect()
    }

    pub fn cleared_secret_ids(&self) -> HashSet<String> {
        self.cleared_secrets.clone()
    }

    pub(super) fn clear_secret(&mut self, id: &str, window: &mut Window, cx: &mut Context<Self>) {
        self.cleared_secrets.insert(id.to_string());
        if let Some(input) = self.inputs.get(id) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        if let Some(input) = self.textareas.get(id) {
            input.update(cx, |input, cx| input.set_value("", window, cx));
        }
        cx.notify();
    }

    pub(super) fn value(&self, id: &str, cx: &App) -> String {
        self.values
            .get(id)
            .map(|value| value.read(cx).clone())
            .unwrap_or_default()
    }

    pub(super) fn visible(&self, field: &DeclarativeFormField, cx: &App) -> bool {
        field
            .visible_when
            .iter()
            .all(|rule| self.value(&rule.field, cx) == rule.equals)
    }
}

fn value_text(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
}

fn typed_value(field_type: DeclarativeFieldType, value: String) -> Result<Value, String> {
    match field_type {
        DeclarativeFieldType::Number => value.parse::<i64>().map(Value::from).or_else(|_| {
            value
                .parse::<f64>()
                .ok()
                .and_then(serde_json::Number::from_f64)
                .map(Value::Number)
                .ok_or_else(|| "number field is invalid".to_string())
        }),
        DeclarativeFieldType::Checkbox => Ok(Value::Bool(value.parse().unwrap_or(false))),
        _ => Ok(Value::String(value)),
    }
}
