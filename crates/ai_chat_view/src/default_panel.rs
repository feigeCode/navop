//! 默认 Agent 聊天面板。
//!
//! 给侧边栏提供一个同步可创建的包装层:内部异步读取 provider 配置,
//! 构建 [`AgentChatView`],并暴露 close event 与外部消息入口。

use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Subscription,
    Window, div, prelude::FluentBuilder,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, v_flex};
use one_core::{
    llm::{GlobalProviderState, ProviderConfig, storage::ProviderRepository},
    storage::{GlobalStorageState, traits::Repository},
    tab_container::{TabContent, TabContentEvent},
};

use crate::{
    AcpAgentConfig, AgentChatTheme, AgentChatView, AgentChatViewConfig, AgentChatViewEvent,
    CodeBlockAction, MentionItem, build_acp_agent_configs, build_plan_tool_registry,
};

#[derive(Clone, Debug)]
pub enum DefaultAgentChatPanelEvent {
    Close,
}

pub struct DefaultAgentChatPanel {
    focus_handle: FocusHandle,
    view: Option<Entity<AgentChatView>>,
    view_subscription: Option<Subscription>,
    pending_message: Option<String>,
    pending_system_instruction: Option<String>,
    pending_resource_context: Option<(
        agent_runtime::ResourceContext,
        Vec<MentionItem>,
        Vec<agent_runtime::ResourceRef>,
    )>,
    pending_code_block_actions: Vec<CodeBlockAction>,
    theme: Option<AgentChatTheme>,
    error: Option<String>,
}

impl DefaultAgentChatPanel {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        Self::new_with_context(
            agent_runtime::ResourceContext::new(),
            Vec::new(),
            window,
            cx,
        )
    }

    pub fn new_with_context(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let available_resources = resources.resources.clone();
        Self::new_with_context_and_catalog(resources, mentions, available_resources, window, cx)
    }

    pub fn new_with_context_and_catalog(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let panel = Self {
            focus_handle: cx.focus_handle(),
            view: None,
            view_subscription: None,
            pending_message: None,
            pending_system_instruction: None,
            pending_resource_context: None,
            pending_code_block_actions: Vec::new(),
            theme: None,
            error: None,
        };
        Self::spawn_build_view(resources, mentions, available_resources, window, cx);
        panel
    }

    pub fn send_external_message(&mut self, message: String, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.send_external_message(message, cx));
        } else {
            self.pending_message = Some(message);
            cx.notify();
        }
    }

    pub fn set_system_instruction(&mut self, instruction: Option<String>, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_system_instruction(instruction, cx));
        } else {
            self.pending_system_instruction = instruction;
            cx.notify();
        }
    }

    pub fn set_resource_context(
        &mut self,
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        cx: &mut Context<Self>,
    ) {
        let available_resources = resources.resources.clone();
        self.set_resource_context_with_catalog(resources, mentions, available_resources, cx);
    }

    pub fn set_resource_context_with_catalog(
        &mut self,
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        cx: &mut Context<Self>,
    ) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_resource_context_with_catalog(
                    resources,
                    mentions,
                    available_resources,
                    cx,
                );
            });
        } else {
            self.pending_resource_context = Some((resources, mentions, available_resources));
            cx.notify();
        }
    }

    pub fn register_code_block_action(&mut self, action: CodeBlockAction, cx: &mut Context<Self>) {
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.register_code_block_action(action, cx));
        } else {
            self.pending_code_block_actions.push(action);
            cx.notify();
        }
    }

    pub fn set_theme(&mut self, theme: Option<AgentChatTheme>, cx: &mut Context<Self>) {
        self.theme = theme.clone();
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| view.set_theme(theme, cx));
        }
        cx.notify();
    }

    fn spawn_build_view(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        available_resources: Vec<agent_runtime::ResourceRef>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let provider_state = cx.global::<GlobalProviderState>().clone();
        let registry = build_plan_tool_registry(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build plan tool registry");
            agent_runtime::ToolRegistry::new()
        });
        let acp_agents = build_acp_agent_configs(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build ACP agent configs");
            Vec::new()
        });
        let window_handle = window.window_handle();

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            let provider_configs = load_enabled_provider_configs(&storage);
            let config = AgentChatViewConfig::from_provider_state(
                resources,
                mentions,
                provider_configs,
                registry,
                provider_state,
            )
            .await
            .map(|config| {
                build_sidebar_config(
                    config.with_available_resources(available_resources),
                    acp_agents,
                )
            });

            let _ = cx.update_window(window_handle, |_, window, cx| {
                if let Some(panel) = this.upgrade() {
                    panel.update(cx, |panel, cx| match config {
                        Ok(mut config) => {
                            if let Some(theme) = panel.theme.clone() {
                                config = config.with_theme(theme);
                            }
                            let view = AgentChatView::view_with_config(config, window, cx);
                            let view_subscription =
                                cx.subscribe(&view, |_, _, event: &AgentChatViewEvent, cx| {
                                    match event {
                                        AgentChatViewEvent::Close => {
                                            cx.emit(DefaultAgentChatPanelEvent::Close);
                                        }
                                    }
                                });
                            if let Some(instruction) = panel.pending_system_instruction.clone() {
                                view.update(cx, |view, cx| {
                                    view.set_system_instruction(Some(instruction), cx);
                                });
                            }
                            if let Some((resources, mentions, available_resources)) =
                                panel.pending_resource_context.take()
                            {
                                view.update(cx, |view, cx| {
                                    view.set_resource_context_with_catalog(
                                        resources,
                                        mentions,
                                        available_resources,
                                        cx,
                                    );
                                });
                            }
                            for action in panel.pending_code_block_actions.drain(..) {
                                view.update(cx, |view, cx| {
                                    view.register_code_block_action(action, cx);
                                });
                            }
                            if let Some(message) = panel.pending_message.take() {
                                view.update(cx, |view, cx| {
                                    view.send_external_message(message, cx);
                                });
                            }
                            panel.view = Some(view);
                            panel.view_subscription = Some(view_subscription);
                            panel.error = None;
                            cx.notify();
                        }
                        Err(error) => {
                            panel.error = Some(error.to_string());
                            cx.notify();
                        }
                    });
                }
            });
        })
        .detach();
    }
}

