//! Provider Form Dialog - 添加/编辑 LLM Provider 的表单对话框

use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, IntoElement, ParentElement,
    Render, SharedString, Styled, Subscription, WeakEntity, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, IndexPath, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    select::{Select, SelectEvent, SelectItem, SelectState},
    switch::Switch,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::llm::default_api_base;
use one_core::llm::manager::GlobalProviderState;
use one_core::llm::types::{ProviderConfig, ProviderType};
use rust_i18n::t;

const CUSTOM_MODEL_ID: &str = "__custom__";

/// 切换供应商类型后 api_base 输入框应采取的动作。
#[derive(Clone, Debug, PartialEq, Eq)]
enum ApiBaseAction {
    /// 保持用户当前输入不变。
    Keep,
    /// 清空输入框：新类型没有通用默认地址。
    Clear,
    /// 写入新类型的默认地址。
    Set(String),
}

/// 切换供应商类型后 api_base 该显示什么。
///
/// 只在输入框为空、或内容仍等于上一次自动预填值时才改写，
/// 以免切类型时把用户手填的自建 / 代理地址冲掉。
fn api_base_action_on_type_change(
    current: &str,
    last_applied_preset: Option<&str>,
    new_type: ProviderType,
) -> ApiBaseAction {
    let current = current.trim();
    let untouched = current.is_empty() || Some(current) == last_applied_preset;

    if !untouched {
        return ApiBaseAction::Keep;
    }

    match default_api_base(new_type) {
        Some(preset) => ApiBaseAction::Set(preset.to_string()),
        None => ApiBaseAction::Clear,
    }
}

/// Provider 类型选择项
#[derive(Clone, Debug)]
pub struct ProviderTypeItem {
    pub provider_type: ProviderType,
}

impl ProviderTypeItem {
    pub fn new(provider_type: ProviderType) -> Self {
        Self { provider_type }
    }
}

impl SelectItem for ProviderTypeItem {
    type Value = ProviderType;

    fn title(&self) -> SharedString {
        self.provider_type.display_name().into()
    }

    fn value(&self) -> &Self::Value {
        &self.provider_type
    }
}

/// 模型选择项
#[derive(Clone, Debug)]
pub struct ModelItem {
    id: String,
    label: String,
    is_custom: bool,
}

impl ModelItem {
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            is_custom: false,
        }
    }

    pub fn custom() -> Self {
        Self {
            id: CUSTOM_MODEL_ID.to_string(),
            label: t!("LlmProviders.model_custom").to_string(),
            is_custom: true,
        }
    }

    pub fn is_custom(&self) -> bool {
        self.is_custom
    }
}

impl SelectItem for ModelItem {
    type Value = String;

    fn title(&self) -> SharedString {
        self.label.clone().into()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }
}

#[derive(Clone)]
struct ModelRow {
    select: Entity<SelectState<Vec<ModelItem>>>,
    custom_input: Entity<InputState>,
}

#[derive(Clone)]
enum ModelLoadStatusKind {
    Success,
    Warning,
    Error,
}

#[derive(Clone)]
struct ModelLoadStatus {
    kind: ModelLoadStatusKind,
    message: String,
}

impl ModelLoadStatus {
    fn success(message: String) -> Self {
        Self {
            kind: ModelLoadStatusKind::Success,
            message,
        }
    }

    fn warning(message: String) -> Self {
        Self {
            kind: ModelLoadStatusKind::Warning,
            message,
        }
    }

    fn error(message: String) -> Self {
        Self {
            kind: ModelLoadStatusKind::Error,
            message,
        }
    }
}

