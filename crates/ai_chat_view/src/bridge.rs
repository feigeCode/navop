//! 模型桥接:把 onetcli 的 [`LlmProvider`] 适配为 agent_runtime 的 [`ModelClient`]。
//!
//! `agent_runtime` 刻意只依赖 `llm-connector` 类型、通过 [`ModelClient`] trait 抽象模型,
//! 不直接耦合 `one-core`。本模块作为**集成胶水**,把已配置的 [`LlmProvider`] 包装成
//! `ModelClient`:
//! - [`LlmModelClient::complete`] 走非流式 `chat_completion`,保留 `tool_calls`(支撑 Planner
//!   的 function-calling 闭环);
//! - [`LlmModelClient::complete_stream`] 走 `chat_stream`,把增量映射为 `TextDelta` 实现逐段输出。
//!
//! [`build_runtime`] 则用一个 `ModelClient` 装配出可直接驱动的 [`Runtime`]
//! (内部使用 [`LlmPlanner`])。

use std::sync::Arc;

use self::model_policy::{
    should_disable_function_calling_for_model, should_disable_tool_choice_for_model,
    should_stream_tools_via_completion,
};
use self::stream_tools::merge_stream_tool_calls;
use agent_runtime::model::{
    ModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, ToolCall,
    model_response_into_stream,
};
use agent_runtime::{Runtime, RuntimeError, RuntimeServices, ToolRegistry, ToolRouter};
use async_trait::async_trait;
use futures::StreamExt;
use futures::stream;
use one_core::llm::{
    ChatRequest, GlobalProviderState, LlmConnector, LlmProvider, ProviderConfig,
    extract_stream_text,
};

mod model_policy;
mod stream_tools;
#[cfg(test)]
mod tests;

/// 把 [`LlmProvider`] 适配为 agent_runtime 的 [`ModelClient`]。
///
/// 持有一个具体 provider 快照与采样参数(模型名 / 温度 / 最大 token)。切换 provider
/// 或模型时,调用方应重建一个新的 `LlmModelClient`(进而重建 [`Runtime`])。
pub struct LlmModelClient {
    provider: Arc<dyn LlmProvider>,
    model: String,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
}

impl LlmModelClient {
    pub fn new(provider: Arc<dyn LlmProvider>, model: impl Into<String>) -> Self {
        Self {
            provider,
            model: model.into(),
            temperature: None,
            max_tokens: None,
        }
    }

    pub fn with_temperature(mut self, temperature: f32) -> Self {
        self.temperature = Some(temperature);
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> Self {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// 把运行时的 [`ModelRequest`] 转换为 `llm-connector` 的 [`ChatRequest`]。
    fn to_chat_request(&self, request: ModelRequest) -> ChatRequest {
        let disable_function_calling =
            should_disable_function_calling_for_model(self.provider.provider_name(), &self.model);
        let disable_tool_choice =
            should_disable_tool_choice_for_model(self.provider.provider_name(), &self.model);

        ChatRequest {
            model: self.model.clone(),
            messages: request.messages,
            tools: if disable_function_calling {
                None
            } else {
                (!request.tools.is_empty()).then_some(request.tools)
            },
            tool_choice: if disable_function_calling {
                None
            } else if disable_tool_choice {
                None
            } else {
                request.tool_choice
            },
            temperature: request.temperature.or(self.temperature),
            max_tokens: request.max_tokens.or(self.max_tokens),
            ..Default::default()
        }
    }
}

#[async_trait]
impl ModelClient for LlmModelClient {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        let chat_request = self.to_chat_request(request);
        tracing::debug!(
            model = %self.model,
            messages = chat_request.messages.len(),
            tools = chat_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            "调用模型(complete)"
        );
        let response = self
            .provider
            .chat_completion(&chat_request)
            .await
            .map_err(|e| RuntimeError::model(e.to_string()))?;
        let tool_calls = response.tool_calls().to_vec();
        tracing::debug!(
            model = %self.model,
            text_len = response.content.len(),
            tool_calls = tool_calls.len(),
            "模型返回(complete)"
        );
        Ok(ModelResponse {
            text: (!response.content.is_empty()).then(|| response.content.clone()),
            tool_calls,
        })
    }

    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        if !request.tools.is_empty()
            && should_stream_tools_via_completion(self.provider.provider_name(), &self.model)
        {
            let response = self.complete(request).await?;
            return Ok(model_response_into_stream(response));
        }

        let chat_request = self.to_chat_request(request);
        tracing::debug!(
            model = %self.model,
            messages = chat_request.messages.len(),
            tools = chat_request.tools.as_ref().map(|t| t.len()).unwrap_or(0),
            "调用模型(complete_stream)"
        );
        let inner = self
            .provider
            .chat_stream(&chat_request)
            .await
            .map_err(|e| RuntimeError::model(e.to_string()))?;

        // 状态机:逐段读取底层流,把文本映射为 TextDelta、把分片的工具调用聚合起来;
        // 流尽时补一条 Completed(完整文本 + 聚合后的工具调用)。
        let mapped = stream::unfold(
            StreamState::Streaming {
                inner,
                text: String::new(),
                tool_calls: Vec::new(),
            },
            |state| async move {
                match state {
                    StreamState::Streaming {
                        mut inner,
                        mut text,
                        mut tool_calls,
                    } => {
                        loop {
                            match inner.next().await {
                                Some(Ok(chunk)) => {
                                    merge_stream_tool_calls(&mut tool_calls, &chunk);
                                    if let Some(t) = extract_stream_text(&chunk)
                                        && !t.is_empty()
                                    {
                                        text.push_str(t);
                                        let event = ModelStreamEvent::TextDelta(t.to_string());
                                        return Some((
                                            Ok(event),
                                            StreamState::Streaming {
                                                inner,
                                                text,
                                                tool_calls,
                                            },
                                        ));
                                    }
                                    // 仅工具调用分片 / 空增量:继续读取下一段。
                                }
                                Some(Err(err)) => {
                                    let event = Err(RuntimeError::model(err.to_string()));
                                    return Some((event, StreamState::Done));
                                }
                                None => {
                                    tracing::debug!(
                                        text_len = text.len(),
                                        tool_calls = tool_calls.len(),
                                        "模型返回(complete_stream 聚合完成)"
                                    );
                                    let response = ModelResponse {
                                        text: (!text.is_empty()).then_some(text),
                                        tool_calls,
                                    };
                                    let event = Ok(ModelStreamEvent::Completed(response));
                                    return Some((event, StreamState::Done));
                                }
                            }
                        }
                    }
                    StreamState::Done => None,
                }
            },
        );

        Ok(Box::pin(mapped))
    }

