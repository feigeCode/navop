use std::sync::Arc;

use agent_runtime::{
    ModelClient, ModelRequest, ModelResponse, ModelStream, ModelStreamEvent, ResourceContext,
    Runtime, RuntimeError, RuntimeEventReceiver, SessionId, TaskKind, ToolRegistry, ToolRouter,
    TurnId, UserInput,
};
use async_trait::async_trait;
use futures::{StreamExt, stream};
use llm_connector::types::{ChatRequest, StreamingResponse, ToolCall};

use crate::llm::{ChatStream, LlmProvider, extract_stream_text_parts};

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

    fn to_chat_request(&self, request: ModelRequest) -> ChatRequest {
        ChatRequest {
            model: self.model.clone(),
            messages: request.messages,
            tools: (!request.tools.is_empty()).then_some(request.tools),
            tool_choice: request.tool_choice,
            temperature: request.temperature.or(self.temperature),
            max_tokens: request.max_tokens.or(self.max_tokens),
            ..Default::default()
        }
    }
}

#[async_trait]
impl ModelClient for LlmModelClient {
    async fn complete(&self, request: ModelRequest) -> Result<ModelResponse, RuntimeError> {
        let response = self
            .provider
            .chat_completion(&self.to_chat_request(request))
            .await
            .map_err(|error| RuntimeError::model(error.to_string()))?;

        Ok(ModelResponse {
            text: (!response.content.is_empty()).then_some(response.content.clone()),
            tool_calls: response.tool_calls().to_vec(),
        })
    }

    async fn complete_stream(&self, request: ModelRequest) -> Result<ModelStream, RuntimeError> {
        let chat_request = self.to_chat_request(request);
        let inner = self
            .provider
            .chat_stream(&chat_request)
            .await
            .map_err(|error| RuntimeError::model(error.to_string()))?;

        let mapped = stream::unfold(
            StreamState::Streaming {
                inner,
                text: String::new(),
                reasoning: String::new(),
                tool_calls: Vec::new(),
            },
            |state| async move {
                match state {
                    StreamState::Streaming {
                        mut inner,
                        mut text,
                        mut reasoning,
                        mut tool_calls,
                    } => loop {
                        match inner.next().await {
                            Some(Ok(chunk)) => {
                                merge_stream_tool_calls(&mut tool_calls, &chunk);
                                let parts = extract_stream_text_parts(&chunk);
                                if let Some(delta) = parts.reasoning
                                    && !delta.is_empty()
                                {
                                    reasoning.push_str(delta);
                                    return Some((
                                        Ok(ModelStreamEvent::ReasoningDelta(delta.to_string())),
                                        StreamState::Streaming {
                                            inner,
                                            text,
                                            reasoning,
                                            tool_calls,
                                        },
                                    ));
                                }
                                if let Some(delta) = parts.content
                                    && !delta.is_empty()
                                {
                                    text.push_str(delta);
                                    return Some((
                                        Ok(ModelStreamEvent::TextDelta(delta.to_string())),
                                        StreamState::Streaming {
                                            inner,
                                            text,
                                            reasoning,
                                            tool_calls,
                                        },
                                    ));
                                }
                            }
                            Some(Err(error)) => {
                                return Some((
                                    Err(RuntimeError::model(error.to_string())),
                                    StreamState::Done,
                                ));
                            }
                            None => {
                                let response = ModelResponse {
                                    text: (!text.is_empty()).then_some(text),
                                    tool_calls,
                                };
                                return Some((
                                    Ok(ModelStreamEvent::Completed(response)),
                                    StreamState::Done,
                                ));
                            }
                        }
                    },
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

pub fn build_runtime(
    model: Arc<dyn ModelClient>,
    registry: ToolRegistry,
) -> Arc<agent_runtime::Runtime> {
    let tools = Arc::new(ToolRouter::new(registry));
    Arc::new(agent_runtime::Runtime::new(
        agent_runtime::RuntimeServices::new(model, tools),
    ))
}

pub struct PlanRuntimeController {
    runtime: Arc<Runtime>,
    resources: ResourceContext,
    session_id: Option<SessionId>,
}

impl PlanRuntimeController {
    pub fn new(runtime: Arc<Runtime>, resources: ResourceContext) -> Self {
        Self {
            runtime,
            resources,
            session_id: None,
        }
    }

    pub fn session_id(&self) -> Option<&SessionId> {
        self.session_id.as_ref()
    }

    pub fn subscribe(&self) -> RuntimeEventReceiver {
        self.runtime.subscribe()
    }

    pub fn start_turn(&mut self, input: impl Into<UserInput>) -> Result<TurnId, RuntimeError> {
        let session_id = self.ensure_session_id().clone();
        self.runtime
            .start_turn(&session_id, input.into(), TaskKind::Agent)
    }

    pub fn interrupt(&self) -> Result<(), RuntimeError> {
        match self.session_id.as_ref() {
            Some(session_id) => self.runtime.interrupt(session_id),
            None => Ok(()),
        }
    }

    fn ensure_session_id(&mut self) -> &SessionId {
        self.session_id.get_or_insert_with(|| {
            self.runtime
                .create_session(self.resources.clone())
                .id()
                .clone()
        })
    }
}

enum StreamState {
    Streaming {
        inner: ChatStream,
        text: String,
        reasoning: String,
        tool_calls: Vec<ToolCall>,
    },
    Done,
}

fn merge_stream_tool_calls(acc: &mut Vec<ToolCall>, chunk: &StreamingResponse) {
    let Some(snapshot) = chunk
        .choices
        .first()
        .and_then(|choice| choice.delta.tool_calls.as_ref())
    else {
        return;
    };

    for call in snapshot {
        match acc
            .iter_mut()
            .find(|existing| same_tool_call(existing, call))
        {
            Some(existing) => *existing = call.clone(),
            None => acc.push(call.clone()),
        }
    }
}

fn same_tool_call(left: &ToolCall, right: &ToolCall) -> bool {
    match (left.index, right.index) {
        (Some(left), Some(right)) => left == right,
        _ => !left.id.is_empty() && left.id == right.id,
    }
}

#[cfg(test)]
mod tests;
