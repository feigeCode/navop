//! 默认 Agent 聊天面板。
//!
//! 给侧边栏提供一个同步可创建的包装层:内部异步读取 provider 配置,
//! 构建 [`AgentChatView`],并暴露 close event 与外部消息入口。

use gpui::{
    App, AppContext, AsyncApp, Context, Entity, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString, Styled, Window, div,
    prelude::FluentBuilder,
};
use gpui_component::{ActiveTheme, Icon, IconName, Sizable, Size, v_flex};
use one_core::{
    llm::{GlobalProviderState, ProviderConfig, storage::ProviderRepository},
    storage::{GlobalStorageState, traits::Repository},
    tab_container::{TabContent, TabContentEvent},
};

use crate::{
    AgentChatView, AgentChatViewConfig, CodeBlockAction, MentionItem, build_plan_tool_registry,
};

#[derive(Clone, Debug)]
pub enum DefaultAgentChatPanelEvent {
    Close,
}

pub struct DefaultAgentChatPanel {
    focus_handle: FocusHandle,
    view: Option<Entity<AgentChatView>>,
    pending_message: Option<String>,
    pending_system_instruction: Option<String>,
    pending_resource_context: Option<(agent_runtime::ResourceContext, Vec<MentionItem>)>,
    pending_code_block_actions: Vec<CodeBlockAction>,
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
        let panel = Self {
            focus_handle: cx.focus_handle(),
            view: None,
            pending_message: None,
            pending_system_instruction: None,
            pending_resource_context: None,
            pending_code_block_actions: Vec::new(),
            error: None,
        };
        Self::spawn_build_view(resources, mentions, window, cx);
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
        if let Some(view) = &self.view {
            view.update(cx, |view, cx| {
                view.set_resource_context(resources, mentions, cx);
            });
        } else {
            self.pending_resource_context = Some((resources, mentions));
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

    fn spawn_build_view(
        resources: agent_runtime::ResourceContext,
        mentions: Vec<MentionItem>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let storage = cx.global::<GlobalStorageState>().storage.clone();
        let provider_state = cx.global::<GlobalProviderState>().clone();
        let registry = build_plan_tool_registry(cx).unwrap_or_else(|error| {
            tracing::warn!(%error, "Failed to build plan tool registry");
            agent_runtime::ToolRegistry::new()
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
            .map(|config| config.sidebar_mode(true));

            let _ = cx.update_window(window_handle, |_, window, cx| {
                if let Some(panel) = this.upgrade() {
                    panel.update(cx, |panel, cx| match config {
                        Ok(config) => {
                            let view = AgentChatView::view_with_config(config, window, cx);
                            if let Some(instruction) = panel.pending_system_instruction.clone() {
                                view.update(cx, |view, cx| {
                                    view.set_system_instruction(Some(instruction), cx);
                                });
                            }
                            if let Some((resources, mentions)) =
                                panel.pending_resource_context.take()
                            {
                                view.update(cx, |view, cx| {
                                    view.set_resource_context(resources, mentions, cx);
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
        div()
            .track_focus(&self.focus_handle)
            .size_full()
            .when_some(self.view.clone(), |this, view| this.child(view))
            .when(self.view.is_none(), |this| {
                this.child(
                    v_flex()
                        .size_full()
                        .items_center()
                        .justify_center()
                        .gap_2()
                        .text_color(cx.theme().muted_foreground)
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

fn enabled_provider_configs(configs: Vec<ProviderConfig>) -> Vec<ProviderConfig> {
    configs
        .into_iter()
        .filter(|config| config.enabled)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::enabled_provider_configs;
    use one_core::llm::{ProviderConfig, ProviderType};

    #[test]
    fn enabled_provider_configs_filters_disabled_entries() {
        let enabled = ProviderConfig {
            id: 1,
            name: "enabled".to_string(),
            provider_type: ProviderType::OpenAI,
            enabled: true,
            ..ProviderConfig::default()
        };
        let disabled = ProviderConfig {
            id: 2,
            name: "disabled".to_string(),
            provider_type: ProviderType::OpenAI,
            enabled: false,
            ..ProviderConfig::default()
        };

        let configs = enabled_provider_configs(vec![enabled, disabled]);

        assert_eq!(1, configs.len());
        assert_eq!("enabled", configs[0].name);
    }
}
