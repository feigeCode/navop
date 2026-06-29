use super::*;
use agent_runtime::model::{Message, ModelStreamEvent, Tool, ToolChoice, collect_model_stream};
use llm_connector::types::{ChatResponse, Choice, Delta, FunctionCall, StreamingChoice};
use one_core::llm::StreamingResponse;
use std::sync::atomic::{AtomicUsize, Ordering};

fn sample_provider() -> Arc<dyn LlmProvider> {
    sample_provider_named("noop")
}

fn sample_provider_named(name: &'static str) -> Arc<dyn LlmProvider> {
    // 仅用于构造 LlmModelClient;这些测试不实际发起网络请求。
    struct NoopProvider {
        name: &'static str,
    }
    #[async_trait]
    impl LlmProvider for NoopProvider {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<String> {
            Ok(String::new())
        }
        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> anyhow::Result<one_core::llm::ChatStream> {
            Ok(Box::pin(stream::empty()))
        }
        async fn models(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }
        fn provider_name(&self) -> &str {
            self.name
        }
    }
    Arc::new(NoopProvider { name })
}

fn completion_provider_named(
    name: &'static str,
    response: ChatResponse,
) -> (Arc<dyn LlmProvider>, Arc<AtomicUsize>) {
    struct CompletionProvider {
        name: &'static str,
        response: ChatResponse,
        stream_calls: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl LlmProvider for CompletionProvider {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<String> {
            Ok(self.response.content.clone())
        }

        async fn chat_completion(&self, _request: &ChatRequest) -> anyhow::Result<ChatResponse> {
            Ok(self.response.clone())
        }

        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> anyhow::Result<one_core::llm::ChatStream> {
            self.stream_calls.fetch_add(1, Ordering::SeqCst);
            Ok(Box::pin(stream::empty()))
        }

        async fn models(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }

        fn provider_name(&self) -> &str {
            self.name
        }
    }

    let stream_calls = Arc::new(AtomicUsize::new(0));
    (
        Arc::new(CompletionProvider {
            name,
            response,
            stream_calls: stream_calls.clone(),
        }),
        stream_calls,
    )
}

fn streaming_provider_named(
    name: &'static str,
    chunks: Vec<StreamingResponse>,
) -> Arc<dyn LlmProvider> {
    struct StreamingProvider {
        name: &'static str,
        chunks: Vec<StreamingResponse>,
    }

    #[async_trait]
    impl LlmProvider for StreamingProvider {
        async fn chat(&self, _request: &ChatRequest) -> anyhow::Result<String> {
            Ok(String::new())
        }

        async fn chat_stream(
            &self,
            _request: &ChatRequest,
        ) -> anyhow::Result<one_core::llm::ChatStream> {
            let chunks = self.chunks.clone().into_iter().map(Ok);
            Ok(Box::pin(stream::iter(chunks)))
        }

        async fn models(&self) -> anyhow::Result<Vec<String>> {
            Ok(vec![])
        }

        fn provider_name(&self) -> &str {
            self.name
        }
    }

    Arc::new(StreamingProvider { name, chunks })
}

#[test]
fn maps_model_request_into_chat_request() {
    let client = LlmModelClient::new(sample_provider(), "gpt-test")
        .with_temperature(0.3)
        .with_max_tokens(1024);

    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_tools(vec![Tool::function("echo", None, serde_json::json!({}))])
        .with_tool_choice(ToolChoice::auto());

    let chat_request = client.to_chat_request(request);
    assert_eq!(chat_request.model, "gpt-test");
    assert_eq!(chat_request.messages.len(), 1);
    assert!(chat_request.tools.as_ref().is_some_and(|t| t.len() == 1));
    assert!(chat_request.tool_choice.is_some());
    assert_eq!(chat_request.temperature, Some(0.3));
    assert_eq!(chat_request.max_tokens, Some(1024));
}

#[test]
fn empty_tools_map_to_none() {
    let client = LlmModelClient::new(sample_provider(), "m");
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let chat_request = client.to_chat_request(request);
    assert!(chat_request.tools.is_none(), "无工具时应映射为 None");
}

#[test]
fn deepseek_v4_keeps_tools_and_tool_choice() {
    let client = LlmModelClient::new(sample_provider_named("deepseek"), "deepseek-v4-flash");
    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_tools(vec![Tool::function("echo", None, serde_json::json!({}))])
        .with_tool_choice(ToolChoice::auto());

    let chat_request = client.to_chat_request(request);

    assert!(chat_request.tools.as_ref().is_some_and(|t| t.len() == 1));
    assert!(chat_request.tool_choice.is_some());
}

#[test]
fn non_thinking_models_keep_tool_choice() {
    let client = LlmModelClient::new(sample_provider_named("deepseek"), "deepseek-chat");
    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_tools(vec![Tool::function("echo", None, serde_json::json!({}))])
        .with_tool_choice(ToolChoice::auto());

    let chat_request = client.to_chat_request(request);

    assert!(chat_request.tools.as_ref().is_some_and(|t| t.len() == 1));
    assert!(chat_request.tool_choice.is_some());
}

#[test]
fn ollama_keeps_tools_but_drops_tool_choice() {
    let client = LlmModelClient::new(sample_provider_named("ollama"), "qwen3:14b");
    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_tools(vec![Tool::function("echo", None, serde_json::json!({}))])
        .with_tool_choice(ToolChoice::auto());

    let chat_request = client.to_chat_request(request);

    assert!(
        chat_request
            .tools
            .as_ref()
            .is_some_and(|tools| tools.len() == 1)
    );
    assert!(
        chat_request.tool_choice.is_none(),
        "ollama rejects explicit tool_choice but can still receive tools"
    );
}

#[tokio::test]
async fn ollama_stream_with_tools_uses_completion_fallback_for_tool_calls() {
    let expected_call = tool_call(0, "call_echo", "echo", "{\"message\":\"db-1\"}");
    let response = ChatResponse {
        choices: vec![Choice {
            index: 0,
            message: Message::assistant_with_tool_calls(vec![expected_call.clone()]),
            finish_reason: Some("tool_calls".to_string()),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (provider, stream_calls) = completion_provider_named("ollama", response);
    let client = LlmModelClient::new(provider, "qwen3:14b");
    let request = ModelRequest::new(vec![Message::user("hi")])
        .with_tools(vec![Tool::function("echo", None, serde_json::json!({}))])
        .with_tool_choice(ToolChoice::auto());

    let stream = client
        .complete_stream(request)
        .await
        .expect("stream should be created");
    let response = collect_model_stream(stream)
        .await
        .expect("stream should collect");

    assert_eq!(0, stream_calls.load(Ordering::SeqCst));
    assert_eq!(1, response.tool_calls.len());
    assert_eq!("echo", response.tool_calls[0].function.name);
    assert_eq!(
        "{\"message\":\"db-1\"}",
        response.tool_calls[0].function.arguments
    );
}

#[tokio::test]
async fn stream_reasoning_and_content_are_distinct_events() {
    let provider = streaming_provider_named(
        "test",
        vec![
            StreamingResponse {
                choices: vec![StreamingChoice {
                    index: 0,
                    delta: Delta {
                        thinking: Some("内部思考".to_string()),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }],
                ..Default::default()
            },
            StreamingResponse {
                choices: vec![StreamingChoice {
                    index: 0,
                    delta: Delta {
                        content: Some("正式回复".to_string()),
                        ..Default::default()
                    },
                    finish_reason: None,
                    logprobs: None,
                }],
                ..Default::default()
            },
        ],
    );
    let client = LlmModelClient::new(provider, "thinking-model");
    let request = ModelRequest::new(vec![Message::user("hi")]);
    let mut stream = client
        .complete_stream(request)
        .await
        .expect("stream should be created");

    let first = stream.next().await.expect("reasoning event").expect("ok");
    assert!(
        matches!(first, ModelStreamEvent::ReasoningDelta(delta) if delta == "内部思考"),
        "thinking delta must not be emitted as visible assistant text"
    );
    let second = stream.next().await.expect("text event").expect("ok");
    assert!(matches!(second, ModelStreamEvent::TextDelta(delta) if delta == "正式回复"));
    let third = stream.next().await.expect("completed event").expect("ok");
    assert!(
        matches!(third, ModelStreamEvent::Completed(response) if response.text.as_deref() == Some("正式回复"))
    );
}

#[test]
fn model_name_reflects_configured_model() {
    let client = LlmModelClient::new(sample_provider(), "my-model");
    assert_eq!(client.model_name(), "my-model");
}

fn tool_call(index: usize, id: &str, name: &str, args: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        call_type: "function".into(),
        function: FunctionCall {
            name: name.into(),
            arguments: args.into(),
            ..Default::default()
        },
        index: Some(index),
        ..Default::default()
    }
}

fn tool_call_delta(index: usize, args: &str) -> ToolCall {
    let mut call = tool_call(index, "", "", args);
    call.call_type.clear();
    call
}

fn chunk(calls: Vec<ToolCall>) -> StreamingResponse {
    StreamingResponse {
        choices: vec![StreamingChoice {
            index: 0,
            delta: Delta {
                tool_calls: Some(calls),
                ..Default::default()
            },
            finish_reason: None,
            logprobs: None,
        }],
        ..Default::default()
    }
}

#[test]
fn growing_snapshots_merge_into_single_call() {
    // 模拟连接器:每个分片都重发完整快照(带 id+index,arguments 递增)。
    let mut acc = Vec::new();
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![tool_call(0, "c1", "update_plan", "{\"plan\":")]),
    );
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![tool_call(
            0,
            "c1",
            "update_plan",
            "{\"plan\":[{\"step\":\"a\",",
        )]),
    );
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![tool_call(
            0,
            "c1",
            "update_plan",
            "{\"plan\":[{\"step\":\"a\",\"status\":\"pending\"}]}",
        )]),
    );

    assert_eq!(
        acc.len(),
        1,
        "同一调用的多个增长快照必须合并为一个,不能按前缀拆成多次调用"
    );
    assert_eq!(
        acc[0].function.arguments,
        "{\"plan\":[{\"step\":\"a\",\"status\":\"pending\"}]}"
    );
    serde_json::from_str::<serde_json::Value>(&acc[0].function.arguments)
        .expect("最终累积的参数应为合法 JSON(旧逻辑会留下不完整前缀触发 EOF)");
}

