//! LLM 供应商设置页：左侧列表 + 右侧详情（master-detail），空态是热门供应商画廊。

use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, AsyncApp, Context, Entity, FocusHandle, Focusable, FontWeight,
    InteractiveElement, IntoElement, ParentElement, Render, ScrollHandle, SharedString,
    StatefulInteractiveElement, Styled, Subscription, WeakEntity, Window, div, px,
};
use gpui_component::{
    ActiveTheme, WindowExt,
    button::{Button, ButtonVariant, ButtonVariants},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState},
    scroll::{Scrollbar, ScrollbarMode},
    v_flex,
};
use one_assets::IconName;
use one_core::llm::{
    GlobalProviderState,
    notifier::emit_provider_config_changed,
    storage::ProviderRepository,
    types::{ProviderConfig, ProviderType},
};
use one_core::storage::{GlobalStorageState, StorageManager, traits::Repository};
use rust_i18n::t;

use super::provider_badge::{provider_badge, provider_status_dot};
use super::provider_detail::{ProviderDetail, ProviderDetailEvent};

/// 空态画廊的分组：热门海外 / 国内 / 本地与自托管 / 自定义。
struct PresetGroup {
    title: &'static str,
    types: &'static [ProviderType],
}

const PRESET_GROUPS: &[PresetGroup] = &[
    PresetGroup {
        title: "LlmProviders.gallery_group_global",
        types: &[
            ProviderType::OpenAI,
            ProviderType::Anthropic,
            ProviderType::Google,
            ProviderType::OpenRouter,
            ProviderType::AzureOpenAI,
        ],
    },
    PresetGroup {
        title: "LlmProviders.gallery_group_cn",
        types: &[
            ProviderType::DeepSeek,
            ProviderType::Zhipu,
            ProviderType::Aliyun,
            ProviderType::Volcengine,
            ProviderType::Moonshot,
            ProviderType::SiliconFlow,
            ProviderType::TencentHunyuan,
            ProviderType::Xiaomi,
            ProviderType::MiniMax,
        ],
    },
    PresetGroup {
        title: "LlmProviders.gallery_group_local",
        types: &[
            ProviderType::Ollama,
            ProviderType::Xinference,
            ProviderType::OpenCodeGo,
        ],
    },
    PresetGroup {
        title: "LlmProviders.gallery_group_custom",
        types: &[ProviderType::OpenAICompatible],
    },
];

/// 列表搜索：名称、供应商类型、任一模型命中即可（大小写不敏感）。
pub fn provider_matches_query(provider: &ProviderConfig, query: &str) -> bool {
    let query = query.trim().to_ascii_lowercase();
    if query.is_empty() {
        return true;
    }
    provider.name.to_ascii_lowercase().contains(&query)
        || provider
            .provider_type
            .display_name()
            .to_ascii_lowercase()
            .contains(&query)
        || provider
            .models
            .iter()
            .any(|model| model.to_ascii_lowercase().contains(&query))
}

pub struct LlmProvidersView {
    focus_handle: FocusHandle,
    storage_manager: StorageManager,
    providers: Vec<ProviderConfig>,
    loading: bool,
    search_input: Entity<InputState>,
    search_query: String,
    /// 当前选中的供应商；`None` 表示在编辑一个还没保存的新供应商。
    selected_id: Option<i64>,
    detail: Option<Entity<ProviderDetail>>,
    detail_subscription: Option<Subscription>,
    /// 左栏列表的滚动句柄（给列表挂可见滚动条）。
    list_scroll: ScrollHandle,
}

