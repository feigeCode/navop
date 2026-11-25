//! Provider 详情面板：设置页右侧的内嵌编辑区（不再用弹窗）。
//!
//! 结构：头部（品牌块 + 名称 + 类型 + 启用 + 测试连接）→ 基础信息 / 连接 / 模型 /
//! 高级（可折叠）→ 底部保存栏。保存由外层 [`crate::settings::llm_providers_view`]
//! 落库，这里只负责取值与校验。

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Disableable, IndexPath, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    checkbox::Checkbox,
    h_flex,
    input::{Input, InputEvent, InputState, Textarea, TextareaState},
    scroll::{Scrollbar, ScrollbarMode},
    select::{Select, SelectEvent, SelectItem, SelectState},
    switch::Switch,
    v_flex,
};
use one_assets::IconName;
use one_core::gpui_tokio::Tokio;
use one_core::llm::default_api_base;
use one_core::llm::manager::GlobalProviderState;
use one_core::llm::types::{ApiMode, ProviderConfig, ProviderType, ReasoningEffort};
use rust_i18n::t;

use super::provider_badge::{provider_accent, provider_badge};

/// 详情面板对外的三种意图；落库、确认删除由父视图处理。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderDetailEvent {
    /// 用户点了保存，父视图应调用 [`ProviderDetail::get_config`] 取值。
    Submit,
    /// 用户点了取消，父视图应丢弃当前草稿。
    Cancel,
    /// 用户点了删除（会先弹确认框）。
    Delete,
}

fn is_valid_header_name(name: &str) -> bool {
    !name.is_empty()
        && name.bytes().all(|byte| {
            byte.is_ascii_alphanumeric()
                || matches!(
                    byte,
                    b'!' | b'#'
                        | b'$'
                        | b'%'
                        | b'&'
                        | b'\''
                        | b'*'
                        | b'+'
                        | b'-'
                        | b'.'
                        | b'^'
                        | b'_'
                        | b'`'
                        | b'|'
                        | b'~'
                )
        })
}

fn parse_header_lines(input: &str) -> Result<Vec<(String, String)>, String> {
    let mut headers: Vec<(String, String)> = Vec::new();

    for (line_index, raw_line) in input.lines().enumerate() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((name, value)) = line.split_once(':') else {
            return Err(format!(
                "Line {} must use the format 'Header-Name: value'",
                line_index + 1
            ));
        };
        let name = name.trim();
        let value = value.trim();
        if !is_valid_header_name(name) {
            return Err(format!(
                "Line {} has an invalid header name",
                line_index + 1
            ));
        }

        if let Some((_, existing_value)) = headers
            .iter_mut()
            .find(|(existing_name, _)| existing_name.eq_ignore_ascii_case(name))
        {
            *existing_value = value.to_string();
        } else {
            headers.push((name.to_string(), value.to_string()));
        }
    }

    Ok(headers)
}

fn format_header_lines(headers: &[(String, String)]) -> String {
    headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// 解析「自定义请求头」输入。
///
/// 请求头对所有供应商类型都生效：`LlmConnector` 会把它们并入每次聊天请求，
/// 因此这里不再按类型丢弃用户输入。
fn parse_headers_for_provider(
    _provider_type: ProviderType,
    input: &str,
) -> Result<Vec<(String, String)>, String> {
    parse_header_lines(input)
}

fn filter_loaded_models(provider_type: ProviderType, models: Vec<String>) -> Vec<String> {
    if provider_type != ProviderType::OpenCodeGo {
        return models;
    }

    models
        .into_iter()
        .filter(|model| {
            !matches!(
                model.to_ascii_lowercase().as_str(),
                "beam-free" | "iceman-contributor-free"
            )
        })
        .collect()
}

/// 合并模型目录：保留原有顺序，追加新出现的模型（去重、去空白）。
pub fn merge_models(existing: &[String], incoming: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = existing
        .iter()
        .map(|model| model.trim().to_string())
        .filter(|model| !model.is_empty())
        .collect();
    for model in incoming {
        let model = model.trim();
        if model.is_empty() || merged.iter().any(|existing| existing == model) {
            continue;
        }
        merged.push(model.to_string());
    }
    merged
}

/// 模型列表的搜索过滤：大小写不敏感的子串匹配。
pub fn filter_models<'a>(models: &'a [String], query: &str) -> Vec<&'a String> {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return models.iter().collect();
    }
    models
        .iter()
        .filter(|model| model.to_ascii_lowercase().contains(&query))
        .collect()
}

/// 勾选 / 取消勾选一个模型，保持原有顺序。
pub fn toggle_model(selected: &mut Vec<String>, model: &str, checked: bool) {
    let model = model.trim();
    if model.is_empty() {
        return;
    }
    match (checked, selected.iter().any(|item| item == model)) {
        (true, false) => selected.push(model.to_string()),
        (false, true) => selected.retain(|item| item != model),
        _ => {}
    }
}

/// 批量选择动作；作用范围是**当前过滤结果**，这样「全选」不会把搜索外的模型
/// 也一起勾上。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SelectionAction {
    /// 勾选过滤结果里的全部模型。
    SelectAll,
    /// 取消勾选过滤结果里的全部模型。
    Clear,
    /// 反转过滤结果里每个模型的勾选状态。
    Invert,
}

/// 对 `query` 过滤出的模型批量应用 `action`，原地更新 `selected`。
fn apply_selection_action(
    available: &[String],
    query: &str,
    selected: &mut Vec<String>,
    action: SelectionAction,
) {
    let scope: Vec<String> = filter_models(available, query)
        .into_iter()
        .cloned()
        .collect();
    for model in scope {
        match action {
            SelectionAction::SelectAll => toggle_model(selected, &model, true),
            SelectionAction::Clear => toggle_model(selected, &model, false),
            SelectionAction::Invert => {
                let checked = selected.iter().any(|item| item == &model);
                toggle_model(selected, &model, !checked);
            }
        }
    }
}

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

