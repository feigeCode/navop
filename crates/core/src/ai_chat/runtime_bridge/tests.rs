use std::sync::{Arc, Mutex};

use agent_runtime::model::MockModelClient;
use agent_runtime::{
    ModelClient, ModelRequest, ModelResponse, ModelStreamEvent, ResourceContext, RuntimeEvent,
    ToolRegistry,
};
use anyhow::Result;
use async_trait::async_trait;
use futures::StreamExt;
use llm_connector::types::{
    ChatRequest, ChatResponse, Choice, Delta, FunctionCall, Message, StreamingChoice,
    StreamingResponse, Tool, ToolCall, ToolChoice,
};

use super::{LlmModelClient, PlanRuntimeController, build_runtime};
use crate::llm::{ChatStream, LlmProvider};

struct RecordingProvider {
    response: ChatResponse,
    stream_chunks: Vec<StreamingResponse>,
    requests: Mutex<Vec<ChatRequest>>,
}

impl RecordingProvider {
    fn new(response: ChatResponse) -> Self {
        Self {
            response,
            stream_chunks: Vec::new(),
            requests: Mutex::new(Vec::new()),
        }
    }

    fn with_stream_chunks(mut self, stream_chunks: Vec<StreamingResponse>) -> Self {
        self.stream_chunks = stream_chunks;
        self
    }

    fn requests(&self) -> Vec<ChatRequest> {
        self.requests
            .lock()
            .expect("request lock should not be poisoned")
            .clone()
    }
}

#[async_trait]
impl LlmProvider for RecordingProvider {
    async fn chat(&self, _request: &ChatRequest) -> Result<String> {
        Ok(self.response.content.clone())
    }

    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse> {
        self.requests
            .lock()
            .expect("request lock should not be poisoned")
            .push(request.clone());
        Ok(self.response.clone())
    }

    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream> {
        self.requests
            .lock()
            .expect("request lock should not be poisoned")
            .push(request.clone());
        let chunks = self.stream_chunks.clone().into_iter().map(Ok);
        Ok(Box::pin(futures::stream::iter(chunks)))
    }

    async fn models(&self) -> Result<Vec<String>> {
        Ok(vec!["agent-model".to_string()])
    }

    fn provider_name(&self) -> &str {
        "recording"
    }
}