impl LlmProvidersView {
    fn notify_provider_configs_changed(cx: &mut App) {
        if let Some(state) = cx.try_global::<GlobalProviderState>() {
            state.manager().clear_cache();
        }
        emit_provider_config_changed(cx);
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let focus_handle = cx.focus_handle();
        let storage_state = cx.global::<GlobalStorageState>();
        let storage_manager = storage_state.storage.clone();

        let search_input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(t!("LlmProviders.search_placeholder"))
                .clean_on_escape()
        });
        let search_subscription = cx.subscribe(
            &search_input,
            |view: &mut Self, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    view.search_query = view.search_input.read(cx).value().to_string();
                    cx.notify();
                }
            },
        );

        let instance = Self {
            focus_handle,
            storage_manager,
            providers: vec![],
            loading: false,
            search_input,
            search_query: String::new(),
            selected_id: None,
            detail: None,
            detail_subscription: Some(search_subscription),
            list_scroll: ScrollHandle::new(),
        };
        cx.spawn(async move |entity: WeakEntity<Self>, cx: &mut AsyncApp| {
            let _ = entity.update(cx, |this, cx| {
                this.load_providers(cx);
            });
        })
        .detach();

        instance
    }

    fn load_providers(&mut self, cx: &mut Context<Self>) {
        self.loading = true;
        self.reload_providers(cx);
    }

    fn reload_providers(&mut self, cx: &mut Context<Self>) {
        let repo = self
            .storage_manager
            .get::<ProviderRepository>()
            .expect("ProviderRepository not found");
        match repo.list() {
            Ok(providers) => {
                self.providers = providers;
            }
            Err(e) => {
                tracing::error!("Failed to load providers: {}", e);
            }
        }
        self.loading = false;

        if let Some(selected_id) = self.selected_id {
            if !self
                .providers
                .iter()
                .any(|provider| provider.id == selected_id)
            {
                self.selected_id = None;
                self.detail = None;
            }
        }
        cx.notify();
    }

    fn filtered_providers(&self) -> Vec<&ProviderConfig> {
        self.providers
            .iter()
            .filter(|provider| provider_matches_query(provider, &self.search_query))
            .collect()
    }

    fn default_or_first(&self) -> Option<&ProviderConfig> {
        self.providers
            .iter()
            .find(|provider| provider.is_default)
            .or_else(|| self.providers.first())
    }

    fn select_provider(&mut self, provider_id: i64, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider) = self
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .cloned()
        else {
            return;
        };
        self.selected_id = Some(provider_id);
        self.open_detail(Some(provider), window, cx);
    }

    fn add_provider(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.selected_id = None;
        self.open_detail(None, window, cx);
    }

    fn add_preset(
        &mut self,
        provider_type: ProviderType,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = cx.new(|cx| ProviderDetail::for_preset(provider_type, window, cx));
        self.attach_detail(detail, window, cx);
    }

    fn open_detail(
        &mut self,
        provider: Option<ProviderConfig>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let detail = cx.new(|cx| ProviderDetail::new(provider, window, cx));
        self.attach_detail(detail, window, cx);
    }

    fn attach_detail(
        &mut self,
        detail: Entity<ProviderDetail>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let subscription = cx.subscribe_in(
            &detail,
            window,
            |view: &mut Self, _, event: &ProviderDetailEvent, window, cx| match event {
                ProviderDetailEvent::Submit => view.save_detail(window, cx),
                ProviderDetailEvent::Cancel => view.cancel_edit(window, cx),
                ProviderDetailEvent::Delete => view.request_delete(window, cx),
            },
        );
        self.detail = Some(detail);
        self.detail_subscription = Some(subscription);
        cx.notify();
    }

    fn save_detail(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(detail) = self.detail.clone() else {
            return;
        };
        let Some(mut config) = detail.update(cx, |detail, cx| detail.get_config(cx)) else {
            return;
        };

        let is_update = self
            .providers
            .iter()
            .any(|provider| provider.id == config.id);
        let repo = self
            .storage_manager
            .get::<ProviderRepository>()
            .expect("ProviderRepository not found");

        if config.is_default {
            if let Ok(existing) = repo.list() {
                for mut item in existing {
                    if item.id != config.id && item.is_default {
                        item.is_default = false;
                        if let Err(e) = repo.update(&item) {
                            tracing::error!("Failed to unset default provider: {}", e);
                        }
                    }
                }
            }
        }

        let result = if is_update {
            repo.update(&config)
        } else {
            repo.insert(&mut config).map(|_| ())
        };

        match result {
            Ok(()) => {
                Self::notify_provider_configs_changed(cx);
                self.reload_providers(cx);
                self.selected_id = Some(config.id);
                self.open_detail(Some(config), window, cx);
            }
            Err(e) => {
                tracing::error!("Failed to save provider: {}", e);
            }
        }
    }

    fn cancel_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        match self.selected_id {
            Some(provider_id) => self.select_provider(provider_id, window, cx),
            None => {
                self.detail = None;
                self.detail_subscription = None;
                // 还有别的供应商时回到默认那家，否则停在空态画廊。
                if let Some(provider) = self.default_or_first() {
                    let provider = provider.clone();
                    self.selected_id = Some(provider.id);
                    self.open_detail(Some(provider), window, cx);
                } else {
                    cx.notify();
                }
            }
        }
    }

    fn request_delete(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(provider_id) = self.selected_id else {
            return;
        };
        let name = self
            .providers
            .iter()
            .find(|provider| provider.id == provider_id)
            .map(|provider| provider.name.clone())
            .unwrap_or_default();
        self.confirm_delete_provider(provider_id, name, window, cx);
    }

    fn delete_provider(&mut self, provider_id: i64, cx: &mut Context<Self>) {
        let repo = self
            .storage_manager
            .get::<ProviderRepository>()
            .expect("ProviderRepository not found");

        match repo.delete(provider_id) {
            Ok(_) => {
                Self::notify_provider_configs_changed(cx);
                self.selected_id = None;
                self.detail = None;
                self.detail_subscription = None;
                self.load_providers(cx);
            }
            Err(e) => tracing::error!("Failed to delete provider: {}", e),
        }
    }

    fn confirm_delete_provider(
        &mut self,
        provider_id: i64,
        provider_name: String,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let view = cx.entity().clone();

        window.open_dialog(cx, move |dialog, _, _| {
            let view_clone = view.clone();
            dialog
                .title(t!("LlmProviders.delete_confirm_title").to_string())
                .child(
                    t!(
                        "LlmProviders.delete_confirm_message",
                        provider_name = provider_name
                    )
                    .to_string(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.delete"))
                        .ok_variant(ButtonVariant::Danger)
                        .cancel_text(t!("Common.cancel")),
                )
                .on_ok(move |_, _, cx: &mut App| {
                    _ = view_clone.update(cx, |view, cx| {
                        view.delete_provider(provider_id, cx);
                    });
                    true
                })
        });
    }

    fn render_provider_row(
        &self,
        provider: &ProviderConfig,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let provider_id = provider.id;
        let is_default = provider.is_default;
        let provider_type = provider.provider_type;
        let enabled = provider.enabled;
        let subtitle = if provider.model.trim().is_empty() {
            t!("LlmProviders.default_model_auto").to_string()
        } else {
            provider.model.clone()
        };
        let theme = cx.theme().clone();

        h_flex()
            .id(SharedString::from(format!("provider-row-{provider_id}")))
            .gap_2()
            .items_center()
            .px_2()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(selected, |this| this.bg(theme.secondary))
            .hover(|style| style.bg(theme.muted))
            .on_click(cx.listener(move |view, _, window, cx| {
                view.select_provider(provider_id, window, cx);
            }))
            .child(provider_badge(provider_type, 28.))
            .child(
                v_flex()
                    .flex_1()
                    .min_w_0()
                    .gap_0p5()
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_sm()
                                    .font_weight(FontWeight::MEDIUM)
                                    .child(provider.name.clone()),
                            )
                            .when(is_default, |this| {
                                this.child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.primary)
                                        .child(t!("LlmProviders.default_badge").to_string()),
                                )
                            }),
                    )
                    .child(
                        h_flex()
                            .gap_1()
                            .items_center()
                            .child(provider_status_dot(provider_type, enabled, cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .truncate()
                                    .text_xs()
                                    .text_color(theme.muted_foreground)
                                    .child(subtitle),
                            ),
                    ),
            )
            .into_any_element()
    }

    fn render_list(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let providers: Vec<ProviderConfig> =
            self.filtered_providers().into_iter().cloned().collect();
        let selected_id = self.selected_id;

        let mut list = v_flex().gap_0p5();
        for provider in &providers {
            list = list.child(self.render_provider_row(
                provider,
                selected_id == Some(provider.id),
                cx,
            ));
        }
        if providers.is_empty() && !self.providers.is_empty() {
            list = list.child(
                div()
                    .p_3()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("LlmProviders.search_no_result").to_string()),
            );
        }

        v_flex()
            .w(px(248.))
            .h_full()
            .flex_shrink_0()
            .gap_2()
            .child(Input::new(&self.search_input).prefix(IconName::Search))
            .child(
                div()
                    .relative()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("provider-list")
                            .size_full()
                            .overflow_y_scroll()
                            .track_scroll(&self.list_scroll)
                            .child(list),
                    )
                    .child(
                        div()
                            .absolute()
                            .top_0()
                            .right_0()
                            .bottom_0()
                            .w(px(12.))
                            .child(
                                Scrollbar::vertical(&self.list_scroll).mode(ScrollbarMode::Always),
                            ),
                    ),
            )
            .child(
                Button::new("add-provider")
                    .with_variant(ButtonVariant::Secondary)
                    .icon(IconName::Plus)
                    .label(t!("LlmProviders.add_provider"))
                    .on_click(cx.listener(|view, _, window, cx| {
                        view.add_provider(window, cx);
                    })),
            )
            .into_any_element()
    }

    fn render_gallery(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let mut groups = v_flex().gap_5();
        for group in PRESET_GROUPS {
            let mut cards = h_flex().gap_2().flex_wrap();
            for provider_type in group.types {
                let provider_type = *provider_type;
                cards = cards.child(
                    h_flex()
                        .id(SharedString::from(format!(
                            "gallery-{}",
                            provider_type.as_str()
                        )))
                        .gap_2()
                        .items_center()
                        .w(px(190.))
                        .p_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .cursor_pointer()
                        .hover(|style| style.bg(cx.theme().muted))
                        .on_click(cx.listener(move |view, _, window, cx| {
                            view.add_preset(provider_type, window, cx);
                        }))
                        .child(provider_badge(provider_type, 30.))
                        .child(
                            div()
                                .min_w_0()
                                .truncate()
                                .text_sm()
                                .child(provider_type.display_name()),
                        ),
                );
            }
            groups = groups.child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(cx.theme().muted_foreground)
                            .child(t!(group.title).to_string()),
                    )
                    .child(cards),
            );
        }

        v_flex()
            .flex_1()
            .h_full()
            .gap_4()
            .child(
                v_flex()
                    .gap_1()
                    .child(
                        div()
                            .text_lg()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(t!("LlmProviders.gallery_title").to_string()),
                    )
                    .child(
                        div()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!("LlmProviders.gallery_description").to_string()),
                    ),
            )
            .child(groups)
            .into_any_element()
    }

    fn render_detail(&mut self, cx: &mut Context<Self>) -> AnyElement {
        match self.detail.clone() {
            Some(detail) => div()
                .flex_1()
                .min_w_0()
                .h_full()
                .child(detail)
                .into_any_element(),
            None => v_flex()
                .flex_1()
                .h_full()
                .items_center()
                .justify_center()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("LlmProviders.select_provider_hint").to_string()),
                )
                .child(
                    Button::new("add-provider-empty")
                        .with_variant(ButtonVariant::Primary)
                        .icon(IconName::Plus)
                        .label(t!("LlmProviders.add_provider"))
                        .on_click(cx.listener(|view, _, window, cx| {
                            view.add_provider(window, cx);
                        })),
                )
                .into_any_element(),
        }
    }
}