    fn model_name(&self) -> &str {
        &self.model
    }
}

/// `complete_stream` 映射的内部状态。
enum StreamState {
    /// 仍在转发底层流(携带底层流、已累积文本与已聚合的工具调用)。
    Streaming {
        inner: one_core::llm::ChatStream,
        text: String,
        tool_calls: Vec<ToolCall>,
    },
    /// 已产出 Completed,流结束。
    Done,
}

/// 用一个 [`ModelClient`] 与工具注册表装配出可驱动的 [`Runtime`]。
///
/// Runtime 运行 codex 风格的 `AgentTask`:模型驱动,按需调用业务工具与
/// `update_plan` checklist,简单问答直接回答、不规划。
pub fn build_runtime(model: Arc<dyn ModelClient>, registry: ToolRegistry) -> Arc<Runtime> {
    let tools = Arc::new(ToolRouter::new(registry));
    Arc::new(Runtime::new(RuntimeServices::new(model, tools)))
}

/// 用正式模型 provider 装配 Runtime。
pub fn build_runtime_from_llm_provider(
    provider: Arc<dyn LlmProvider>,
    model: impl Into<String>,
    registry: ToolRegistry,
    temperature: Option<f32>,
    max_tokens: Option<u32>,
) -> Arc<Runtime> {
    let mut client = LlmModelClient::new(provider, model);
    if let Some(temperature) = temperature {
        client = client.with_temperature(temperature);
    }
    if let Some(max_tokens) = max_tokens {
        client = client.with_max_tokens(max_tokens);
    }
    build_runtime(Arc::new(client), registry)
}

/// 用普通 `ProviderConfig` 同步装配 Runtime。
///
/// `ProviderType::OnetCli` 需要 `GlobalProviderState` 中的云端 client,请使用
/// [`build_runtime_from_provider_state`]。
pub fn build_runtime_from_provider_config(
    config: &ProviderConfig,
    model: impl Into<String>,
    registry: ToolRegistry,
) -> anyhow::Result<Arc<Runtime>> {
    let provider: Arc<dyn LlmProvider> = Arc::new(LlmConnector::from_config(config)?);
    Ok(build_runtime_from_llm_provider(
        provider,
        model,
        registry,
        config.temperature,
        config.max_tokens.and_then(|v| u32::try_from(v).ok()),
    ))
}

/// 用 `GlobalProviderState` 异步装配 Runtime,支持 OnetCli 等需要 manager 的 provider。
pub async fn build_runtime_from_provider_state(
    provider_state: GlobalProviderState,
    config: &ProviderConfig,
    model: impl Into<String>,
    registry: ToolRegistry,
) -> anyhow::Result<Arc<Runtime>> {
    let provider = provider_state.manager().get_provider(config).await?;
    Ok(build_runtime_from_llm_provider(
        provider,
        model,
        registry,
        config.temperature,
        config.max_tokens.and_then(|v| u32::try_from(v).ok()),
    ))
}