/// 思考级别选择项。`None` 表示「自动」：请求里不写 `reasoning_effort`。
#[derive(Clone, Debug)]
pub struct EffortItem {
    effort: Option<ReasoningEffort>,
}

impl EffortItem {
    pub fn new(effort: Option<ReasoningEffort>) -> Self {
        Self { effort }
    }
}

impl SelectItem for EffortItem {
    type Value = Option<ReasoningEffort>;

    fn title(&self) -> SharedString {
        reasoning_effort_label(self.effort)
    }

    fn value(&self) -> &Self::Value {
        &self.effort
    }
}

/// 思考级别文案；「自动」与具体档位分开取文案，便于各语言分别措辞。
fn reasoning_effort_label(effort: Option<ReasoningEffort>) -> SharedString {
    match effort {
        None => t!("LlmProviders.reasoning_effort_auto").to_string().into(),
        Some(ReasoningEffort::None) => t!("LlmProviders.reasoning_effort_none").to_string().into(),
        Some(ReasoningEffort::Low) => t!("LlmProviders.reasoning_effort_low").to_string().into(),
        Some(ReasoningEffort::Medium) => t!("LlmProviders.reasoning_effort_medium")
            .to_string()
            .into(),
        Some(ReasoningEffort::High) => t!("LlmProviders.reasoning_effort_high").to_string().into(),
    }
}

/// 「接口协议」选择项：`/chat/completions` 或 `/responses`（issue #565）。
#[derive(Clone, Debug)]
pub struct ApiModeItem {
    api_mode: ApiMode,
}

impl ApiModeItem {
    pub fn new(api_mode: ApiMode) -> Self {
        Self { api_mode }
    }
}

impl SelectItem for ApiModeItem {
    type Value = ApiMode;

    fn title(&self) -> SharedString {
        api_mode_label(self.api_mode)
    }

    fn value(&self) -> &Self::Value {
        &self.api_mode
    }
}

fn api_mode_label(api_mode: ApiMode) -> SharedString {
    match api_mode {
        ApiMode::ChatCompletions => t!("LlmProviders.api_mode_chat_completions")
            .to_string()
            .into(),
        ApiMode::Responses => t!("LlmProviders.api_mode_responses").to_string().into(),
    }
}

/// 「接口协议」下拉的全部选项：先 chat/completions（默认），后 responses。
fn api_mode_items() -> Vec<ApiModeItem> {
    ApiMode::all().into_iter().map(ApiModeItem::new).collect()
}

/// 思考级别下拉的全部档位：自动 / 关闭 / 低 / 中 / 高。
fn reasoning_effort_items() -> Vec<EffortItem> {
    [
        None,
        Some(ReasoningEffort::None),
        Some(ReasoningEffort::Low),
        Some(ReasoningEffort::Medium),
        Some(ReasoningEffort::High),
    ]
    .into_iter()
    .map(EffortItem::new)
    .collect()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StatusKind {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug)]
struct Status {
    kind: StatusKind,
    message: String,
}

impl Status {
    fn success(message: String) -> Self {
        Self {
            kind: StatusKind::Success,
            message,
        }
    }

    fn warning(message: String) -> Self {
        Self {
            kind: StatusKind::Warning,
            message,
        }
    }

    fn error(message: String) -> Self {
        Self {
            kind: StatusKind::Error,
            message,
        }
    }
}

/// Provider 详情面板（设置页右栏）。
pub struct ProviderDetail {
    focus_handle: FocusHandle,
    provider_id: Option<i64>,
    created_at: i64,
    max_tokens: Option<i32>,
    temperature: Option<f32>,
    name_input: Entity<InputState>,
    provider_type_select: Entity<SelectState<Vec<ProviderTypeItem>>>,
    api_key_input: Entity<InputState>,
    api_base_input: Entity<InputState>,
    api_version_input: Entity<InputState>,
    /// 思考级别选择器：请求里要不要带 `reasoning_effort` 以及带哪一档。
    effort_select: Entity<SelectState<Vec<EffortItem>>>,
    /// 「接口协议」：`/chat/completions` 或 `/responses`。
    api_mode_select: Entity<SelectState<Vec<ApiModeItem>>>,
    extra_headers_input: Entity<TextareaState>,
    /// 供应商级代理地址；留空表示跟随全局代理。
    proxy_input: Entity<InputState>,
    model_filter_input: Entity<InputState>,
    model_filter: String,
    custom_model_input: Entity<InputState>,
    /// 模型目录（拉取到的 + 手动添加的）。
    available_models: Vec<String>,
    /// 勾选进 `ProviderConfig.models` 的模型。
    selected_models: Vec<String>,
    /// 默认模型，必须是 `selected_models` 里的一个。
    default_model: String,
    is_default: bool,
    enabled: bool,
    models_loading: bool,
    /// 详情主体与模型列表各自的滚动句柄，用来给 `overflow_y_scroll` 挂可见滚动条。
    body_scroll: ScrollHandle,
    model_scroll: ScrollHandle,
    status: Option<Status>,
    validation_error: Option<SharedString>,
    /// 当前 api_base 输入框里是不是上一次自动预填的值（用于判断能否安全改写）。
    last_applied_preset: Option<String>,
    _subscriptions: Vec<Subscription>,
}

impl ProviderDetail {
    pub fn new(
        config: Option<ProviderConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(config, None, window, cx)
    }

    /// 从空态画廊的预设卡片新建：带出名称与默认 Base URL。
    pub fn for_preset(
        provider_type: ProviderType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        Self::build(None, Some(provider_type), window, cx)
    }

