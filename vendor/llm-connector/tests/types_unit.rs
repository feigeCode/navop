//! Types Unit Tests
//!
//! Verified the core data types and their helpers.

use llm_connector::types::{ChatRequest, Message, MessageBlock, Role, StreamingResponse, ToolCall};

#[test]
fn test_message_creation() {
    let msg = Message::user("Hello");
    assert_eq!(msg.role, Role::User);
    assert_eq!(msg.content_as_text(), "Hello");

    let sys = Message::system("System");
    assert_eq!(sys.role, Role::System);
    assert_eq!(sys.content_as_text(), "System");
}

#[test]
fn test_chat_request_builder() {
    let req = ChatRequest::new("model")
        .add_message(Message::user("h1"))
        .with_temperature(0.5)
        .with_max_tokens(100)
        .with_stream(true); // Added with_stream

    assert_eq!(req.model, "model");
    assert_eq!(req.messages.len(), 1);
    assert_eq!(req.temperature, Some(0.5));
    assert_eq!(req.max_tokens, Some(100));
    assert_eq!(req.stream, Some(true)); // Added assertion for stream
}

#[test]
fn test_message_blocks() {
    let msg = Message::new(
        Role::User,
        vec![MessageBlock::text("t1"), MessageBlock::text("t2")],
    );
    assert_eq!(msg.content.len(), 2);
    assert_eq!(msg.content_as_text(), "t1\nt2");
}

#[test]
fn tool_call_null_strings_deserialize_as_empty() {
    let call: ToolCall = serde_json::from_str(
        r#"{
            "id": null,
            "type": null,
            "function": {
                "name": null,
                "arguments": null
            }
        }"#,
    )
    .unwrap();

    assert_eq!(call.id, "");
    assert_eq!(call.call_type, "");
    assert_eq!(call.function.name, "");
    assert_eq!(call.function.arguments, "");

    let serialized = serde_json::to_value(call).unwrap();
    assert!(serialized.get("id").is_none());
    assert!(serialized.get("type").is_none());
    assert!(serialized["function"].get("name").is_none());
    assert!(serialized["function"].get("arguments").is_none());

    let missing: ToolCall = serde_json::from_str(r#"{"function": {}}"#).unwrap();
    assert_eq!(missing.id, "");
    assert_eq!(missing.call_type, "");
    assert_eq!(missing.function.name, "");
    assert_eq!(missing.function.arguments, "");

    let populated: ToolCall = serde_json::from_str(
        r#"{
            "id": "call_123",
            "type": "function",
            "function": {
                "name": "lookup",
                "arguments": "{\"query\":\"test\"}"
            }
        }"#,
    )
    .unwrap();
    assert_eq!(populated.id, "call_123");
    assert_eq!(populated.call_type, "function");
    assert_eq!(populated.function.name, "lookup");
    assert_eq!(populated.function.arguments, r#"{"query":"test"}"#);
}

#[test]
fn siliconflow_streaming_tool_call_chunk_deserializes() {
    let response: StreamingResponse = serde_json::from_str(
        r#"{
            "id": "019fb5ca7ff8bafa3dc48338beb8a4a3",
            "object": "chat.completion.chunk",
            "created": 1785461375,
            "model": "deepseek-ai/DeepSeek-V4-Flash",
            "choices": [
                {
                    "index": 0,
                    "delta": {
                        "content": null,
                        "tool_calls": [
                            {
                                "index": 0,
                                "id": null,
                                "type": null,
                                "function": {
                                    "name": "",
                                    "arguments": "{}"
                                }
                            }
                        ]
                    },
                    "finish_reason": null
                }
            ],
            "system_fingerprint": "",
            "usage": {
                "prompt_tokens": 12057,
                "completion_tokens": 82,
                "total_tokens": 12139,
                "completion_tokens_details": {
                    "reasoning_tokens": 54
                },
                "prompt_tokens_details": {
                    "cached_tokens": 0
                },
                "prompt_cache_hit_tokens": 0,
                "prompt_cache_miss_tokens": 12057
            }
        }"#,
    )
    .unwrap();

    assert_eq!(response.model, "deepseek-ai/DeepSeek-V4-Flash");

    let tool_call = &response.choices[0].delta.tool_calls.as_ref().unwrap()[0];
    assert_eq!(tool_call.index, Some(0));
    assert_eq!(tool_call.id, "");
    assert_eq!(tool_call.call_type, "");
    assert_eq!(tool_call.function.name, "");
    assert_eq!(tool_call.function.arguments, "{}");

    let usage = response.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 12057);
    assert_eq!(usage.completion_tokens, 82);
    assert_eq!(usage.total_tokens, 12139);
    assert_eq!(usage.prompt_cache_hit_tokens, Some(0));
    assert_eq!(usage.prompt_cache_miss_tokens, Some(12057));
    assert_eq!(
        usage.completion_tokens_details.unwrap().reasoning_tokens,
        Some(54)
    );
    assert_eq!(usage.prompt_tokens_details.unwrap().cached_tokens, Some(0));
}