/// Provider 表单对话框
pub struct ProviderForm {
    focus_handle: FocusHandle,
    provider_id: Option<i64>,
    name_input: Entity<InputState>,
    provider_type_select: Entity<SelectState<Vec<ProviderTypeItem>>>,
    api_key_input: Entity<InputState>,
    api_base_input: Entity<InputState>,
    api_version_input: Entity<InputState>,
    model_items: Vec<ModelItem>,
    model_rows: Vec<ModelRow>,
    is_default: bool,
    models_loading: bool,
    model_load_status: Option<ModelLoadStatus>,
    /// 当前 api_base 输入框里是不是上一次自动预填的值（用于判断能否安全改写）。
    last_applied_preset: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProviderForm {
    pub fn new_with_config(
        config: Option<ProviderConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        // 创建 provider 类型选择器：新增与编辑共用同一份类型清单。
        let types = ProviderType::all();
        let provider_types: Vec<ProviderTypeItem> =
            types.into_iter().map(ProviderTypeItem::new).collect();

        let selected_index = if let Some(ref cfg) = config {
            provider_types
                .iter()
                .position(|item| item.provider_type == cfg.provider_type)
                .map(|i| IndexPath::new(i))
        } else {
            Some(IndexPath::new(0))
        };

        // 与上面 selected_index 的取值来源保持一致，但不依赖 IndexPath 内部字段。
        let selected_type = match config.as_ref() {
            Some(cfg) => cfg.provider_type,
            None => provider_types
                .first()
                .map(|item| item.provider_type)
                .unwrap_or(ProviderType::OpenAI),
        };

        let provider_type_select =
            cx.new(|cx| SelectState::new(provider_types, selected_index, window, cx));

        // 创建输入框
        let name_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.name_placeholder"));
            if let Some(ref cfg) = config {
                state = state.default_value(&cfg.name);
            }
            state
        });