    fn build(
        config: Option<ProviderConfig>,
        preset: Option<ProviderType>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let focus_handle = cx.focus_handle();
        let provider_types: Vec<ProviderTypeItem> = ProviderType::all()
            .into_iter()
            .map(ProviderTypeItem::new)
            .collect();

        let selected_type = config
            .as_ref()
            .map(|cfg| cfg.provider_type)
            .or(preset)
            .or_else(|| provider_types.first().map(|item| item.provider_type))
            .unwrap_or(ProviderType::OpenAI);

        let type_index = provider_types
            .iter()
            .position(|item| item.provider_type == selected_type)
            .or(Some(0))
            .map(IndexPath::new);
        let provider_type_select =
            cx.new(|cx| SelectState::new(provider_types, type_index, window, cx));

        let default_base = default_api_base(selected_type)
            .map(str::to_string)
            .or_else(|| config.as_ref().and_then(|cfg| cfg.api_base.clone()));

        let name_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.name_placeholder"));
            if let Some(ref cfg) = config {
                state = state.default_value(&cfg.name);
            } else if let Some(provider_type) = preset {
                state = state.default_value(provider_type.display_name());
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
            if let Some(base) = default_base.clone() {
                state = state.default_value(base);
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

        let effort_items = reasoning_effort_items();
        let effort_index = config
            .as_ref()
            .and_then(|cfg| cfg.reasoning_effort)
            .and_then(|effort| {
                effort_items
                    .iter()
                    .position(|item| item.effort == Some(effort))
            })
            .map(IndexPath::new)
            // 没配置过（或配置值不可识别）时落在「自动」，与上游默认语义一致。
            .or(Some(IndexPath::new(0)));
        let effort_select = cx.new(|cx| SelectState::new(effort_items, effort_index, window, cx));

        let api_mode_options = api_mode_items();
        let api_mode_index = config
            .as_ref()
            .map(|cfg| cfg.api_mode)
            .and_then(|mode| {
                api_mode_options
                    .iter()
                    .position(|item| item.api_mode == mode)
            })
            .map(IndexPath::new)
            // 老配置没有该字段，落在默认的 chat/completions。
            .or(Some(IndexPath::new(0)));
        let api_mode_select =
            cx.new(|cx| SelectState::new(api_mode_options, api_mode_index, window, cx));

        let extra_headers_input = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx)
                .auto_grow(2, 7)
                .placeholder(t!("LlmProviders.extra_headers_placeholder"));
            if let Some(ref cfg) = config {
                if !cfg.extra_headers.is_empty() {
                    state = state.default_value(format_header_lines(&cfg.extra_headers));
                }
            }
            state
        });

