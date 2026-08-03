//! Shared Utilities for OpenAI-Compatible Protocols
//!
//! Many LLM providers (Zhipu, Aliyun, DeepSeek, etc.) adopt an API structure
//! that is highly compatible with the official OpenAI specification.
//! This module provides shared parsing and conversion utilities to reduce
//! boilerplate across different protocol implementations.

use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::StreamReasoningStrategy;
use crate::types::{ChatResponse, EmbedResponse, EmbeddingData, Usage};
use serde::Deserialize;

// ============================================================================
// Standard OpenAI Compatible Response Types (Internal Parsing)
// ============================================================================

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsResponse {
    pub id: Option<String>,
    pub object: Option<String>,
    pub created: Option<u64>,
    pub model: Option<String>,
    pub choices: Option<Vec<ChatCompletionsChoice>>,
    pub usage: Option<ChatCompletionsUsage>,
    pub system_fingerprint: Option<String>,

    // Potentially proprietary fields we ignore or handle specially
    #[serde(default)]
    pub request_id: Option<String>,

    #[serde(default)]
    pub output: Option<ChatCompletionsOutput>,
}

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsOutput {
    pub choices: Option<Vec<ChatCompletionsChoice>>,
    pub usage: Option<ChatCompletionsUsage>,
}

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsChoice {
    pub index: Option<u32>,
    pub message: Option<ChatCompletionsMessage>,
    pub delta: Option<ChatCompletionsMessage>, // For streaming
    pub finish_reason: Option<String>,
}

#[derive(Deserialize, Debug, Clone)]
pub struct ChatCompletionsMessage {
    #[allow(dead_code)]
    pub role: Option<String>,
    pub content: Option<String>,
    pub tool_calls: Option<serde_json::Value>,
    // DeepSeek reasoning content Extension
    pub reasoning_content: Option<String>,
}

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsUsage {
    pub prompt_tokens: Option<u32>,
    pub completion_tokens: Option<u32>,
    pub total_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_cache_hit_tokens: Option<u32>,
    #[serde(default)]
    pub prompt_cache_miss_tokens: Option<u32>,
}

// ============================================================================
// Shared Parsers
// ============================================================================

/// Parse a standard OpenAI-compatible JSON response into a ChatResponse
pub fn parse_chat_completions_chat_response(
    response: &str,
    provider_name: &str,
    stream_reasoning_strategy: StreamReasoningStrategy,
) -> Result<ChatResponse, LlmConnectorError> {
    let raw: ChatCompletionsResponse = serde_json::from_str(response)
        .map_err(|e| LlmConnectorError::ParseError(format!("{}: {}", provider_name, e)))?;

    let ChatCompletionsResponse {
        id,
        object,
        created,
        model,
        choices,
        usage,
        system_fingerprint,
        request_id,
        output,
    } = raw;

    let (output_choices, output_usage) = match output {
        Some(output) => (output.choices, output.usage),
        None => (None, None),
    };

    let effective_choices = choices.or(output_choices);
    let effective_usage = usage.or(output_usage);

    // Extract usage
    let usage = effective_usage.map(|u| Usage {
        prompt_tokens: u.prompt_tokens.unwrap_or(0),
        completion_tokens: u.completion_tokens.unwrap_or(0),
        total_tokens: u.total_tokens.unwrap_or(0),
        prompt_cache_hit_tokens: u.prompt_cache_hit_tokens,
        prompt_cache_miss_tokens: u.prompt_cache_miss_tokens,
        ..Default::default()
    });

    // Extract choices
    let mut mapped_choices = Vec::new();
    let mut main_content = String::new();
    let mut main_reasoning = None;

    if let Some(choices) = effective_choices {
        for choice in choices {
            let msg_source = choice.message.or(choice.delta); // Support standard and delta
            if let Some(msg) = msg_source {
                let normalized = crate::protocols::common::openai_compatible::normalize_openai_compatible_content(
                    msg.content,
                    msg.reasoning_content,
                    stream_reasoning_strategy,
                );
                let content_str = normalized.content;
                let reasoning_str = normalized.reasoning;

                // Keep the first choice's content as main
                if choice.index.unwrap_or(0) == 0 {
                    main_content = content_str.clone();
                    main_reasoning = reasoning_str.clone();
                }

                let mapped_tool_calls =
                    crate::protocols::common::openai_compatible::map_openai_compatible_tool_calls(
                        msg.tool_calls,
                    );

                let mut final_message = if let Some(tc) = mapped_tool_calls {
                    crate::types::Message::assistant_with_tool_calls(tc)
                } else {
                    crate::types::Message::assistant(&content_str)
                };

                final_message.reasoning_content = reasoning_str;

                mapped_choices.push(crate::types::Choice {
                    index: choice.index.unwrap_or(0),
                    message: final_message,
                    finish_reason: choice.finish_reason,
                    logprobs: None, // Simplified for now
                });
            }
        }
    }

    Ok(ChatResponse {
        id: id.unwrap_or_else(|| request_id.unwrap_or_default()),
        object: object.unwrap_or_else(|| "chat.completion".to_string()),
        created: created.unwrap_or(0),
        model: model.unwrap_or_default(),
        choices: mapped_choices,
        content: main_content,
        reasoning_content: main_reasoning,
        usage,
        system_fingerprint,
    })
}