        let api_key_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .masked(true)
                .placeholder(t!("LlmProviders.api_key_placeholder"));
            if let Some(ref cfg) = config {
                if let Some(ref key) = cfg.api_key {
                    state = state.default_value(key);
                }
            }
            state
        });

        let api_base_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.api_base_placeholder"));
            if let Some(ref cfg) = config {
                if let Some(ref base) = cfg.api_base {
                    state = state.default_value(base);
                }
            }
            state
        });

        let api_version_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.api_version_placeholder"));
            if let Some(ref cfg) = config {
                if let Some(ref version) = cfg.api_version {
                    state = state.default_value(version);
                }
            }
            state
        });

        let config_models = config.as_ref().map(|cfg| {
            if cfg.models.is_empty() {
                vec![cfg.model.clone()]
            } else {
                cfg.models.clone()
            }
        });
        let initial_models = config_models.unwrap_or_default();
        let model_items = Self::build_model_items(&initial_models);
        let model_rows = if initial_models.is_empty() {
            vec![Self::create_model_row_with_items(
                None,
                &model_items,
                window,
                cx,
            )]
        } else {
            initial_models
                .iter()
                .map(|model| {
                    Self::create_model_row_with_items(Some(model.clone()), &model_items, window, cx)
                })
                .collect()
        };

        // 编辑既有配置时，如果存下来的 api_base 正好等于该类型的默认值，
        // 就认为它仍是「未被改过的预填值」，后续切类型可以安全改写。
        let last_applied_preset = config
            .as_ref()
            .and_then(|cfg| cfg.api_base.clone())
            .filter(|base| default_api_base(selected_type) == Some(base.as_str()));

        // 切换供应商类型时把默认地址预填进 api_base，让用户知道该填什么。
        let subscription = cx.subscribe_in(
            &provider_type_select,
            window,
            |form: &mut Self, _, event: &SelectEvent<Vec<ProviderTypeItem>>, window, cx| {
                let SelectEvent::Confirm(Some(provider_type)) = event else {
                    return;
                };

                let current = form.api_base_input.read(cx).value().to_string();
                let action = api_base_action_on_type_change(
                    &current,
                    form.last_applied_preset.as_deref(),
                    *provider_type,
                );

                match action {
                    ApiBaseAction::Keep => {}
                    ApiBaseAction::Clear => {
                        form.api_base_input.update(cx, |input, cx| {
                            input.set_value("", window, cx);
                        });
                        form.last_applied_preset = None;
                    }
                    ApiBaseAction::Set(preset) => {
                        form.api_base_input.update(cx, |input, cx| {
                            input.set_value(preset.clone(), window, cx);
                        });
                        form.last_applied_preset = Some(preset);
                    }
                }

                cx.notify();
            },
        );

        Self {
            focus_handle,
            provider_id: config.clone().map(|c| c.id),
            name_input,
            provider_type_select,
            api_key_input,
            api_base_input,
            api_version_input,
            model_items,
            model_rows,
            is_default: config.map(|cfg| cfg.is_default).unwrap_or(false),
            models_loading: false,
            model_load_status: None,
            last_applied_preset,
            _subscriptions: vec![subscription],
        }
    }

    fn build_model_items(models: &[String]) -> Vec<ModelItem> {
        let mut items = Vec::new();
        for model in models {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                continue;
            }
            if items.iter().any(|item: &ModelItem| item.id == trimmed) {
                continue;
            }
            items.push(ModelItem::new(trimmed.to_string(), trimmed.to_string()));
        }
        items.push(ModelItem::custom());
        items
    }

    fn create_model_row_with_items(
        selected: Option<String>,
        items: &[ModelItem],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ModelRow {
        let selected_value = selected.unwrap_or_default();
        let custom_index = items.iter().position(|item| item.is_custom()).unwrap_or(0);
        let selected_index = if !selected_value.is_empty() {
            items
                .iter()
                .position(|item| item.id == selected_value)
                .or(Some(custom_index))
                .map(IndexPath::new)
        } else {
            Some(IndexPath::new(0))
        };

        let select = cx.new(|cx| SelectState::new(items.to_vec(), selected_index, window, cx));
        let custom_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.model_placeholder"));
            if !selected_value.is_empty() && items.iter().all(|item| item.id != selected_value) {
                state = state.default_value(&selected_value);
            }
            state
        });

        ModelRow {
            select,
            custom_input,
        }
    }

    fn create_model_row(
        &self,
        selected: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> ModelRow {
        let items = if self.model_items.is_empty() {
            vec![ModelItem::custom()]
        } else {
            self.model_items.clone()
        };
        Self::create_model_row_with_items(selected, &items, window, cx)
    }

    fn collect_models(&self, cx: &mut Context<Self>) -> Vec<String> {
        let mut models = Vec::new();
        for row in &self.model_rows {
            let selected = row.select.read(cx).selected_value().cloned();
            let value = match selected {
                Some(model_id) if model_id == CUSTOM_MODEL_ID => {
                    row.custom_input.read(cx).value().to_string()
                }
                Some(model_id) => model_id,
                None => continue,
            };
            let trimmed = value.trim();
            if trimmed.is_empty() {
                continue;
            }
            if models.iter().any(|model| model == trimmed) {
                continue;
            }
            models.push(trimmed.to_string());
        }
        models
    }

    fn add_model_row(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let row = self.create_model_row(None, window, cx);
        self.model_rows.push(row);
        cx.notify();
    }

    fn remove_model_row(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if self.model_rows.len() <= 1 {
            self.model_rows.clear();
            self.model_rows
                .push(self.create_model_row(None, window, cx));
            cx.notify();
            return;
        }
        if index < self.model_rows.len() {
            self.model_rows.remove(index);
            cx.notify();
        }
    }

    fn refresh_model_items(
        &mut self,
        mut items: Vec<ModelItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if items.iter().all(|item| !item.is_custom()) {
            items.push(ModelItem::custom());
        }

        let custom_index = items.iter().position(|item| item.is_custom()).unwrap_or(0);
        self.model_items = items.clone();

        if self.model_rows.is_empty() {
            self.model_rows
                .push(Self::create_model_row_with_items(None, &items, window, cx));
            return;
        }

        for row in &self.model_rows {
            let selected = row.select.read(cx).selected_value().cloned();
            row.select.update(cx, |state, cx| {
                state.set_items(items.clone(), window, cx);
                let index = match selected.as_ref() {
                    Some(value) => items
                        .iter()
                        .position(|item| item.id == *value)
                        .unwrap_or(custom_index),
                    None => 0,
                };
                state.set_selected_index(Some(IndexPath::new(index)), window, cx);
            });

            if let Some(value) = selected {
                if value != CUSTOM_MODEL_ID && items.iter().all(|item| item.id != value) {
                    row.custom_input.update(cx, |input, cx| {
                        input.set_value(value.clone(), window, cx);
                    });
                }
            }
        }
    }

    fn load_models(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if self.models_loading {
            return;
        }

        self.models_loading = true;
        self.model_load_status = None;
        cx.notify();

        let provider_type = self
            .provider_type_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or(ProviderType::OpenAI);
        let name = self.name_input.read(cx).value().to_string();
        let api_key = self.api_key_input.read(cx).value().to_string();
        let api_base = self.api_base_input.read(cx).value().to_string();
        let api_version = self.api_version_input.read(cx).value().to_string();
        let mut models = self.collect_models(cx);
        let fallback_model = models
            .first()
            .cloned()
            .unwrap_or_else(|| "gpt-4".to_string());
        if models.is_empty() {
            models.push(fallback_model.clone());
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间不应早于 UNIX 纪元")
            .as_secs() as i64;

        let config = ProviderConfig {
            id: 0,
            name,
            provider_type,
            api_key: if api_key.is_empty() {
                None
            } else {
                Some(api_key)
            },
            api_base: if api_base.is_empty() {
                None
            } else {
                Some(api_base)
            },
            api_version: if api_version.is_empty() {
                None
            } else {
                Some(api_version)
            },
            model: fallback_model,
            models,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            enabled: true,
            is_default: false,
            created_at: now,
            updated_at: now,
        };

        let global_provider_state = cx.global::<GlobalProviderState>().clone();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = Tokio::spawn(cx, async move {
                let provider = global_provider_state
                    .manager()
                    .get_provider(&config)
                    .await?;
                provider.models().await
            })
            .await;

            let result: Result<Vec<String>, String> = match result {
                Ok(Ok(models)) => Ok(models),
                Ok(Err(e)) => Err(e.to_string()),
                Err(e) => Err(t!("LlmProviders.model_task_failed", error = e).to_string()),
            };

            let _ = cx.update(|cx| {
                if let Some(window_id) = cx.active_window() {
                    let _ = cx.update_window(window_id, |_, window, cx| {
                        if let Some(entity) = this.upgrade() {
                            entity.update(cx, |form, cx| {
                                form.models_loading = false;
                                match result {
                                    Ok(models) => {
                                        let loaded_count = models.len();
                                        if models.is_empty() {
                                            window.push_notification(
                                                t!("LlmProviders.model_list_empty").to_string(),
                                                cx,
                                            );
                                            form.model_load_status =
                                                Some(ModelLoadStatus::warning(
                                                    t!("LlmProviders.model_load_empty_inline")
                                                        .to_string(),
                                                ));
                                        } else {
                                            let message = t!(
                                                "LlmProviders.model_load_success",
                                                count = loaded_count
                                            )
                                            .to_string();
                                            window.push_notification(message.clone(), cx);
                                            form.model_load_status =
                                                Some(ModelLoadStatus::success(message));
                                        }
                                        let items = Self::build_model_items(&models);
                                        form.refresh_model_items(items, window, cx);
                                    }
                                    Err(message) => {
                                        let message =
                                            t!("LlmProviders.model_load_failed", error = message)
                                                .to_string();
                                        window.push_notification(message.clone(), cx);
                                        form.model_load_status =
                                            Some(ModelLoadStatus::error(message));
                                    }
                                }
                                cx.notify();
                            });
                        }
                    });
                }
            });
        })
        .detach();
    }

    pub fn get_config(&mut self, cx: &mut Context<Self>) -> Option<ProviderConfig> {
        let name = self.name_input.read(cx).value().to_string();
        let provider_type = self
            .provider_type_select
            .read(cx)
            .selected_value()
            .cloned()
            .unwrap_or(ProviderType::OpenAI);
        let api_key = self.api_key_input.read(cx).value().to_string();
        let api_base = self.api_base_input.read(cx).value().to_string();
        let api_version = self.api_version_input.read(cx).value().to_string();
        let models = self.collect_models(cx);

        if name.trim().is_empty() {
            tracing::warn!("{}", t!("LlmProviders.name_required"));
            return None;
        }

        if models.is_empty() {
            tracing::warn!("{}", t!("LlmProviders.model_required"));
            return None;
        }

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("系统时间不应早于 UNIX 纪元")
            .as_secs() as i64;

        let model = models[0].clone();

        Some(ProviderConfig {
            id: self.provider_id.unwrap_or(now),
            name,
            provider_type,
            api_key: if api_key.is_empty() {
                None
            } else {
                Some(api_key)
            },
            api_base: if api_base.is_empty() {
                None
            } else {
                Some(api_base)
            },
            api_version: if api_version.is_empty() {
                None
            } else {
                Some(api_version)
            },
            model,
            models,
            max_tokens: Some(4096),
            temperature: Some(0.7),
            enabled: true,
            is_default: self.is_default,
            created_at: now,
            updated_at: now,
        })
    }
}
impl Focusable for ProviderForm {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ProviderForm {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let mut model_rows = v_flex().gap_2();
        for (index, row) in self.model_rows.iter().enumerate() {
            let selected = row.select.read(cx).selected_value().cloned();
            let is_custom = matches!(selected.as_deref(), Some(CUSTOM_MODEL_ID));

            let mut row_view = h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().child(Select::new(&row.select)));