        let proxy_input = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("LlmProviders.proxy_placeholder"));
            if let Some(ref cfg) = config
                && let Some(ref proxy) = cfg.proxy
            {
                state = state.default_value(proxy);
            }
            state
        });

        let model_filter_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("LlmProviders.model_search_placeholder"))
        });
        let custom_model_input = cx.new(|cx| {
            InputState::new(window, cx).placeholder(t!("LlmProviders.model_placeholder"))
        });

        // 已有配置：模型列表按保存时的勾选还原；`models` 为空的老数据退回 `model`。
        let (available_models, selected_models, default_model) = match config.as_ref() {
            Some(cfg) => {
                let models = if cfg.models.is_empty() {
                    vec![cfg.model.clone()]
                } else {
                    cfg.models.clone()
                };
                let models = merge_models(&[], &models);
                let default = if models.iter().any(|model| model == &cfg.model) {
                    cfg.model.clone()
                } else {
                    models.first().cloned().unwrap_or_default()
                };
                (models.clone(), models, default)
            }
            None => (Vec::new(), Vec::new(), String::new()),
        };

        let last_applied_preset = default_base
            .clone()
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

                // 类型切换后「额外请求头」是否展示会变，重新渲染。
                cx.notify();
            },
        );

        let model_filter_subscription = cx.subscribe(
            &model_filter_input,
            |form: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    form.model_filter = form.model_filter_input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );

        let now = unix_now();

        Self {
            focus_handle,
            provider_id: config.as_ref().map(|cfg| cfg.id),
            created_at: config.as_ref().map(|cfg| cfg.created_at).unwrap_or(now),
            max_tokens: config
                .as_ref()
                .and_then(|cfg| cfg.max_tokens)
                .or(Some(4096)),
            temperature: config
                .as_ref()
                .and_then(|cfg| cfg.temperature)
                .or(Some(0.7)),
            name_input,
            provider_type_select,
            api_key_input,
            api_base_input,
            api_version_input,
            effort_select,
            api_mode_select,
            extra_headers_input,
            proxy_input,
            model_filter_input,
            model_filter: String::new(),
            custom_model_input,
            available_models,
            selected_models,
            default_model,
            is_default: config.as_ref().map(|cfg| cfg.is_default).unwrap_or(false),
            enabled: config.as_ref().map(|cfg| cfg.enabled).unwrap_or(true),
            models_loading: false,
            body_scroll: ScrollHandle::new(),
            model_scroll: ScrollHandle::new(),
            status: None,
            validation_error: None,
            last_applied_preset,
            _subscriptions: vec![subscription, model_filter_subscription],
        }
    }

    fn selected_type(&self, cx: &App) -> ProviderType {
        self.provider_type_select
            .read(cx)
            .selected_value()
            .copied()
            .unwrap_or(ProviderType::OpenAI)
    }

    fn set_model_checked(&mut self, model: &str, checked: bool, cx: &mut Context<Self>) {
        toggle_model(&mut self.selected_models, model, checked);
        if !self
            .selected_models
            .iter()
            .any(|item| item == &self.default_model)
        {
            self.default_model = self.selected_models.first().cloned().unwrap_or_default();
        }
        self.validation_error = None;
        cx.notify();
    }

    fn set_default_model(&mut self, model: &str, cx: &mut Context<Self>) {
        if !self.selected_models.iter().any(|item| item == model) {
            toggle_model(&mut self.selected_models, model, true);
        }
        self.default_model = model.to_string();
        cx.notify();
    }

    fn select_all_models(&mut self, cx: &mut Context<Self>) {
        apply_selection_action(
            &self.available_models,
            &self.model_filter,
            &mut self.selected_models,
            SelectionAction::SelectAll,
        );
        self.reconcile_default_model();
        cx.notify();
    }

    fn clear_models(&mut self, cx: &mut Context<Self>) {
        apply_selection_action(
            &self.available_models,
            &self.model_filter,
            &mut self.selected_models,
            SelectionAction::Clear,
        );
        self.reconcile_default_model();
        cx.notify();
    }

    fn invert_models(&mut self, cx: &mut Context<Self>) {
        apply_selection_action(
            &self.available_models,
            &self.model_filter,
            &mut self.selected_models,
            SelectionAction::Invert,
        );
        self.reconcile_default_model();
        cx.notify();
    }

    /// 默认模型必须始终是已勾选的模型；勾选集合变了就顺手纠正。
    fn reconcile_default_model(&mut self) {
        if self
            .selected_models
            .iter()
            .any(|item| item == &self.default_model)
        {
            return;
        }
        self.default_model = self.selected_models.first().cloned().unwrap_or_default();
    }

    fn add_custom_model(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let value = self.custom_model_input.read(cx).value().trim().to_string();
        if value.is_empty() {
            return;
        }
        self.available_models = merge_models(&self.available_models, std::slice::from_ref(&value));
        toggle_model(&mut self.selected_models, &value, true);
        if self.default_model.is_empty() {
            self.default_model = value;
        }
        self.custom_model_input.update(cx, |input, cx| {
            input.set_value("", window, cx);
        });
        cx.notify();
    }

    fn current_config(
        &mut self,
        cx: &mut Context<Self>,
        require_models: bool,
    ) -> Option<ProviderConfig> {
        let provider_type = self.selected_type(cx);
        let name = self.name_input.read(cx).value().trim().to_string();
        if name.is_empty() {
            self.validation_error = Some(t!("LlmProviders.name_required").to_string().into());
            cx.notify();
            return None;
        }

        let api_key = self.api_key_input.read(cx).value().to_string();
        let api_base = self.api_base_input.read(cx).value().to_string();
        let api_version = self.api_version_input.read(cx).value().to_string();
        let extra_headers = match parse_headers_for_provider(
            provider_type,
            self.extra_headers_input.read(cx).value().as_ref(),
        ) {
            Ok(headers) => headers,
            Err(error) => {
                self.validation_error =
                    Some(t!("LlmProviders.extra_headers_invalid", error = error).into());
                cx.notify();
                return None;
            }
        };

        let mut models = self.selected_models.clone();
        if models.is_empty() && require_models {
            self.validation_error = Some(t!("LlmProviders.model_required").to_string().into());
            cx.notify();
            return None;
        }
        if models.is_empty() {
            // 探活时需要有个占位模型，避免构造 provider 失败。
            models.push(self.default_model.clone());
            if models[0].is_empty() {
                models[0] = "gpt-4".to_string();
            }
        }

        let model = if models.iter().any(|item| item == &self.default_model) {
            self.default_model.clone()
        } else {
            models.first().cloned().unwrap_or_default()
        };

        self.validation_error = None;

        Some(ProviderConfig {
            id: self.provider_id.unwrap_or_else(unix_now),
            name,
            provider_type,
            api_key: non_empty(api_key),
            api_base: non_empty(api_base),
            api_version: non_empty(api_version),
            extra_headers,
            proxy: non_empty(self.proxy_input.read(cx).value().to_string()),
            reasoning_effort: self
                .effort_select
                .read(cx)
                .selected_value()
                .copied()
                .flatten(),
            api_mode: self
                .api_mode_select
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            model,
            models,
            max_tokens: self.max_tokens,
            temperature: self.temperature,
            enabled: self.enabled,
            is_default: self.is_default,
            created_at: self.created_at,
            updated_at: unix_now(),
        })
    }

    /// 供父视图在收到 [`ProviderDetailEvent::Submit`] 后取值。
    pub fn get_config(&mut self, cx: &mut Context<Self>) -> Option<ProviderConfig> {
        self.current_config(cx, true)
    }

    /// 拉取模型列表并填进目录（勾选全部拉到的模型）。
    fn load_models(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_fetch(window, cx, FetchMode::Load);
    }

    /// 只探活：不修改模型目录。
    fn test_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_fetch(window, cx, FetchMode::Test);
    }

    /// 拉取 / 探活取值：不能沿用「保存时必须先勾选模型」的校验。
    ///
    /// 新建提供商时模型目录必然为空，而「加载模型」正是把目录填满的前置动作；
    /// 若这里要求已勾选，就会落进「没模型 → 不许拉取 → 没模型」的死循环，
    /// 点下去只剩底部一条「至少需要选择一个模型」，看起来像按钮没反应。
    fn fetch_config(&mut self, cx: &mut Context<Self>) -> Option<ProviderConfig> {
        self.current_config(cx, false)
    }

    fn run_fetch(&mut self, _window: &mut Window, cx: &mut Context<Self>, mode: FetchMode) {
        if self.models_loading {
            return;
        }
        let Some(config) = self.fetch_config(cx) else {
            return;
        };

        self.models_loading = true;
        self.status = None;
        cx.notify();

        let provider_type = config.provider_type;
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
                Ok(Ok(models)) => Ok(filter_loaded_models(provider_type, models)),
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
                                        let count = models.len();
                                        if count == 0 {
                                            form.status = Some(Status::warning(
                                                t!("LlmProviders.model_load_empty_inline")
                                                    .to_string(),
                                            ));
                                        } else if mode == FetchMode::Load {
                                            form.available_models =
                                                merge_models(&form.available_models, &models);
                                            for model in &models {
                                                toggle_model(
                                                    &mut form.selected_models,
                                                    model,
                                                    true,
                                                );
                                            }
                                            if form.default_model.is_empty() {
                                                form.default_model = form
                                                    .selected_models
                                                    .first()
                                                    .cloned()
                                                    .unwrap_or_default();
                                            }
                                            form.status = Some(Status::success(
                                                t!(
                                                    "LlmProviders.model_load_success",
                                                    count = count
                                                )
                                                .to_string(),
                                            ));
                                        } else {
                                            form.status = Some(Status::success(
                                                t!(
                                                    "LlmProviders.test_connection_success",
                                                    count = count
                                                )
                                                .to_string(),
                                            ));
                                        }
                                        window.push_notification(
                                            form.status
                                                .as_ref()
                                                .map(|status| status.message.clone())
                                                .unwrap_or_default(),
                                            cx,
                                        );
                                    }
                                    Err(message) => {
                                        let message = if mode == FetchMode::Test {
                                            t!(
                                                "LlmProviders.test_connection_failed",
                                                error = message
                                            )
                                            .to_string()
                                        } else {
                                            t!("LlmProviders.model_load_failed", error = message)
                                                .to_string()
                                        };
                                        form.status = Some(Status::error(message.clone()));
                                        window.push_notification(message, cx);
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

    fn render_header(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let provider_type = self.selected_type(cx);
        let title: SharedString = if self.provider_id.is_some() {
            self.name_input
                .read(cx)
                .value()
                .to_string()
                .trim()
                .to_string()
                .into()
        } else {
            t!("LlmProviders.detail_new_title").to_string().into()
        };
        let test_label = if self.models_loading {
            t!("LlmProviders.testing_connection")
        } else {
            t!("LlmProviders.test_connection")
        };

        h_flex()
            .gap_3()
            .items_center()
            .child(provider_badge(provider_type, 40.))
            .child(
                v_flex()
                    .gap_1()
                    .min_w_0()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(title),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(provider_type.display_name()),
                    ),
            )
            .child(div().flex_1())
            .child(self.render_status_chip(cx))
            .child(
                Switch::new("provider-enabled")
                    .checked(self.enabled)
                    .label(if self.enabled {
                        t!("LlmProviders.action_disable").to_string()
                    } else {
                        t!("LlmProviders.action_enable").to_string()
                    })
                    .on_click(cx.listener(|this, enabled: &bool, _, cx| {
                        this.enabled = *enabled;
                        cx.notify();
                    })),
            )
            .child(
                Button::new("test-connection")
                    .with_variant(ButtonVariant::Secondary)
                    .icon(IconName::CircleCheck)
                    .label(test_label)
                    .disabled(self.models_loading)
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.test_connection(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_status_chip(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(status) = self.status.clone() else {
            return div().into_any_element();
        };
        let color = match status.kind {
            StatusKind::Success => cx.theme().success,
            StatusKind::Warning => cx.theme().warning,
            StatusKind::Error => cx.theme().danger,
        };
        div()
            .max_w(px(360.))
            .text_xs()
            .text_color(color)
            .child(status.message)
            .into_any_element()
    }

    fn field(label: SharedString, control: impl IntoElement) -> AnyElement {
        v_flex()
            .gap_1()
            .min_w_0()
            .child(div().text_sm().font_weight(FontWeight::MEDIUM).child(label))
            .child(control)
            .into_any_element()
    }

    fn two_columns(left: AnyElement, right: AnyElement) -> AnyElement {
        h_flex()
            .gap_4()
            .items_start()
            .child(div().flex_1().min_w_0().child(left))
            .child(div().flex_1().min_w_0().child(right))
            .into_any_element()
    }

    fn section(&self, title: SharedString, content: impl IntoElement, cx: &App) -> AnyElement {
        v_flex()
            .gap_3()
            .p_4()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().background)
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(content)
            .into_any_element()
    }

    fn render_basic(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let name_field = Self::field(
            t!("LlmProviders.name_label").to_string().into(),
            Input::new(&self.name_input),
        );
        let type_field = Self::field(
            t!("LlmProviders.provider_type_label").to_string().into(),
            Select::new(&self.provider_type_select),
        );
        let effort_field = Self::field(
            t!("LlmProviders.reasoning_effort_label").to_string().into(),
            v_flex()
                .gap_1()
                .child(Select::new(&self.effort_select))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("LlmProviders.reasoning_effort_help").to_string()),
                ),
        );

        let default_row = h_flex()
            .gap_3()
            .items_center()
            .child(
                Switch::new("detail-default-switch")
                    .checked(self.is_default)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.is_default = *checked;
                        cx.notify();
                    })),
            )
            .child(
                v_flex()
                    .gap_0p5()
                    .child(
                        div()
                            .text_sm()
                            .child(t!("LlmProviders.set_default").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("LlmProviders.set_default_help").to_string()),
                    ),
            );

        self.section(
            t!("LlmProviders.section_basic").to_string().into(),
            v_flex()
                .gap_3()
                .child(Self::two_columns(name_field, type_field))
                .child(effort_field)
                .child(default_row),
            cx,
        )
    }

    fn render_connection(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let key_field = Self::field(
            t!("LlmProviders.api_key_label").to_string().into(),
            Input::new(&self.api_key_input).mask_toggle(),
        );
        let base_field = Self::field(
            t!("LlmProviders.api_base_label").to_string().into(),
            v_flex()
                .gap_1()
                .child(Input::new(&self.api_base_input))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("LlmProviders.api_base_help").to_string()),
                ),
        );
        let version_field = Self::field(
            t!("LlmProviders.api_version_label").to_string().into(),
            Input::new(&self.api_version_input),
        );
        let api_mode_field = Self::field(
            t!("LlmProviders.api_mode_label").to_string().into(),
            v_flex()
                .gap_1()
                .child(Select::new(&self.api_mode_select))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("LlmProviders.api_mode_help").to_string()),
                ),
        );
        let headers_field = Self::field(
            t!("LlmProviders.extra_headers_label").to_string().into(),
            v_flex()
                .gap_1()
                .child(Textarea::new(&self.extra_headers_input))
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("LlmProviders.extra_headers_help").to_string()),
                ),
        );
        let proxy_field = Self::field(
            t!("LlmProviders.proxy_label").to_string().into(),
            v_flex().gap_1().child(Input::new(&self.proxy_input)).child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("LlmProviders.proxy_help").to_string()),
            ),
        );

        self.section(
            t!("LlmProviders.section_connection").to_string().into(),
            v_flex()
                .gap_3()
                .child(key_field)
                .child(api_mode_field)
                .child(Self::two_columns(base_field, version_field))
                .child(headers_field)
                .child(proxy_field),
            cx,
        )
    }

    fn render_models(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let theme = cx.theme().clone();
        let load_label = if self.models_loading {
            t!("LlmProviders.model_loading")
        } else {
            t!("LlmProviders.model_load")
        };

        let filtered: Vec<String> = filter_models(&self.available_models, &self.model_filter)
            .into_iter()
            .cloned()
            .collect();

        let mut rows = v_flex().gap_0p5();
        let entity = cx.entity();
        if filtered.is_empty() {
            rows = rows.child(
                div()
                    .py_6()
                    .text_sm()
                    .text_color(theme.muted_foreground)
                    .child(t!("LlmProviders.model_empty_hint").to_string()),
            );
        }
        for (index, model) in filtered.iter().enumerate() {
            let checked = self.selected_models.iter().any(|item| item == model);
            let is_default = checked && self.default_model == *model;
            let model_for_toggle = model.clone();
            let model_for_default = model.clone();

            let star_icon = if is_default {
                IconName::Star
            } else {
                IconName::StarOff
            };

            rows = rows.child(
                h_flex()
                    .id(SharedString::from(format!("model-row-{index}")))
                    .gap_2()
                    .items_center()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .hover(|style| style.bg(theme.muted))
                    .child(
                        Checkbox::new(("model-check", index))
                            .checked(checked)
                            .on_click({
                                let entity = entity.clone();
                                move |checked, _window, cx| {
                                    let model = model_for_toggle.clone();
                                    entity.update(cx, |this, cx| {
                                        this.set_model_checked(&model, *checked, cx);
                                    });
                                }
                            }),
                    )
                    .child(div().flex_1().min_w_0().text_sm().child(model.clone()))
                    .when(checked, |this| {
                        this.child(
                            Button::new(("model-default", index))
                                .with_variant(ButtonVariant::Ghost)
                                .icon(star_icon)
                                .compact()
                                .text_color(if is_default {
                                    theme.primary
                                } else {
                                    theme.muted_foreground
                                })
                                .tooltip(t!("LlmProviders.set_as_default").to_string())
                                .on_click({
                                    let entity = entity.clone();
                                    move |_, _window, cx| {
                                        let model = model_for_default.clone();
                                        entity.update(cx, |this, cx| {
                                            this.set_default_model(&model, cx);
                                        });
                                    }
                                }),
                        )
                    }),
            );
        }

        let summary = if self.model_filter.trim().is_empty() {
            t!(
                "LlmProviders.model_selected_summary",
                selected = self.selected_models.len(),
                total = self.available_models.len()
            )
            .to_string()
        } else {
            format!(
                "{} · {}",
                t!(
                    "LlmProviders.model_selected_summary",
                    selected = self.selected_models.len(),
                    total = self.available_models.len()
                ),
                t!("LlmProviders.model_filter_summary", count = filtered.len())
            )
        };

        let bulk_actions = h_flex()
            .gap_1()
            .items_center()
            .child(
                Button::new("detail-select-all-models")
                    .with_variant(ButtonVariant::Ghost)
                    .compact()
                    .label(t!("LlmProviders.model_select_all"))
                    .on_click(cx.listener(|this, _, _, cx| this.select_all_models(cx))),
            )
            .child(
                Button::new("detail-invert-models")
                    .with_variant(ButtonVariant::Ghost)
                    .compact()
                    .label(t!("LlmProviders.model_invert"))
                    .on_click(cx.listener(|this, _, _, cx| this.invert_models(cx))),
            )
            .child(
                Button::new("detail-clear-models")
                    .with_variant(ButtonVariant::Ghost)
                    .compact()
                    .label(t!("LlmProviders.model_clear"))
                    .on_click(cx.listener(|this, _, _, cx| this.clear_models(cx))),
            );

        let filter_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(summary),
            )
            .child(div().flex_1())
            .child(bulk_actions)
            .child(Input::new(&self.model_filter_input).w(px(200.)));

        let custom_row = h_flex()
            .gap_2()
            .items_center()
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(Input::new(&self.custom_model_input)),
            )
            .child(
                Button::new("detail-add-model")
                    .with_variant(ButtonVariant::Secondary)
                    .icon(IconName::Plus)
                    .label(t!("LlmProviders.model_add"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.add_custom_model(window, cx);
                    })),
            );

        let body = v_flex()
            .gap_2()
            .child(filter_row)
            .child(
                div()
                    .relative()
                    .max_h(px(240.))
                    .child(
                        // `occlude` 让外层设置页与详情页自己的滚动区在鼠标位于列表上时
                        // 都不参与滚动，避免一次滚轮同时带动多个滚动容器。
                        div()
                            .id("provider-model-list")
                            .occlude()
                            .max_h(px(240.))
                            .overflow_y_scroll()
                            .track_scroll(&self.model_scroll)
                            .child(rows),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.model_scroll).mode(ScrollbarMode::Always),
                            ),
                    ),
            )
            .child(
                h_flex().gap_2().items_center().child(
                    Button::new("detail-load-models")
                        .with_variant(ButtonVariant::Secondary)
                        .icon(IconName::Refresh)
                        .label(load_label)
                        .disabled(self.models_loading)
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.load_models(window, cx);
                        })),
                ),
            )
            .child(custom_row);

        self.section(
            t!("LlmProviders.section_models").to_string().into(),
            body,
            cx,
        )
    }

    fn render_footer(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let hint = self.validation_error.clone().or_else(|| {
            self.status.as_ref().map(|status| {
                let color_hint = status.message.clone();
                SharedString::from(color_hint)
            })
        });

        let mut footer = h_flex().gap_2().items_center();
        if let Some(hint) = hint {
            footer = footer.child(
                div()
                    .text_xs()
                    .text_color(if self.validation_error.is_some() {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(hint),
            );
        }
        footer = footer.child(div().flex_1());

        if self.provider_id.is_some() {
            footer = footer.child(
                Button::new("detail-delete-provider")
                    .with_variant(ButtonVariant::Danger)
                    .label(t!("LlmProviders.action_delete"))
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(ProviderDetailEvent::Delete);
                    })),
            );
        }

        footer
            .child(
                Button::new("detail-cancel")
                    .with_variant(ButtonVariant::Ghost)
                    .label(t!("Common.cancel"))
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(ProviderDetailEvent::Cancel);
                    })),
            )
            .child(
                Button::new("detail-save")
                    .with_variant(ButtonVariant::Primary)
                    .label(t!("Common.save"))
                    .on_click(cx.listener(|_, _, _, cx| {
                        cx.emit(ProviderDetailEvent::Submit);
                    })),
            )
            .into_any_element()
    }
}