#[tokio::test]
async fn llm_model_client_complete_preserves_request_options_and_tool_calls() {
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "update_plan".to_string(),
            arguments: r#"{"steps":[]}"#.to_string(),
            ..Default::default()
        },
        ..Default::default()
    };
    let provider = Arc::new(RecordingProvider::new(ChatResponse {
        content: "planning".to_string(),
        choices: vec![Choice {
            message: Message::assistant("planning").with_tool_calls(vec![tool_call.clone()]),
            finish_reason: Some("tool_calls".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    }));
    let client = LlmModelClient::new(provider.clone(), "agent-model")
        .with_temperature(0.2)
        .with_max_tokens(512);
    let tool = Tool::function(
        "update_plan",
        Some("Update plan".to_string()),
        json_schema(),
    );

    let response = client
        .complete(
            ModelRequest::new(vec![Message::user("make a plan")])
                .with_tools(vec![tool])
                .with_tool_choice(ToolChoice::auto()),
        )
        .await
        .expect("model request should complete");

    assert_eq!(Some("planning"), response.text.as_deref());
    assert_eq!(1, response.tool_calls.len());
    assert_eq!(tool_call.id, response.tool_calls[0].id);
    assert_eq!(
        tool_call.function.name,
        response.tool_calls[0].function.name
    );
    assert_eq!(
        tool_call.function.arguments,
        response.tool_calls[0].function.arguments
    );
    let requests = provider.requests();
    assert_eq!(1, requests.len());
    assert_eq!("agent-model", requests[0].model);
    assert_eq!(Some(0.2), requests[0].temperature);
    assert_eq!(Some(512), requests[0].max_tokens);
    assert_eq!(
        1,
        requests[0]
            .tools
            .as_ref()
            .expect("tools should be set")
            .len()
    );
    assert!(matches!(
        requests[0].tool_choice.as_ref(),
        Some(ToolChoice::Mode(mode)) if mode == "auto"
    ));
}

#[tokio::test]
async fn llm_model_client_complete_stream_forwards_text_and_aggregates_tool_calls() {
    let tool_call = ToolCall {
        id: "call_1".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "update_plan".to_string(),
            arguments: r#"{"steps":[{"title":"Inspect"}]}"#.to_string(),
            ..Default::default()
        },
        index: Some(0),
        ..Default::default()
    };
    let provider = Arc::new(
        RecordingProvider::new(ChatResponse::default()).with_stream_chunks(vec![
            stream_text_chunk("he"),
            stream_text_chunk("llo"),
            stream_tool_chunk(tool_call.clone()),
        ]),
    );
    let client = LlmModelClient::new(provider.clone(), "agent-model");

    let mut stream = client
        .complete_stream(ModelRequest::new(vec![Message::user("make a plan")]))
        .await
        .expect("model stream should start");

    let first = stream.next().await.expect("first event").expect("ok event");
    let second = stream
        .next()
        .await
        .expect("second event")
        .expect("ok event");
    let completed = stream
        .next()
        .await
        .expect("completed event")
        .expect("ok event");
    assert!(stream.next().await.is_none());

    assert!(matches!(first, ModelStreamEvent::TextDelta(delta) if delta == "he"));
    assert!(matches!(second, ModelStreamEvent::TextDelta(delta) if delta == "llo"));
    match completed {
        ModelStreamEvent::Completed(response) => {
            assert_eq!(Some("hello"), response.text.as_deref());
            assert_eq!(1, response.tool_calls.len());
            assert_eq!("call_1", response.tool_calls[0].id);
            assert_eq!("update_plan", response.tool_calls[0].function.name);
            assert_eq!(
                r#"{"steps":[{"title":"Inspect"}]}"#,
                response.tool_calls[0].function.arguments
            );
        }
        other => panic!("expected completed event, got {other:?}"),
    }
}

#[tokio::test]
async fn plan_runtime_controller_reuses_session_and_starts_turns() {
    let model = Arc::new(MockModelClient::new(vec![
        ModelResponse::text("first"),
        ModelResponse::text("second"),
    ]));
    let runtime = build_runtime(model.clone(), ToolRegistry::default());
    let mut controller = PlanRuntimeController::new(runtime.clone(), ResourceContext::new());
    let mut events = controller.subscribe();

    let first = controller.start_turn("inspect database").unwrap();
    let session_id = controller.session_id().cloned().unwrap();
    wait_for_turn_completed(&mut events, &session_id).await;

    let second = controller.start_turn("continue").unwrap();
    wait_for_turn_completed(&mut events, &session_id).await;

    assert_ne!(first, second);
    assert_eq!(Some(&session_id), controller.session_id());
    assert_eq!(2, model.request_count());
    assert_eq!(
        4,
        runtime
            .session(&session_id)
            .unwrap()
            .history_snapshot()
            .len()
    );
}

async fn wait_for_turn_completed(
    events: &mut agent_runtime::RuntimeEventReceiver,
    session_id: &agent_runtime::SessionId,
) {
    tokio::time::timeout(std::time::Duration::from_secs(1), async {
        loop {
            let event = events.recv().await.expect("runtime event should arrive");
            if event.session_id() == session_id
                && matches!(event, RuntimeEvent::TurnCompleted { .. })
            {
                break;
            }
        }
    })
    .await
    .expect("turn should complete");
}

fn json_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {}
    })
}

fn stream_text_chunk(content: &str) -> StreamingResponse {
    StreamingResponse {
        id: String::new(),
        object: "chat.completion.chunk".to_string(),
        created: 0,
        model: "agent-model".to_string(),
        content: content.to_string(),
        choices: vec![StreamingChoice {
            index: 0,
            delta: Delta {
                content: Some(content.to_string()),
                ..Default::default()
            },
            finish_reason: None,
            logprobs: None,
        }],
        reasoning_content: None,
        usage: None,
        system_fingerprint: None,
    }
}

fn stream_tool_chunk(tool_call: ToolCall) -> StreamingResponse {
    StreamingResponse {
        id: String::new(),
        object: "chat.completion.chunk".to_string(),
        created: 0,
        model: "agent-model".to_string(),
        content: String::new(),
        choices: vec![StreamingChoice {
            index: 0,
            delta: Delta {
                tool_calls: Some(vec![tool_call]),
                ..Default::default()
            },
            finish_reason: Some("tool_calls".to_string()),
            logprobs: None,
        }],
        reasoning_content: None,
        usage: None,
        system_fingerprint: None,
    }
}