impl Focusable for LlmProvidersView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for LlmProvidersView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.loading && self.providers.is_empty() {
            return v_flex().size_full().items_center().justify_center().child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("LlmProviders.loading").to_string()),
            );
        }

        if self.providers.is_empty() && self.detail.is_none() {
            return v_flex()
                .size_full()
                .h(px(560.))
                .child(self.render_gallery(cx));
        }

        let list = self.render_list(cx);
        let detail = self.render_detail(cx);

        h_flex()
            .size_full()
            .h(px(560.))
            .gap_4()
            .items_start()
            .child(list)
            .child(detail)
    }
}

#[cfg(test)]
mod tests {
    use super::{PRESET_GROUPS, provider_matches_query};
    use one_core::llm::types::{ProviderConfig, ProviderType};

    #[test]
    fn gallery_covers_every_provider_type_exactly_once() {
        let mut listed: Vec<ProviderType> = PRESET_GROUPS
            .iter()
            .flat_map(|group| group.types.iter().copied())
            .collect();
        listed.sort_by_key(|provider_type| provider_type.as_str());
        listed.dedup();

        let mut all = ProviderType::all();
        all.sort_by_key(|provider_type| provider_type.as_str());

        assert_eq!(listed, all, "画廊应覆盖全部供应商类型且不重复");
    }

    fn sample(name: &str, provider_type: ProviderType, models: &[&str]) -> ProviderConfig {
        ProviderConfig {
            name: name.to_string(),
            provider_type,
            models: models.iter().map(|model| model.to_string()).collect(),
            ..ProviderConfig::default()
        }
    }

    #[test]
    fn list_search_matches_name_type_and_models() {
        let provider = sample("Work OpenAI", ProviderType::OpenRouter, &["gpt-5.5", "o4"]);

        assert!(provider_matches_query(&provider, ""));
        assert!(provider_matches_query(&provider, "openai"));
        assert!(provider_matches_query(&provider, "OPENROUTER"));
        assert!(provider_matches_query(&provider, "5.5"));
        assert!(!provider_matches_query(&provider, "claude"));
    }
}