impl Focusable for ProviderDetail {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl gpui::EventEmitter<ProviderDetailEvent> for ProviderDetail {}

impl Render for ProviderDetail {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let badge_color = provider_accent(self.selected_type(cx));
        let header = self.render_header(cx);
        let basic = self.render_basic(cx);
        let connection = self.render_connection(cx);
        let models = self.render_models(cx);
        let footer = self.render_footer(cx);

        v_flex()
            .size_full()
            .gap_4()
            .child(div().h(px(3.)).w_full().rounded(px(2.)).bg(badge_color))
            .child(header)
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        // 滚动区自己持有 hitbox + 滚轮监听，`occlude` 只挡住外层设置页的
                        // 滚动容器，不会连自己的滚动一起挡掉（不要换成 `overflow_y_scrollbar`，
                        // 它会把滚动 hitbox 移到包装层，occlude 就会把自己的滚动也禁掉）。
                        div()
                            .id("provider-detail-body")
                            .occlude()
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.body_scroll)
                            .pr_2()
                            .child(
                                v_flex()
                                    .gap_4()
                                    .child(basic)
                                    .child(connection)
                                    .child(models),
                            ),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.body_scroll).mode(ScrollbarMode::Always),
                            ),
                    ),
            )
            .child(div().h(px(1.)).w_full().bg(cx.theme().border))
            .child(footer)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FetchMode {
    Load,
    Test,
}

