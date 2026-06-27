use agent_runtime::{ResourceContext, RuntimeEvent, RuntimeEventReceiver, SessionId, ToolRegistry};
use gpui::{AsyncApp, Context};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

use super::super::acp::config::AcpAgentConfig;
use super::super::acp::connection::AcpConnection;
use super::super::plan_tools::build_plan_tool_registry;
use super::super::runtime_bridge::{LlmModelClient, PlanRuntimeController, build_runtime};
use super::super::types::AiChatPlanBackend;
use super::AiChatPanel;
use crate::llm::manager::GlobalProviderState;

use rust_i18n::t;
impl AiChatPanel {
    pub(super) fn send_plan_message(&mut self, content: String, cx: &mut Context<Self>) {
        match self.plan_backend {
            AiChatPlanBackend::LocalRuntime => self.send_local_plan_message(content, cx),
            AiChatPlanBackend::AcpAgent => self.send_acp_plan_message(content, cx),
        }
    }

    pub(super) fn send_local_plan_message(&mut self, content: String, cx: &mut Context<Self>) {
        if content.trim().is_empty() || self.engine.is_loading {
            return;
        }

        let Some((provider_id, provider_config, selected_model)) = self.resolve_current_provider()
        else {
            cx.notify();
            return;
        };

        if let Some(session_id) = self.ensure_session_id(&provider_id.to_string(), cx) {
            self.persist_user_message(session_id, &content, cx);
        }
        if let Err(error) = self.refresh_plan_tool_registry(cx) {
            self.engine
                .push_assistant(format!("Error: failed to load Plan tools: {error}"));
            cx.notify();
            return;
        }

        self.engine.push_user_message(content.clone());
        self.engine.auto_scroll_enabled = true;
        self.engine.is_loading = true;
        let cancel_token = CancellationToken::new();
        self.engine.cancel_token = Some(cancel_token.clone());
        self.engine.scroll_to_bottom();
        cx.notify();

        let runtime_key = plan_runtime_key(
            provider_id,
            &selected_model,
            self.engine.model_settings.max_tokens,
            self.engine.model_settings.temperature,
        );
        if self.plan_runtime_key.as_deref() == Some(runtime_key.as_str())
            && self.plan_controller.is_some()
        {
            self.start_existing_plan_turn(content, cancel_token, cx);
            return;
        }

        self.start_new_plan_runtime(
            content,
            provider_config,
            selected_model,
            runtime_key,
            cancel_token,
            cx,
        );
    }

    pub(super) fn send_acp_plan_message(&mut self, content: String, cx: &mut Context<Self>) {
        if content.trim().is_empty() || self.engine.is_loading {
            return;
        }

        let Some(config) = self.acp_agent_config.clone() else {
            self.engine
                .push_assistant("Error: ACP agent config is not set");
            cx.notify();
            return;
        };

        if let Some(session_id) = self.ensure_session_id("acp", cx) {
            self.persist_user_message(session_id, &content, cx);
        }
        self.engine.push_user_message(content.clone());
        self.engine.auto_scroll_enabled = true;
        self.engine.is_loading = true;
        let cancel_token = CancellationToken::new();
        self.engine.cancel_token = Some(cancel_token.clone());
        self.engine.scroll_to_bottom();
        cx.notify();

        if let Some(connection) = self.acp_connection.as_ref() {
            let events = connection.subscribe();
            let session_id = connection.session_id();
            connection.prompt(content);
            self.spawn_plan_event_pump(events, session_id, cancel_token, cx);
            return;
        }

        self.start_new_acp_connection(config, content, cancel_token, cx);
    }

