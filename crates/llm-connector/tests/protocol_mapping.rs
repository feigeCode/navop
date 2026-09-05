//! Protocol Mapping Tests (V2)
//!
//! Verified the correctness of request building and response parsing for each provider.

use llm_connector::core::Protocol;
use llm_connector::protocols::adapters::aliyun::AliyunProtocol;
use llm_connector::protocols::adapters::anthropic::AnthropicProtocol;
use llm_connector::protocols::adapters::openai::OpenAIProtocol;
use llm_connector::protocols::adapters::zhipu::ZhipuProtocol;
use llm_connector::types::{AnthropicToolChoice, AnthropicToolDefinition, ChatRequest, Message};

#[test]
fn test_openai_request_mapping() {
    let protocol = OpenAIProtocol::new("test-key");
    let request = ChatRequest::new("gpt-4").add_message(Message::user("Hello"));

    let mapped = protocol.build_request(&request).unwrap();
    assert_eq!(mapped.model, "gpt-4");
    assert_eq!(mapped.messages.len(), 1);
    assert_eq!(mapped.messages[0]["role"], "user");
    assert_eq!(mapped.messages[0]["content"], "Hello");
}

#[test]
fn test_anthropic_request_mapping() {
    let protocol = AnthropicProtocol::new("test-key");
    let request = ChatRequest::new("claude-3")
        .add_message(Message::system("System prompt"))
        .add_message(Message::user("Hello"));

    let mapped = protocol.build_request(&request).unwrap();
    assert_eq!(mapped.model, "claude-3");
    assert_eq!(mapped.system.unwrap(), "System prompt");
    assert_eq!(mapped.messages.len(), 1);
    assert_eq!(mapped.messages[0].role, "user");
}

#[test]
fn test_zhipu_request_mapping() {
    let protocol = ZhipuProtocol::new("test-key");
    let request = ChatRequest::new("glm-4").add_message(Message::user("Hello zhipu"));

    let mapped = protocol.build_request(&request).unwrap();
    assert_eq!(mapped.model, "glm-4");
    assert_eq!(mapped.messages.len(), 1);
    assert_eq!(mapped.messages[0]["role"], "user");
}

#[test]
fn test_aliyun_request_mapping() {
    let protocol = AliyunProtocol::new("test-key");
    let request = ChatRequest::new("qwen-max").add_message(Message::user("Hello aliyun"));

    let mapped = protocol.build_request(&request).unwrap();
    assert_eq!(mapped.model, "qwen-max");
    assert_eq!(mapped.input.messages.len(), 1);
    assert_eq!(mapped.input.messages[0]["role"], "user");
}

#[test]
fn test_anthropic_native_tool_mapping_from_public_request_model() {
    let protocol = AnthropicProtocol::new("test-key");
    let request = ChatRequest::new("claude-opus-4-6")
        .with_anthropic_tools(vec![AnthropicToolDefinition {
            tool_type: Some("custom".to_string()),
            name: Some("write_file".to_string()),
            description: Some("Write a file to disk".to_string()),
            input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "path": { "type": "string" },
                    "content": { "type": "string" }
                },
                "required": ["path", "content"]
            })),
            custom_input_schema: Some(serde_json::json!({
                "$schema": "https://json-schema.org/draft/2020-12/schema",
                "type": "object",
                "properties": {
                    "mode": { "enum": ["overwrite", "append"] }
                }
            })),
            input_examples: Some(serde_json::json!([
                {
                    "path": "/tmp/demo.txt",
                    "content": "hello",
                    "mode": "overwrite"
                }
            ])),
            strict: Some(true),
            allowed_callers: Some(serde_json::json!(["claude_code", "xrouter"])),
            defer_loading: Some(true),
            cache_control: Some(serde_json::json!({"type": "ephemeral"})),
            eager_input_streaming: Some(true),
            extra: std::collections::HashMap::from([(
                "mcp_toolset".to_string(),
                serde_json::json!({"server": "filesystem"}),
            )]),
        }])
        .with_anthropic_tool_choice(AnthropicToolChoice::tool("write_file"))
        .add_message(Message::user("write a file"));

    let mapped = protocol.build_request(&request).unwrap();
    let payload = serde_json::to_value(&mapped).unwrap();

    assert_eq!(payload["tools"][0]["type"], "custom");
    assert_eq!(payload["tools"][0]["name"], "write_file");
    assert_eq!(
        payload["tools"][0]["input_schema"]["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        payload["tools"][0]["custom_input_schema"]["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(payload["tools"][0]["strict"], true);
    assert_eq!(payload["tools"][0]["allowed_callers"][1], "xrouter");
    assert_eq!(payload["tools"][0]["defer_loading"], true);
    assert_eq!(payload["tools"][0]["eager_input_streaming"], true);
    assert_eq!(payload["tools"][0]["mcp_toolset"]["server"], "filesystem");
    assert_eq!(payload["tool_choice"]["type"], "tool");
    assert_eq!(payload["tool_choice"]["name"], "write_file");
}