fn unix_now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("系统时间不应早于 UNIX 纪元")
        .as_secs() as i64
}

fn non_empty(value: String) -> Option<String> {
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ApiBaseAction, ApiModeItem, EffortItem, ProviderDetail, SelectionAction,
        api_base_action_on_type_change, api_mode_items, apply_selection_action,
        filter_loaded_models, filter_models, format_header_lines, merge_models, parse_header_lines,
        parse_headers_for_provider, reasoning_effort_items, toggle_model,
    };
    use gpui::TestAppContext;
    use gpui_component::select::SelectItem;
    use one_core::llm::types::{ApiMode, ProviderType, ReasoningEffort};

    /// 「接口协议」下拉的顺序与取值：先 chat/completions（默认），后 /responses。
    #[test]
    fn api_mode_items_offer_chat_completions_then_responses() {
        let items = api_mode_items();
        assert_eq!(2, items.len());
        assert_eq!(&ApiMode::ChatCompletions, items[0].value());
        assert_eq!(&ApiMode::Responses, items[1].value());

        let responses = ApiModeItem::new(ApiMode::Responses);
        assert_eq!(&ApiMode::Responses, responses.value());
    }

    #[test]
    fn reasoning_effort_items_cover_all_levels_in_order() {
        let values: Vec<Option<ReasoningEffort>> = reasoning_effort_items()
            .into_iter()
            .map(|item| item.effort)
            .collect();
        assert_eq!(
            values,
            [
                None,
                Some(ReasoningEffort::None),
                Some(ReasoningEffort::Low),
                Some(ReasoningEffort::Medium),
                Some(ReasoningEffort::High),
            ],
            "下拉顺序：自动 / 关闭 / 低 / 中 / 高"
        );
    }

    #[test]
    fn effort_items_expose_their_level_as_value() {
        let item = EffortItem::new(Some(ReasoningEffort::High));
        assert_eq!(item.value(), &Some(ReasoningEffort::High));
        let auto = EffortItem::new(None);
        assert_eq!(auto.value(), &None);
    }

    #[test]
    fn merge_models_keeps_order_and_drops_duplicates() {
        let existing = vec!["gpt-5".to_string(), "gpt-4.1".to_string()];
        let incoming = vec![
            " gpt-4.1 ".to_string(),
            "o4-mini".to_string(),
            "".to_string(),
            "gpt-5".to_string(),
        ];
        assert_eq!(
            merge_models(&existing, &incoming),
            vec![
                "gpt-5".to_string(),
                "gpt-4.1".to_string(),
                "o4-mini".to_string()
            ]
        );
    }

    #[test]
    fn model_filter_is_case_insensitive_substring_match() {
        let models = vec![
            "gpt-5.1".to_string(),
            "claude-sonnet-4".to_string(),
            "DeepSeek-V4".to_string(),
        ];
        let hits: Vec<&String> = filter_models(&models, "GPT-5");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0], "gpt-5.1");
        assert_eq!(filter_models(&models, "  ").len(), 3);
        assert_eq!(filter_models(&models, "deepseek").len(), 1);
        assert!(filter_models(&models, "zzz").is_empty());
    }

    #[test]
    fn toggling_models_keeps_selection_order() {
        let mut selected: Vec<String> = Vec::new();
        toggle_model(&mut selected, "b", true);
        toggle_model(&mut selected, "a", true);
        toggle_model(&mut selected, "b", true);
        assert_eq!(selected, vec!["b".to_string(), "a".to_string()]);
        toggle_model(&mut selected, "b", false);
        assert_eq!(selected, vec!["a".to_string()]);
        toggle_model(&mut selected, "a", false);
        assert!(selected.is_empty());
    }

    #[test]
    fn bulk_selection_actions_only_touch_filtered_models() {
        let models = vec![
            "gpt-5".to_string(),
            "gpt-4.1".to_string(),
            "o4-mini".to_string(),
        ];

        let mut selected: Vec<String> = Vec::new();
        apply_selection_action(&models, "gpt", &mut selected, SelectionAction::SelectAll);
        assert_eq!(selected, vec!["gpt-5".to_string(), "gpt-4.1".to_string()]);

        apply_selection_action(&models, "gpt", &mut selected, SelectionAction::Invert);
        assert!(selected.is_empty());

        apply_selection_action(&models, "", &mut selected, SelectionAction::SelectAll);
        assert_eq!(selected.len(), 3);
        apply_selection_action(&models, "gpt", &mut selected, SelectionAction::Clear);
        assert_eq!(selected, vec!["o4-mini".to_string()]);
        apply_selection_action(&models, "", &mut selected, SelectionAction::Clear);
        assert!(selected.is_empty());
    }

    #[test]
    fn custom_headers_parse_one_name_value_pair_per_line() {
        let headers =
            parse_header_lines("x-opencode-session: ses_test\nUser-Agent: opencode-navop\r\n\r\n")
                .expect("valid headers");

        assert_eq!(
            vec![
                ("x-opencode-session".to_string(), "ses_test".to_string()),
                ("User-Agent".to_string(), "opencode-navop".to_string()),
            ],
            headers
        );
    }

    #[test]
    fn custom_header_values_may_contain_colons() {
        let headers = parse_header_lines("Authorization: Bearer abc:def").expect("valid header");
        assert_eq!(
            vec![("Authorization".to_string(), "Bearer abc:def".to_string())],
            headers
        );
    }

    #[test]
    fn custom_headers_reject_malformed_lines() {
        let error = parse_header_lines("x-opencode-session ses_test").expect_err("invalid line");
        assert!(
            error.contains('1'),
            "error should identify the line: {error}"
        );
    }

    #[test]
    fn custom_headers_round_trip_through_the_editor() {
        let headers = vec![
            ("x-opencode-session".to_string(), "ses_test".to_string()),
            ("User-Agent".to_string(), "opencode-navop".to_string()),
        ];
        assert_eq!(
            headers,
            parse_header_lines(&format_header_lines(&headers)).expect("formatted headers")
        );
    }

    #[test]
    fn custom_headers_apply_to_every_provider_type() {
        assert_eq!(
            Ok(vec![("X-Tenant-ID".to_string(), "tenant-a".to_string())]),
            parse_headers_for_provider(ProviderType::Anthropic, "X-Tenant-ID: tenant-a")
        );
    }

    #[test]
    fn opencode_go_accepts_custom_request_header_overrides() {
        assert_eq!(
            Ok(vec![(
                "x-opencode-session".to_string(),
                "ses_custom".to_string()
            )]),
            parse_headers_for_provider(ProviderType::OpenCodeGo, "x-opencode-session: ses_custom")
        );
    }

    #[test]
    fn opencode_go_model_list_hides_models_unavailable_outside_its_client() {
        let models = vec![
            "deepseek-v4.1-flash".to_string(),
            "beam-free".to_string(),
            "iceman-contributor-free".to_string(),
            "glm-5.3".to_string(),
        ];

        assert_eq!(
            vec!["deepseek-v4.1-flash".to_string(), "glm-5.3".to_string()],
            filter_loaded_models(ProviderType::OpenCodeGo, models)
        );
    }

    #[test]
    fn other_provider_model_lists_are_not_filtered() {
        let models = vec!["beam-free".to_string(), "custom-model".to_string()];

        assert_eq!(
            models,
            filter_loaded_models(ProviderType::OpenAICompatible, models.clone())
        );
    }

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

    /// 新建提供商还没勾选任何模型时，「加载模型」也必须能取到请求配置。
    ///
    /// 线上表现：模型区写着「还没有模型 — 先拉取列表或手动添加」，点「加载模型」
    /// 却毫无反应，底部只剩一条红字「至少需要选择一个模型」；手动添加一个模型后
    /// 再点，几十个模型立刻全拉回来。根因是拉取误用了保存时的模型校验。
    #[gpui::test]
    fn fetch_config_does_not_require_a_selected_model(cx: &mut TestAppContext) {
        cx.update(|cx| gpui_component::init(cx));

        let (detail, cx) = cx.add_window_view(|window, cx| ProviderDetail::new(None, window, cx));

        detail.update_in(cx, |form, window, cx| {
            form.name_input.update(cx, |input, cx| {
                input.set_value("SiliconFlow", window, cx);
            });
        });

        let (save_config, fetch_config) = detail.update_in(cx, |form, _window, cx| {
            (form.current_config(cx, true), form.fetch_config(cx))
        });

        assert!(save_config.is_none(), "保存路径仍应要求先勾选模型");
        let config = fetch_config.expect("未勾选模型时也应能构造拉取用配置");
        assert!(
            !config.models.is_empty(),
            "拉取用配置仍需带一个占位模型，否则构造 provider 会失败"
        );
    }
}