    pub(super) fn start_new_acp_connection(
        &mut self,
        config: AcpAgentConfig,
        content: String,
        cancel_token: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut AsyncApp| {
                let connection = match AcpConnection::connect(&config, cx).await {
                    Ok(connection) => connection,
                    Err(error) => {
                        update_plan_start_error(this, cx, error.to_string()).await;
                        return;
                    }
                };
                if cancel_token.is_cancelled() {
                    return;
                }
                let events = connection.subscribe();
                let session_id = connection.session_id();
                connection.prompt(content);

                if let Some(entity) = this.upgrade() {
                    let _ = cx.update(|cx| {
                        entity.update(cx, |panel, cx| {
                            panel.acp_connection = Some(connection);
                            panel.spawn_plan_event_pump(events, session_id, cancel_token, cx);
                        });
                    });
                }
            },
        )
        .detach();
    }

    pub(super) fn resolve_current_provider(
        &mut self,
    ) -> Option<(i64, crate::llm::ProviderConfig, String)> {
        let Some(provider_id_str) = self.engine.provider_id.clone() else {
            self.engine
                .push_assistant(t!("AiChat.select_provider_first").to_string());
            return None;
        };
        let provider_id = match provider_id_str.parse::<i64>() {
            Ok(id) => id,
            Err(_) => {
                self.engine
                    .push_assistant(t!("AiChat.invalid_provider_id").to_string());
                return None;
            }
        };
        let Some(config) = self
            .engine
            .provider_configs
            .iter()
            .find(|config| config.id == provider_id)
            .cloned()
        else {
            self.engine
                .push_assistant(t!("AiChat.select_provider_first").to_string());
            return None;
        };
        let model = self
            .engine
            .selected_model
            .clone()
            .filter(|model| !model.trim().is_empty())
            .unwrap_or_else(|| config.model.clone());
        Some((provider_id, config, model))
    }

    pub(super) fn refresh_plan_tool_registry(
        &mut self,
        cx: &mut Context<Self>,
    ) -> anyhow::Result<()> {
        let registry = build_plan_tool_registry(cx);
        match registry {
            Ok(registry) => {
                let changed = !same_tool_registry_names(&registry, &self.plan_tool_registry);
                self.plan_tool_registry = registry;
                if changed {
                    self.plan_controller = None;
                    self.plan_runtime_key = None;
                }
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(super) fn start_existing_plan_turn(
        &mut self,
        content: String,
        cancel_token: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        let Some(controller) = self.plan_controller.as_mut() else {
            return;
        };
        let events = controller.subscribe();
        match controller.start_turn(content) {
            Ok(_) => {
                if let Some(session_id) = controller.session_id().cloned() {
                    self.spawn_plan_event_pump(events, session_id, cancel_token, cx);
                }
            }
            Err(error) => self.fail_plan_turn(error.to_string(), cx),
        }
    }

    pub(super) fn start_new_plan_runtime(
        &mut self,
        content: String,
        provider_config: crate::llm::ProviderConfig,
        selected_model: String,
        runtime_key: String,
        cancel_token: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        let global_provider_state = cx.global::<GlobalProviderState>().clone();
        let registry = self.plan_tool_registry.clone();
        let max_tokens = self.engine.model_settings.max_tokens as u32;
        let temperature = self.engine.model_settings.temperature;

        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut AsyncApp| {
                let provider = match global_provider_state
                    .manager()
                    .get_provider(&provider_config)
                    .await
                {
                    Ok(provider) => provider,
                    Err(error) => {
                        update_plan_start_error(this, cx, error.to_string()).await;
                        return;
                    }
                };
                if cancel_token.is_cancelled() {
                    return;
                }

                let model = Arc::new(
                    LlmModelClient::new(provider, selected_model)
                        .with_max_tokens(max_tokens)
                        .with_temperature(temperature),
                );
                let runtime = build_runtime(model, registry);
                let mut controller = PlanRuntimeController::new(runtime, ResourceContext::new());
                let events = controller.subscribe();
                let start_result = controller.start_turn(content);
                let session_id = controller.session_id().cloned();

                if let Some(entity) = this.upgrade() {
                    let _ = cx.update(|cx| {
                        entity.update(cx, |panel, cx| match (start_result, session_id) {
                            (Ok(_), Some(session_id)) => {
                                panel.plan_controller = Some(controller);
                                panel.plan_runtime_key = Some(runtime_key);
                                panel.spawn_plan_event_pump(events, session_id, cancel_token, cx);
                            }
                            (Err(error), _) => panel.fail_plan_turn(error.to_string(), cx),
                            _ => panel
                                .fail_plan_turn("Plan runtime session not created".to_string(), cx),
                        });
                    });
                }
            },
        )
        .detach();
    }

    pub(super) fn spawn_plan_event_pump(
        &self,
        mut events: RuntimeEventReceiver,
        session_id: SessionId,
        cancel_token: CancellationToken,
        cx: &mut Context<Self>,
    ) {
        cx.spawn(
            async move |this: gpui::WeakEntity<Self>, cx: &mut AsyncApp| {
                while !cancel_token.is_cancelled() {
                    let event = match events.recv().await {
                        Ok(event) => event,
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    };
                    if event.session_id() != &session_id {
                        continue;
                    }
                    let terminal = is_terminal_runtime_event(&event);
                    if let Some(entity) = this.upgrade() {
                        let _ = cx.update(|cx| {
                            entity.update(cx, |panel, cx| {
                                panel.engine.apply_runtime_event(event);
                                panel.engine.scroll_to_bottom();
                                cx.notify();
                            });
                        });
                    } else {
                        break;
                    }
                    if terminal {
                        break;
                    }
                }
            },
        )
        .detach();
    }

    pub(super) fn fail_plan_turn(&mut self, message: String, cx: &mut Context<Self>) {
        self.engine.push_assistant(format!("Error: {message}"));
        self.engine.is_loading = false;
        self.engine.cancel_token = None;
        cx.notify();
    }
}
fn plan_runtime_key(provider_id: i64, model: &str, max_tokens: usize, temperature: f32) -> String {
    format!("{provider_id}:{model}:{max_tokens}:{temperature:.3}")
}

fn same_tool_registry_names(left: &ToolRegistry, right: &ToolRegistry) -> bool {
    let mut left_names = left
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    let mut right_names = right
        .names()
        .into_iter()
        .map(|name| name.to_string())
        .collect::<Vec<_>>();
    left_names.sort();
    right_names.sort();
    left_names == right_names
}

fn is_terminal_runtime_event(event: &RuntimeEvent) -> bool {
    matches!(
        event,
        RuntimeEvent::TurnCompleted { .. }
            | RuntimeEvent::TurnFailed { .. }
            | RuntimeEvent::NeedUserInput { .. }
    )
}

async fn update_plan_start_error(
    this: gpui::WeakEntity<AiChatPanel>,
    cx: &mut AsyncApp,
    message: String,
) {
    if let Some(entity) = this.upgrade() {
        let _ = cx.update(|cx| {
            entity.update(cx, |panel, cx| {
                panel.fail_plan_turn(message, cx);
            });
        });
    }
}
