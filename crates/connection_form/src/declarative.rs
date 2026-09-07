mod render;
mod types;

use std::collections::{HashMap, HashSet};

use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, PathPromptOptions, Window,
};
use gpui_component::{
    input::{InputEvent, InputState, TextareaState},
    select::{SelectEvent, SelectState},
};
use one_core::storage::CredentialReference;
use serde_json::{Map, Value};

use crate::credential::{
    create_credential_picker, CredentialCapabilities, CredentialPickerConfig, CredentialPickerEvent,
    CredentialReferencePicker,
};

pub use types::*;

/// Auth 复合字段内部各子值的键后缀。
fn auth_subkey(field_id: &str, part: &str) -> String {
    format!("{field_id}.{part}")
}

pub struct DeclarativeForm {
    pub(super) config: DeclarativeFormConfig,
    pub(super) active_tab: usize,
    pub(super) focus_handle: FocusHandle,
    pub(super) values: HashMap<String, Entity<String>>,
    pub(super) inputs: HashMap<String, Entity<InputState>>,
    pub(super) textareas: HashMap<String, Entity<TextareaState>>,
    pub(super) selects: HashMap<String, Entity<SelectState<Vec<FormSelectItem>>>>,
    pub(super) cleared_secrets: HashSet<String>,
    pub(super) auth_pickers: HashMap<String, Entity<CredentialReferencePicker>>,
    /// FilePath 浏览结果,渲染时应用(需 Window 写回输入态)
    pending_file_path: Entity<Option<(String, String)>>,
    /// 宿主托管模式:TabBar 由宿主(如中间件表单)提供,引擎只渲染当前 tab 字段。
    host_supplies_tab_bar: bool,
    /// 宿主在托管模式下要隐藏的字段 id(如选中钥匙串引用后的账号/密码)。
    host_hidden: HashSet<String>,
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
        let mut auth_pickers = HashMap::new();
        for field in config.tabs.iter().flat_map(|tab| &tab.fields) {
            if field.field_type == DeclarativeFieldType::Auth {
                Self::init_auth_field(
                    field,
                    initial,
                    &mut values,
                    &mut inputs,
                    &mut auth_pickers,
                    window,
                    cx,
                );
                continue;
            }
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
                            .auto_grow(field.rows, field.rows + 8);
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
            auth_pickers,
            pending_file_path: cx.new(|_| None),
            host_supplies_tab_bar: false,
            host_hidden: HashSet::new(),
        }
    }

    /// 切换为宿主托管模式:引擎不再绘制 TabBar,只渲染 `active_tab` 的字段,
    /// 并用 `hidden` 过滤宿主希望隐藏的字段(如选中钥匙串引用后的账号/密码)。
    pub fn host_content(&mut self, active_tab: usize, hidden: HashSet<String>) {
        self.host_supplies_tab_bar = true;
        self.active_tab = active_tab;
        self.host_hidden = hidden;
    }

    /// 暴露声明字段的底层输入态,供宿主在自定义页(如 SSH 隧道 tab)复用同一控件。
    pub fn input_state(&self, id: &str) -> Option<Entity<InputState>> {
        self.inputs.get(id).cloned()
    }

    /// 暴露声明字段的底层多行输入态,供宿主在自定义页复用同一控件。
    pub fn textarea_state(&self, id: &str) -> Option<Entity<TextareaState>> {
        self.textareas.get(id).cloned()
    }

    fn init_auth_field(
        field: &DeclarativeFormField,
        initial: &Map<String, Value>,
        values: &mut HashMap<String, Entity<String>>,
        inputs: &mut HashMap<String, Entity<InputState>>,
        auth_pickers: &mut HashMap<String, Entity<CredentialReferencePicker>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let (mut username, mut password, mut reference) = (String::new(), String::new(), None);
        if let Some(Value::Object(object)) = initial.get(&field.id) {
            username = object
                .get("username")
                .map(value_text)
                .unwrap_or_default();
            password = object
                .get("password")
                .map(value_text)
                .unwrap_or_default();
            reference = object
                .get("credential_reference")
                .and_then(|value| serde_json::from_value(value.clone()).ok());
        }
        for (part, value, masked) in [
            ("username", username.clone(), false),
            ("password", password, true),
        ] {
            let key = auth_subkey(&field.id, part);
            let value_entity = cx.new(|_| value.clone());
            values.insert(key.clone(), value_entity.clone());
            let input = cx.new(|inner| {
                let mut state = InputState::new(window, inner).masked(masked);
                state.set_value(value, window, inner);
                state
            });
            cx.subscribe_in(&input, window, move |_, input, event, _, inner| {
                if matches!(event, InputEvent::Change) {
                    let text = input.read(inner).text().to_string();
                    value_entity.update(inner, |value, _| {
                        *value = text;
                    });
                    inner.notify();
                }
            })
            .detach();
            inputs.insert(key, input);
        }
        let picker = create_credential_picker(
            CredentialPickerConfig::new(field.id.clone(), CredentialCapabilities::login())
                .reference(reference),
            window,
            cx,
        );
        cx.subscribe_in(&picker, window, |_, _, _: &CredentialPickerEvent, _, inner| {
            inner.notify();
        })
        .detach();
        auth_pickers.insert(field.id.clone(), picker);
    }

    pub fn auth_has_reference(&self, field_id: &str, cx: &App) -> bool {
        self.auth_reference(field_id, cx).is_some()
    }

    pub fn auth_reference(&self, field_id: &str, cx: &App) -> Option<CredentialReference> {
        self.auth_pickers
            .get(field_id)
            .and_then(|picker| picker.read(cx).selected_reference())
    }

    /// 回填 Auth 字段的钥匙串引用(编辑/预填模式用)。
    pub fn set_auth_reference(
        &mut self,
        field_id: &str,
        reference: Option<CredentialReference>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(picker) = self.auth_pickers.get(field_id) {
            picker.update(cx, |picker, cx| picker.set_reference(reference, window, cx));
        }
    }

    pub fn auth_value(&self, field_id: &str, part: &str, cx: &App) -> String {
        self.values
            .get(&auth_subkey(field_id, part))
            .map(|value| value.read(cx).clone())
            .unwrap_or_default()
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
            if field.field_type == DeclarativeFieldType::Auth {
                self.collect_auth(field, preserved, &mut config, &mut secrets, cx)?;
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

    fn collect_auth(
        &self,
        field: &DeclarativeFormField,
        preserved: &HashSet<String>,
        config: &mut Map<String, Value>,
        secrets: &mut HashMap<String, String>,
        cx: &App,
    ) -> Result<(), String> {
        let username = self.auth_value(&field.id, "username", cx);
        let password = self.auth_value(&field.id, "password", cx);
        let preserve_password = preserved.contains(&auth_subkey(&field.id, "password"));
        if let Some(reference) = self.auth_reference(&field.id, cx) {
            config.insert(
                field.id.clone(),
                serde_json::json!({ "credential_reference": reference }),
            );
            return Ok(());
        }
        if field.required && password.trim().is_empty() && !preserve_password {
            return Err(format!("{} is required", field.label));
        }
        let mut object = serde_json::Map::new();
        if !username.is_empty() {
            object.insert("username".into(), Value::String(username));
        }
        config.insert(field.id.clone(), Value::Object(object));
        if !password.is_empty() {
            secrets.insert(auth_subkey(&field.id, "password"), password);
        }
        Ok(())
    }

    pub fn visible_secret_ids(&self, cx: &App) -> std::collections::HashSet<String> {
        self.config
            .tabs
            .iter()
            .flat_map(|tab| &tab.fields)
            .filter(|field| field.secret && self.visible(field, cx))
            .map(|field| field.id.clone())
            .chain(
                self.config
                    .tabs
                    .iter()
                    .flat_map(|tab| &tab.fields)
                    .filter(|field| field.field_type == DeclarativeFieldType::Auth && self.visible(field, cx))
                    .map(|field| auth_subkey(&field.id, "password")),
            )
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

    pub fn value(&self, id: &str, cx: &App) -> String {
        self.values
            .get(id)
            .map(|value| value.read(cx).clone())
            .unwrap_or_default()
    }

    pub fn set_field_value(
        &mut self,
        id: &str,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(value_entity) = self.values.get(id) {
            value_entity.update(cx, |current, cx| {
                *current = value.to_string();
                cx.notify();
            });
        }
        if let Some(input) = self.inputs.get(id) {
            input.update(cx, |input, cx| {
                input.set_value(value.to_string(), window, cx);
            });
        } else if let Some(textarea) = self.textareas.get(id) {
            textarea.update(cx, |textarea, cx| {
                textarea.set_value(value.to_string(), window, cx);
            });
        } else if let Some(select) = self.selects.get(id) {
            select.update(cx, |select, cx| {
                select.set_selected_value(&value.to_string(), window, cx);
            });
        }
    }

    /// 渲染前应用 FilePath 浏览结果(需 Window 写回输入态)。
    pub(super) fn apply_pending_file_path(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = self.pending_file_path.update(cx, |pending, _| pending.take());
        if let Some((id, path)) = pending {
            self.set_field_value(&id, &path, window, cx);
        }
    }

    /// FilePath 浏览按钮:弹出文件选择器,结果延后应用。
    pub(super) fn browse_file_path(&mut self, field_id: &str, cx: &mut App) {
        let pending = self.pending_file_path.clone();
        let id = field_id.to_string();
        let future = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            multiple: false,
            directories: false,
            prompt: Some("Select file".into()),
        });
        cx.spawn(async move |cx: &mut AsyncApp| {
            if let Ok(Ok(Some(paths))) = future.await
                && let Some(path) = paths.first()
            {
                let path = path.to_string_lossy().to_string();
                let _ = cx.update(|cx| {
                    pending.update(cx, |pending, cx| {
                        *pending = Some((id, path));
                        cx.notify();
                    });
                });
            }
        })
        .detach();
    }

    pub(super) fn visible(&self, field: &DeclarativeFormField, cx: &App) -> bool {
        field.visible_when.iter().all(|rule| {
            let value = self.value(&rule.field, cx);
            match &rule.equals {
                Some(expected) => value == *expected,
                None => value.trim().is_empty(),
            }
        })
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::declarative::{DECLARATIVE_TEXTAREA_DEFAULT_ROWS, DeclarativeFieldType};

    #[test]
    fn file_path_collects_as_text() {
        assert_eq!(
            Ok(Value::String("C:/data.db".into())),
            typed_value(DeclarativeFieldType::FilePath, "C:/data.db".into())
        );
    }

    #[test]
    fn text_area_default_rows_matches_declarative_engine() {
        let field =
            crate::declarative::DeclarativeFormField::new("body", "", DeclarativeFieldType::TextArea);
        assert_eq!(field.rows, DECLARATIVE_TEXTAREA_DEFAULT_ROWS);
    }
}

#[cfg(test)]
mod auth_tests {
    use gpui::{Context, Entity, Render, TestAppContext, Window, WindowOptions, div};
    use serde_json::{Map, Value, json};

    use super::*;

    struct AuthTestRoot {
        form: Entity<DeclarativeForm>,
    }

    impl Render for AuthTestRoot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl gpui::IntoElement {
            div()
        }
    }

    fn auth_config() -> DeclarativeFormConfig {
        DeclarativeFormConfig {
            tabs: vec![DeclarativeFormTab {
                id: "general".into(),
                label: "General".into(),
                fields: vec![DeclarativeFormField {
                    id: "auth".into(),
                    label: "Authentication".into(),
                    field_type: DeclarativeFieldType::Auth,
                    required: true,
                    default_value: None,
                    placeholder: None,
                    secret: false,
                    options: Vec::new(),
                    visible_when: Vec::new(),
                    rows: 5,
                }],
            }],
        }
    }

    fn open_form(
        cx: &mut TestAppContext,
        initial: Map<String, Value>,
    ) -> gpui::WindowHandle<AuthTestRoot> {
        let config = auth_config();
        cx.update(|cx| {
            cx.set_global(one_core::settings::AppSettings::default());
            gpui_component::init(cx);
            cx.open_window(WindowOptions::default(), |window, cx| {
                cx.new(|cx| {
                    let form =
                        cx.new(|cx| DeclarativeForm::new(config.clone(), &initial, window, cx));
                    AuthTestRoot { form }
                })
            })
        })
        .expect("app context 应可用")
    }

    fn collect_auth(
        window: &gpui::WindowHandle<AuthTestRoot>,
        cx: &mut TestAppContext,
    ) -> (Map<String, Value>, std::collections::HashMap<String, String>) {
        let root = window.root(cx).expect("表单根节点应存在");
        root.read_with(cx, |root, cx| root.form.read(cx).collect(cx))
            .expect("表单应可收集")
    }

    #[gpui::test]
    fn auth_manual_collects_username_and_secret_password(cx: &mut TestAppContext) {
        let initial = json!({ "auth": { "username": "root", "password": "s3cret" } })
            .as_object()
            .cloned()
            .unwrap();
        let window = open_form(cx, initial);
        let (config, secrets) = collect_auth(&window, cx);
        assert_eq!(config["auth"]["username"], "root");
        assert_eq!(secrets.get("auth.password").map(String::as_str), Some("s3cret"));
    }

    #[gpui::test]
    fn auth_reference_collects_reference_and_hides_manual(cx: &mut TestAppContext) {
        let initial = json!({
            "auth": { "credential_reference": { "credential_id": 42, "username": true, "password": true } }
        })
        .as_object()
        .cloned()
        .unwrap();
        let window = open_form(cx, initial);
        let (config, secrets) = collect_auth(&window, cx);
        assert_eq!(config["auth"]["credential_reference"]["credential_id"], 42);
        assert!(secrets.is_empty(), "引用模式不得产出手动密码 secret");
    }

    #[gpui::test]
    fn auth_editing_restores_reference_and_manual_username(cx: &mut TestAppContext) {
        // 编辑已保存连接:引用模式还原钥匙串引用
        let reference = json!({
            "auth": { "credential_reference": { "credential_id": 42, "username": true, "password": true } }
        })
        .as_object()
        .cloned()
        .unwrap();
        let window = open_form(cx, reference);
        window.root(cx).unwrap().read_with(cx, |root, cx| {
            assert!(
                root.form.read(cx).auth_has_reference("auth", cx),
                "编辑回填应还原已选钥匙串引用"
            );
        });

        // 手动模式还原用户名,不进入 secrets(密码靠保留机制)
        let manual = json!({ "auth": { "username": "root" } })
            .as_object()
            .cloned()
            .unwrap();
        let window = open_form(cx, manual);
        window.root(cx).unwrap().read_with(cx, |root, cx| {
            assert!(!root.form.read(cx).auth_has_reference("auth", cx));
            assert_eq!("root", root.form.read(cx).auth_value("auth", "username", cx));
        });
    }
}
