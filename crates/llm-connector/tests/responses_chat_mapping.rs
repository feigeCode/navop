//! `/responses` 与 `/chat/completions` 之间的请求/响应互转测试。
//!
//! 供应商设置里选择「Responses」后，navop 仍然用 `ChatRequest`/`ChatResponse`
//! 与上层交互，因此这里守住互转逻辑，确保文本、工具调用、推理与流式分片都能
//! 正确落到 chat 结构上。

use std::collections::HashMap;

use llm_connector::responses_chat::{
    ResponsesChatStreamMapper, chat_request_to_responses_request,
    responses_response_to_chat_response,
};
use llm_connector::types::{
    ChatRequest, ChatResponse, Choice, FunctionCall, Message, MessageBlock, ReasoningEffort,
    ResponseFormat, ResponsesOutputContent, ResponsesOutputItem, ResponsesResponse,
    ResponsesStreamEvent, ResponsesUsage, Role, Tool, ToolCall, ToolChoice,
};

fn event(event_type: &str, data: serde_json::Value) -> ResponsesStreamEvent {
    ResponsesStreamEvent {
        event_type: event_type.to_string(),
        data: data
            .as_object()
            .expect("测试事件 data 必须是对象")
            .clone()
            .into_iter()
            .collect::<HashMap<String, serde_json::Value>>(),
    }
}

fn weather_tool_call() -> ToolCall {
    ToolCall {
        id: "call_1".to_string(),
        call_type: "function".to_string(),
        function: FunctionCall {
            name: "get_weather".to_string(),
            arguments: r#"{"city":"北京"}"#.to_string(),
            thought_signature: None,
        },
        index: None,
        thought_signature: None,
    }
}

#[test]
fn chat_request_maps_system_instructions_and_user_input() {
    let request = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![Message::system("be concise"), Message::user("你好")],
        temperature: Some(0.2),
        max_tokens: Some(512),
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");

    assert_eq!("gpt-4.1", mapped.model);
    assert_eq!(Some("be concise"), mapped.instructions.as_deref());
    assert_eq!(Some(512), mapped.max_output_tokens);
    assert_eq!(Some(0.2), mapped.temperature);

    let items = mapped
        .input
        .as_ref()
        .expect("input 应存在")
        .as_array()
        .unwrap();
    assert_eq!(1, items.len(), "system 消息应折叠进 instructions");
    assert_eq!("user", items[0]["role"]);
    assert_eq!("input_text", items[0]["content"][0]["type"]);
    assert_eq!("你好", items[0]["content"][0]["text"]);
}

#[test]
fn chat_request_maps_tools_to_flat_responses_shape() {
    let request = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![Message::user("查一下北京天气")],
        tools: Some(vec![Tool::function(
            "get_weather",
            Some("查询天气".to_string()),
            serde_json::json!({
                "type": "object",
                "properties": {"city": {"type": "string"}},
                "required": ["city"],
            }),
        )]),
        tool_choice: Some(ToolChoice::function("get_weather")),
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");

    let tools = mapped.tools.as_ref().expect("tools 应存在");
    assert_eq!("function", tools[0]["type"]);
    assert_eq!("get_weather", tools[0]["name"]);
    assert_eq!("查询天气", tools[0]["description"]);
    assert!(
        tools[0].get("function").is_none(),
        "Responses 工具是扁平结构，不应嵌套 function"
    );

    let choice = mapped.tool_choice.as_ref().expect("tool_choice 应存在");
    assert_eq!("function", choice["type"]);
    assert_eq!("get_weather", choice["name"]);
}

#[test]
fn chat_request_maps_tool_choice_mode_to_string() {
    let request = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![Message::user("hi")],
        tool_choice: Some(ToolChoice::required()),
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");
    assert_eq!(
        Some("required"),
        mapped.tool_choice.as_ref().and_then(|v| v.as_str())
    );
}

