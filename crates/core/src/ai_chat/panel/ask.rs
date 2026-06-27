use gpui::{AsyncApp, Context};
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

use super::super::stream::{ChatStreamProcessor, StreamEvent};
use super::AiChatPanel;
use crate::gpui_tokio::Tokio;
use crate::llm::{
    Message, Role,
    chat_history::{ChatMessage, MessageRepository},
    manager::GlobalProviderState,
};
use crate::storage::{GlobalStorageState, traits::Repository};
use rust_i18n::t;

impl AiChatPanel {
    pub(super) fn send_ask_message(&mut self, content: String, cx: &mut Context<Self>) {
        if content.trim().is_empty() || self.engine.is_loading {
            return;
        }

        let Some(provider_id_str) = self.engine.provider_id.clone() else {
            self.engine
                .push_assistant(t!("AiChat.select_provider_first").to_string());
            cx.notify();
            return;
        };

        let provider_id: i64 = match provider_id_str.parse() {
            Ok(id) => id,
            Err(_) => {
                self.engine
                    .push_assistant(t!("AiChat.invalid_provider_id").to_string());
                cx.notify();
                return;
            }
        };

        // 确保会话存在并持久化用户消息
        if let Some(session_id) = self.ensure_session_id(&provider_id_str, cx) {
            self.persist_user_message(session_id, &content, cx);
        }

        let global_provider_state = cx.global::<GlobalProviderState>().clone();
        let global_state = cx.global::<GlobalStorageState>();
        let storage_manager = global_state.storage.clone();
        let session_id = self.engine.session_id;
        let history_count = self.engine.model_settings.history_count;
        let max_tokens = self.engine.model_settings.max_tokens;
        let temperature = self.engine.model_settings.temperature;
        let system_instruction = self.system_instruction.clone();

        // 获取用户选择的模型
        let selected_model = self.engine.selected_model.clone().unwrap_or_else(|| {
            self.engine
                .provider_configs
                .iter()
                .find(|c| c.id == provider_id)
                .map(|c| c.model.clone())
                .unwrap_or_default()
        });

        // 添加用户消息到 UI 并创建助手消息占位符
        self.engine.push_user_message(content.clone());
        let assistant_msg_id = self.engine.push_streaming_assistant();

        self.engine.auto_scroll_enabled = true;
        self.engine.is_loading = true;

        // 创建取消令牌
        let cancel_token = CancellationToken::new();
        self.engine.cancel_token = Some(cancel_token.clone());

        self.engine.scroll_to_bottom();
        cx.notify();

        // 获取 Tokio runtime handle
        let tokio_handle = Tokio::handle(cx);

        cx.spawn(async move |this, cx: &mut AsyncApp| {
            // 进入 Tokio runtime 上下文
            let _guard = tokio_handle.enter();

            if cancel_token.is_cancelled() {
                return;
            }

            // 构建聊天历史消息
            let messages: Vec<Message> = {
                let mut messages = if let Some(sid) = session_id {
                    if let Some(message_repo) = storage_manager.get::<MessageRepository>() {
                        match message_repo.list_by_session(sid) {
                            Ok(messages) => {
                                let mut msgs: Vec<Message> = messages
                                    .iter()
                                    .map(|msg| {
                                        let role = match msg.role.as_str() {
                                            "user" => Role::User,
                                            "assistant" => Role::Assistant,
                                            "system" => Role::System,
                                            _ => Role::User,
                                        };
                                        Message::text(role, &msg.content)
                                    })
                                    .collect();
                                // 限制历史条数
                                if msgs.len() > history_count {
                                    msgs = msgs.split_off(msgs.len() - history_count);
                                }
                                msgs
                            }
                            Err(_) => vec![Message::text(Role::User, &content)],
                        }
                    } else {
                        vec![Message::text(Role::User, &content)]
                    }
                } else {
                    vec![Message::text(Role::User, &content)]
                };

                if let Some(instruction) = system_instruction.as_deref() {
                    messages.insert(0, Message::text(Role::System, instruction));
                }

                messages
            };

            // 直接使用 ChatStreamProcessor 进行流式对话
            let mut rx = match ChatStreamProcessor::start(
                provider_id,
                Some(selected_model),
                messages,
                max_tokens as u32,
                temperature,
                cancel_token,
                global_provider_state,
                storage_manager.clone(),
            )
            .await
            {
                Ok(rx) => rx,
                Err(e) => {
                    if let Some(entity) = this.upgrade() {
                        let msg_id = assistant_msg_id.clone();
                        let error_msg = e.to_string();
                        let _ = cx.update(|cx| {
                            entity.update(cx, |this, cx| {
                                this.engine.set_message_error(&msg_id, error_msg);
                                this.engine.is_loading = false;
                                this.engine.cancel_token = None;
                                cx.notify();
                            });
                        });
                    }
                    return;
                }
            };

            // 处理流式事件
            while let Some(event) = rx.recv().await {
                match event {
                    StreamEvent::ContentDelta { full_content, .. } => {
                        if let Some(entity) = this.upgrade() {
                            let msg_id = assistant_msg_id.clone();
                            cx.update(|cx| {
                                entity.update(cx, |this, cx| {
                                    this.engine.update_streaming_content(&msg_id, full_content);
                                    this.engine.scroll_to_bottom();
                                    cx.notify();
                                })
                            });
                        } else {
                            return;
                        }
                    }
                    StreamEvent::ReasoningDelta { full_reasoning, .. } => {
                        if let Some(entity) = this.upgrade() {
                            let msg_id = assistant_msg_id.clone();
                            cx.update(|cx| {
                                entity.update(cx, |this, cx| {
                                    this.engine
                                        .update_streaming_reasoning(&msg_id, full_reasoning);
                                    this.engine.scroll_to_bottom();
                                    cx.notify();
                                })
                            });
                        } else {
                            return;
                        }
                    }
                    StreamEvent::Completed { full_content } => {
                        if let Some(entity) = this.upgrade() {
                            let msg_id = assistant_msg_id.clone();
                            let storage_for_save = storage_manager.clone();
                            cx.update(|cx| {
                                entity.update(cx, |this, cx| {
                                    this.engine
                                        .finalize_streaming(&msg_id, full_content.clone());
                                    this.engine.is_loading = false;
                                    this.engine.cancel_token = None;
                                    this.engine.scroll_to_bottom();

                                    // 持久化助手消息
                                    if let Some(sid) = session_id {
                                        let content_to_save = full_content;
                                        let storage = storage_for_save;
                                        cx.spawn(async move |_this, _cx: &mut AsyncApp| {
                                            if let Some(repo) = storage.get::<MessageRepository>() {
                                                let mut msg = ChatMessage::new(
                                                    sid,
                                                    "assistant".to_string(),
                                                    content_to_save,
                                                );
                                                if let Err(e) = repo.insert(&mut msg) {
                                                    warn!(
                                                        "Failed to save assistant message: {}",
                                                        e
                                                    );
                                                }
                                            }
                                        })
                                        .detach();
                                    }

                                    cx.notify();
                                })
                            });
                        }
                        break;
                    }
                    StreamEvent::Error { message } => {
                        if let Some(entity) = this.upgrade() {
                            let msg_id = assistant_msg_id.clone();
                            cx.update(|cx| {
                                entity.update(cx, |this, cx| {
                                    this.engine.set_message_error(&msg_id, message);
                                    this.engine.is_loading = false;
                                    this.engine.cancel_token = None;
                                    this.engine.scroll_to_bottom();
                                    cx.notify();
                                })
                            });
                        }
                        break;
                    }
                    StreamEvent::Cancelled => {
                        info!("Stream cancelled by user");
                        if let Some(entity) = this.upgrade() {
                            let _ = cx.update(|cx| {
                                entity.update(cx, |this, cx| {
                                    this.engine.is_loading = false;
                                    this.engine.cancel_token = None;
                                    cx.notify();
                                });
                            });
                        }
                        break;
                    }
                }
            }
        })
        .detach();
    }
}