#[cfg(test)]
mod tests {
    use super::parse_chat_completions_chat_response;
    use crate::protocols::common::capabilities::StreamReasoningStrategy;

    #[test]
    fn test_parse_dashscope_wrapped_chat_response() {
        let response = r#"{
            "output": {
                "choices": [
                    {
                        "finish_reason": "stop",
                        "message": {
                            "role": "assistant",
                            "content": "Hello from DashScope"
                        }
                    }
                ]
            },
            "usage": {
                "input_tokens": 13,
                "output_tokens": 11,
                "total_tokens": 24
            },
            "request_id": "req_dashscope_1"
        }"#;

        let parsed = parse_chat_completions_chat_response(
            response,
            "aliyun",
            StreamReasoningStrategy::SeparateField,
        )
        .expect("should parse dashscope wrapped response");

        assert_eq!(parsed.id, "req_dashscope_1");
        assert_eq!(parsed.content, "Hello from DashScope");
        assert_eq!(parsed.choices.len(), 1);
        assert_eq!(
            parsed.choices[0].message.content_as_text(),
            "Hello from DashScope"
        );
        assert_eq!(parsed.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.usage.as_ref().map(|u| u.total_tokens), Some(24));
    }

    #[test]
    fn test_parse_chat_response_embedded_think_tags_respects_strategy() {
        let response = r#"{
            "id": "chatcmpl-test",
            "object": "chat.completion",
            "created": 123,
            "model": "test-model",
            "choices": [
                {
                    "index": 0,
                    "message": {
                        "role": "assistant",
                        "content": "<think>step by step</think>final answer"
                    },
                    "finish_reason": "stop"
                }
            ]
        }"#;

        let embedded = parse_chat_completions_chat_response(
            response,
            "deepseek",
            StreamReasoningStrategy::EmbeddedThinkTags,
        )
        .expect("embedded think tags strategy should parse");

        assert_eq!(embedded.content, "final answer");
        assert_eq!(embedded.reasoning_content.as_deref(), Some("step by step"));

        let separate = parse_chat_completions_chat_response(
            response,
            "zhipu",
            StreamReasoningStrategy::SeparateField,
        )
        .expect("separate field strategy should parse");

        assert_eq!(separate.content, "<think>step by step</think>final answer");
        assert_eq!(separate.reasoning_content, None);
    }
}

// ============================================================================
// Standard OpenAI Compatible Embedding Types
// ============================================================================

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsEmbedResponse {
    pub object: Option<String>,
    pub data: Option<Vec<ChatCompletionsEmbedData>>,
    pub model: Option<String>,
    pub usage: Option<ChatCompletionsUsage>,
}

#[derive(Deserialize, Debug)]
pub struct ChatCompletionsEmbedData {
    pub object: Option<String>,
    pub embedding: Vec<f32>,
    pub index: u32,
}

/// Parse a standard OpenAI-compatible embedding JSON response into an EmbedResponse
pub fn parse_chat_completions_embed_response(
    response: &str,
    provider_name: &str,
) -> Result<EmbedResponse, LlmConnectorError> {
    let raw: ChatCompletionsEmbedResponse = serde_json::from_str(response)
        .map_err(|e| LlmConnectorError::ParseError(format!("{}: {}", provider_name, e)))?;

    // Extract usage
    let usage = raw
        .usage
        .map(|u| Usage {
            prompt_tokens: u.prompt_tokens.unwrap_or(0),
            completion_tokens: u.completion_tokens.unwrap_or(0),
            total_tokens: u.total_tokens.unwrap_or(0),
            prompt_cache_hit_tokens: u.prompt_cache_hit_tokens,
            prompt_cache_miss_tokens: u.prompt_cache_miss_tokens,
            ..Default::default()
        })
        .unwrap_or_default();

    // Extract embeddings data
    let mut data = Vec::new();
    if let Some(raw_data) = raw.data {
        for item in raw_data {
            data.push(EmbeddingData {
                object: item.object.unwrap_or_else(|| "embedding".to_string()),
                embedding: item.embedding,
                index: item.index,
            });
        }
    }

    Ok(EmbedResponse {
        object: raw.object.unwrap_or_else(|| "list".to_string()),
        data,
        model: raw.model.unwrap_or_else(|| "unknown".to_string()),
        usage,
    })
}