#[test]
fn chat_request_maps_tool_history_to_function_call_items() {
    let request = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![
            Message::user("北京天气？"),
            Message::assistant_with_tool_calls(vec![weather_tool_call()]),
            Message::tool("晴，25℃", "call_1"),
        ],
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");
    let items = mapped
        .input
        .as_ref()
        .expect("input 应存在")
        .as_array()
        .unwrap();
    assert_eq!(3, items.len());

    assert_eq!("message", items[0]["type"]);
    assert_eq!("user", items[0]["role"]);

    assert_eq!("function_call", items[1]["type"]);
    assert_eq!("call_1", items[1]["call_id"]);
    assert_eq!("get_weather", items[1]["name"]);
    assert_eq!(r#"{"city":"北京"}"#, items[1]["arguments"]);

    assert_eq!("function_call_output", items[2]["type"]);
    assert_eq!("call_1", items[2]["call_id"]);
    assert_eq!("晴，25℃", items[2]["output"]);
}

#[test]
fn chat_request_maps_reasoning_and_response_format() {
    let request = ChatRequest {
        model: "gpt-5.1".to_string(),
        messages: vec![Message::user("hi")],
        reasoning_effort: Some(ReasoningEffort::High),
        response_format: Some(ResponseFormat::json_schema(
            "weather",
            serde_json::json!({"type": "object"}),
        )),
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");

    assert_eq!(
        Some("high"),
        mapped
            .extra
            .get("reasoning")
            .and_then(|v| v["effort"].as_str())
    );
    let format = mapped.extra.get("text").expect("text.format 应存在");
    assert_eq!("json_schema", format["format"]["type"]);
    assert_eq!("weather", format["format"]["name"]);
    assert_eq!(true, format["format"]["strict"]);
}

#[test]
fn chat_request_maps_image_block_to_input_image() {
    let request = ChatRequest {
        model: "gpt-4.1".to_string(),
        messages: vec![Message::new(
            Role::User,
            vec![
                MessageBlock::text("看图"),
                MessageBlock::image_base64("image/png", "abc123"),
            ],
        )],
        ..Default::default()
    };

    let mapped = chat_request_to_responses_request(&request).expect("请求应可映射");
    let items = mapped
        .input
        .as_ref()
        .expect("input 应存在")
        .as_array()
        .unwrap();
    let content = items[0]["content"].as_array().expect("content 应为数组");
    assert_eq!("input_image", content[1]["type"]);
    assert_eq!("data:image/png;base64,abc123", content[1]["image_url"]);
}

#[test]
fn responses_response_maps_text_reasoning_and_usage() {
    let response = ResponsesResponse {
        id: "resp_1".to_string(),
        object: "response".to_string(),
        created_at: 123,
        model: Some("gpt-4.1".to_string()),
        status: Some("completed".to_string()),
        output: Some(vec![
            ResponsesOutputItem {
                item_type: "reasoning".to_string(),
                id: Some("rs_1".to_string()),
                extra: HashMap::from([(
                    "summary".to_string(),
                    serde_json::json!([{"type": "summary_text", "text": "先思考一下"}]),
                )]),
                ..Default::default()
            },
            ResponsesOutputItem {
                item_type: "message".to_string(),
                role: Some("assistant".to_string()),
                content: Some(vec![ResponsesOutputContent {
                    content_type: "output_text".to_string(),
                    text: Some("你好".to_string()),
                    ..Default::default()
                }]),
                ..Default::default()
            },
        ]),
        output_text: "你好".to_string(),
        usage: Some(ResponsesUsage {
            input_tokens: Some(10),
            output_tokens: Some(4),
            total_tokens: Some(14),
            extra: HashMap::new(),
        }),
        ..Default::default()
    };

    let chat = responses_response_to_chat_response(&response);

    assert_eq!("resp_1", chat.id);
    assert_eq!("gpt-4.1", chat.model);
    assert_eq!(123, chat.created);
    assert_eq!("你好", chat.content);
    assert_eq!(Some("先思考一下"), chat.reasoning_content.as_deref());
    assert_eq!(Some("stop"), chat.choices[0].finish_reason.as_deref());

    let usage = chat.usage.expect("usage 应存在");
    assert_eq!(10, usage.prompt_tokens);
    assert_eq!(4, usage.completion_tokens);
    assert_eq!(14, usage.total_tokens);
}

#[test]
fn responses_response_maps_function_call_to_tool_calls() {
    let response = ResponsesResponse {
        id: "resp_2".to_string(),
        object: "response".to_string(),
        status: Some("completed".to_string()),
        output: Some(vec![ResponsesOutputItem {
            item_type: "function_call".to_string(),
            id: Some("fc_1".to_string()),
            name: Some("get_weather".to_string()),
            arguments: Some(r#"{"city":"北京"}"#.to_string()),
            extra: HashMap::from([("call_id".to_string(), serde_json::json!("call_1"))]),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let chat = responses_response_to_chat_response(&response);

    let tool_calls = chat
        .choices
        .first()
        .and_then(|choice| choice.message.tool_calls.clone())
        .expect("应映射出工具调用");
    assert_eq!(1, tool_calls.len());
    assert_eq!("call_1", tool_calls[0].id);
    assert_eq!("function", tool_calls[0].call_type);
    assert_eq!("get_weather", tool_calls[0].function.name);
    assert_eq!(r#"{"city":"北京"}"#, tool_calls[0].function.arguments);
    assert_eq!(Some("tool_calls"), chat.choices[0].finish_reason.as_deref());
}

#[test]
fn responses_stream_mapper_emits_role_then_text_delta() {
    let mut mapper = ResponsesChatStreamMapper::new();

    let created = event(
        "response.created",
        serde_json::json!({
            "response": {"id": "resp_1", "object": "response", "model": "gpt-4.1", "status": "in_progress"}
        }),
    );
    let first = mapper
        .map_event(&created)
        .expect("created 事件应可映射")
        .expect("应产出首个分片");
    assert_eq!("resp_1", first.id);
    assert_eq!("gpt-4.1", first.model);
    assert_eq!(Some(Role::Assistant), first.choices[0].delta.role);

    let delta = event(
        "response.output_text.delta",
        serde_json::json!({"delta": "你"}),
    );
    let chunk = mapper
        .map_event(&delta)
        .expect("文本增量应可映射")
        .expect("应产出文本分片");
    assert_eq!(Some("你"), chunk.choices[0].delta.content.as_deref());
    assert_eq!("你", chunk.content);
}

#[test]
fn responses_stream_mapper_emits_tool_call_deltas_and_finish_reason() {
    let mut mapper = ResponsesChatStreamMapper::new();
    let _ = mapper
        .map_event(&event(
            "response.created",
            serde_json::json!({"response": {"id": "resp_1", "object": "response"}}),
        ))
        .expect("created 事件应可映射");

    let added = event(
        "response.output_item.added",
        serde_json::json!({
            "item": {"type": "function_call", "id": "fc_1", "call_id": "call_1", "name": "get_weather", "arguments": ""}
        }),
    );
    let chunk = mapper
        .map_event(&added)
        .expect("工具起点应可映射")
        .expect("应产出工具分片");
    let tool_call = &chunk.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .expect("应带 tool_calls")[0];
    assert_eq!(Some(0), tool_call.index);
    assert_eq!("call_1", tool_call.id);
    assert_eq!("get_weather", tool_call.function.name);

    let arguments = event(
        "response.function_call_arguments.delta",
        serde_json::json!({"item_id": "fc_1", "delta": "{\"city\""}),
    );
    let chunk = mapper
        .map_event(&arguments)
        .expect("参数增量应可映射")
        .expect("应产出参数分片");
    let tool_call = &chunk.choices[0]
        .delta
        .tool_calls
        .as_ref()
        .expect("应带 tool_calls")[0];
    assert_eq!(Some(0), tool_call.index);
    assert_eq!("{\"city\"", tool_call.function.arguments);
    // 参数分片只携带 arguments。若这里带上 id/type/name，上层会把整条调用当成
    // 新快照覆盖，导致已捕获的工具名被清空（issue #609 的 `<empty>` 症状）。
    assert!(
        tool_call.id.is_empty()
            && tool_call.call_type.is_empty()
            && tool_call.function.name.is_empty(),
        "参数分片不应携带身份字段: id={:?}, type={:?}, name={:?}",
        tool_call.id,
        tool_call.call_type,
        tool_call.function.name
    );

    let completed = event(
        "response.completed",
        serde_json::json!({
            "response": {
                "id": "resp_1",
                "status": "completed",
                "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15}
            }
        }),
    );
    let chunk = mapper
        .map_event(&completed)
        .expect("完成事件应可映射")
        .expect("应产出结束分片");
    assert_eq!(
        Some("tool_calls"),
        chunk.choices[0].finish_reason.as_deref()
    );
    let usage = chunk.usage.expect("结束分片应带 usage");
    assert_eq!(10, usage.prompt_tokens);
    assert_eq!(5, usage.completion_tokens);
}

#[test]
fn responses_stream_mapper_maps_max_output_tokens_to_length() {
    let mut mapper = ResponsesChatStreamMapper::new();
    let incomplete = event(
        "response.incomplete",
        serde_json::json!({
            "response": {
                "id": "resp_1",
                "status": "incomplete",
                "incomplete_details": {"reason": "max_output_tokens"}
            }
        }),
    );

    let chunk = mapper
        .map_event(&incomplete)
        .expect("未完成事件应可映射")
        .expect("应产出结束分片");
    assert_eq!(Some("length"), chunk.choices[0].finish_reason.as_deref());
}

#[test]
fn responses_stream_mapper_reports_failed_response_as_error() {
    let mut mapper = ResponsesChatStreamMapper::new();
    let failed = event(
        "response.failed",
        serde_json::json!({
            "response": {"id": "resp_1", "status": "failed", "error": {"message": "模型过载"}}
        }),
    );

    let error = mapper.map_event(&failed).expect_err("失败事件应转成错误");
    assert!(
        error.to_string().contains("模型过载"),
        "unexpected: {error}"
    );
}

#[test]
fn chat_response_to_responses_response_keeps_tool_calls() {
    let chat = ChatResponse {
        id: "chatcmpl_1".to_string(),
        object: "chat.completion".to_string(),
        created: 1,
        model: "gpt-4.1".to_string(),
        choices: vec![Choice {
            index: 0,
            message: Message::assistant_with_tool_calls(vec![weather_tool_call()]),
            finish_reason: Some("tool_calls".to_string()),
            logprobs: None,
        }],
        ..Default::default()
    };

    let response = llm_connector::types::chat_response_to_responses_response(&chat);
    let output = response.output.expect("output 应存在");
    let function_call = output
        .iter()
        .find(|item| item.item_type == "function_call")
        .expect("回退路径不应丢掉工具调用");

    assert_eq!(Some("get_weather"), function_call.name.as_deref());
    assert_eq!(
        Some(r#"{"city":"北京"}"#),
        function_call.arguments.as_deref()
    );
    assert_eq!(
        Some("call_1"),
        function_call
            .extra
            .get("call_id")
            .and_then(|value| value.as_str())
    );
}