impl EventEmitter<DefaultAgentChatPanelEvent> for DefaultAgentChatPanel {}
impl EventEmitter<TabContentEvent> for DefaultAgentChatPanel {}

impl Focusable for DefaultAgentChatPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for DefaultAgentChatPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_foreground = self
            .theme
            .as_ref()
            .map(|theme| theme.muted_foreground)
            .unwrap_or(cx.theme().muted_foreground);
        let background = self
            .theme
            .as_ref()
            .map(|theme| theme.background)
            .unwrap_or(cx.theme().background);
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .bg(background)
            .when_some(self.view.clone(), |this, view| this.child(view))
            .when(self.view.is_none(), |this| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(muted_foreground)
                        .child(
                            self.error
                                .clone()
                                .unwrap_or_else(|| "Loading AI chat...".to_string()),
                        ),
                )
            })
    }
}

impl TabContent for DefaultAgentChatPanel {
    fn content_key(&self) -> &'static str {
        "DefaultAgentChat"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from("AI Chat")
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::AI.color().with_size(Size::Medium))
    }

    fn width_size(&self, _cx: &App) -> Option<Size> {
        Some(Size::Small)
    }

    fn dump(&self, _cx: &App) -> serde_json::Value {
        serde_json::json!({
            "version": 1,
        })
    }
}

fn load_enabled_provider_configs(
    storage: &one_core::storage::StorageManager,
) -> Vec<ProviderConfig> {
    let Some(repo) = storage.get::<ProviderRepository>() else {
        return Vec::new();
    };
    match repo.list() {
        Ok(configs) => enabled_provider_configs(configs),
        Err(error) => {
            tracing::warn!(%error, "Failed to load AI provider configs");
            Vec::new()
        }
    }
}

pub(crate) fn enabled_provider_configs(configs: Vec<ProviderConfig>) -> Vec<ProviderConfig> {
    configs
        .into_iter()
        .filter(|config| config.enabled)
        .collect()
}

pub(crate) fn build_sidebar_config(
    config: AgentChatViewConfig,
    acp_agents: Vec<AcpAgentConfig>,
) -> AgentChatViewConfig {
    config.sidebar_mode(true).with_acp_agents(acp_agents)
}