#[test]
fn partial_argument_deltas_preserve_initial_tool_identity() {
    let mut acc = Vec::new();
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![tool_call(0, "call_00", "connections_test", "")]),
    );
    for part in ["{", "\"", "connection", "\"", ": ", "\"", "5", "\"", "}"] {
        merge_stream_tool_calls(&mut acc, &chunk(vec![tool_call_delta(0, part)]));
    }

    assert_eq!(acc.len(), 1);
    assert_eq!(acc[0].id, "call_00");
    assert_eq!(acc[0].call_type, "function");
    assert_eq!(acc[0].function.name, "connections_test");
    assert_eq!(acc[0].function.arguments, "{\"connection\": \"5\"}");
}

#[test]
fn keeps_distinct_tool_calls_by_index() {
    let mut acc = Vec::new();
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![
            tool_call(0, "ca", "echo", "{\"a\":1}"),
            tool_call(1, "cb", "update_plan", "{"),
        ]),
    );
    merge_stream_tool_calls(
        &mut acc,
        &chunk(vec![
            tool_call(0, "ca", "echo", "{\"a\":1}"),
            tool_call(1, "cb", "update_plan", "{\"plan\":[]}"),
        ]),
    );

    assert_eq!(acc.len(), 2, "不同 index 的调用应各自保留");
    assert_eq!(acc[0].function.arguments, "{\"a\":1}");
    assert_eq!(acc[1].function.arguments, "{\"plan\":[]}");
}

#[test]
fn falls_back_to_id_when_index_absent() {
    // 个别 provider 不带 index:用非空 id 匹配,仍应覆盖而非追加。
    let mut acc = Vec::new();
    let mut first = tool_call(0, "cx", "echo", "{\"k\":");
    first.index = None;
    let mut second = tool_call(0, "cx", "echo", "{\"k\":1}");
    second.index = None;
    merge_stream_tool_calls(&mut acc, &chunk(vec![first]));
    merge_stream_tool_calls(&mut acc, &chunk(vec![second]));

    assert_eq!(acc.len(), 1);
    assert_eq!(acc[0].function.arguments, "{\"k\":1}");
}