            if is_custom {
                row_view = row_view.child(div().flex_1().child(Input::new(&row.custom_input)));
            }

            row_view = row_view.child(
                Button::new(SharedString::from(format!("remove-model-{}", index)))
                    .with_variant(ButtonVariant::Secondary)
                    .label(t!("LlmProviders.model_remove"))
                    .disabled(self.model_rows.len() <= 1)
                    .on_click(cx.listener(move |view, _, window, cx| {
                        view.remove_model_row(index, window, cx);
                    })),
            );

            model_rows = model_rows.child(row_view);
        }

        let load_label = if self.models_loading {
            t!("LlmProviders.model_loading")
        } else {
            t!("LlmProviders.model_load")
        };

        v_flex()
            .gap_3()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.name_label").to_string()),
                    )
                    .child(Input::new(&self.name_input)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.provider_type_label").to_string()),
                    )
                    .child(Select::new(&self.provider_type_select)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.api_key_label").to_string()),
                    )
                    .child(Input::new(&self.api_key_input).mask_toggle()),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.api_base_label").to_string()),
                    )
                    .child(Input::new(&self.api_base_input)),
            )
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.api_version_label").to_string()),
                    )
                    .child(Input::new(&self.api_version_input)),
            )
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .child(t!("LlmProviders.models_label").to_string()),
                    )
                    .child(model_rows)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("load-models")
                                    .with_variant(ButtonVariant::Secondary)
                                    .label(load_label)
                                    .disabled(self.models_loading)
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.load_models(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("add-model")
                                    .with_variant(ButtonVariant::Secondary)
                                    .label(t!("LlmProviders.model_add"))
                                    .on_click(cx.listener(|view, _, window, cx| {
                                        view.add_model_row(window, cx);
                                    })),
                            ),
                    )
                    .when_some(self.model_load_status.clone(), |this, status| {
                        let color = match status.kind {
                            ModelLoadStatusKind::Success => cx.theme().success,
                            ModelLoadStatusKind::Warning => cx.theme().warning,
                            ModelLoadStatusKind::Error => cx.theme().danger,
                        };
                        this.child(div().text_sm().text_color(color).child(status.message))
                    }),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Switch::new("default-provider-switch")
                            .checked(self.is_default)
                            .on_click(cx.listener(|view, checked, _, cx| {
                                view.is_default = *checked;
                                cx.notify();
                            })),
                    )
                    .child(t!("LlmProviders.set_default").to_string()),
            )
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiBaseAction, api_base_action_on_type_change};
    use one_core::llm::types::ProviderType;

    #[test]
    fn empty_input_is_filled_with_the_new_type_preset() {
        assert_eq!(
            ApiBaseAction::Set("https://openrouter.ai/api/v1".to_string()),
            api_base_action_on_type_change("", None, ProviderType::OpenRouter)
        );
    }

    #[test]
    fn whitespace_only_input_counts_as_empty() {
        assert_eq!(
            ApiBaseAction::Set("https://api.siliconflow.cn/v1".to_string()),
            api_base_action_on_type_change("   ", None, ProviderType::SiliconFlow)
        );
    }

    #[test]
    fn untouched_previous_preset_is_replaced_by_the_new_one() {
        assert_eq!(
            ApiBaseAction::Set("https://api.siliconflow.cn/v1".to_string()),
            api_base_action_on_type_change(
                "https://openrouter.ai/api/v1",
                Some("https://openrouter.ai/api/v1"),
                ProviderType::SiliconFlow,
            )
        );
    }

    #[test]
    fn switching_to_a_type_without_preset_clears_an_untouched_preset() {
        assert_eq!(
            ApiBaseAction::Clear,
            api_base_action_on_type_change(
                "https://openrouter.ai/api/v1",
                Some("https://openrouter.ai/api/v1"),
                ProviderType::OpenAICompatible,
            )
        );
    }

    #[test]
    fn hand_written_base_url_is_never_overwritten() {
        for new_type in [
            ProviderType::OpenRouter,
            ProviderType::OpenAICompatible,
            ProviderType::SiliconFlow,
        ] {
            assert_eq!(
                ApiBaseAction::Keep,
                api_base_action_on_type_change(
                    "https://gateway.internal.example.com/v1",
                    Some("https://openrouter.ai/api/v1"),
                    new_type,
                ),
                "切到 {new_type:?} 时不应覆盖用户手填地址"
            );
        }
    }
}
