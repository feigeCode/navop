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

/// 截断响应体，用于把畸形响应带进错误信息。
///
/// 必须走字符边界：响应体是 UTF-8，直接 `&body[..n]` 在 n 落进多字节字符内部时
/// 会让报错路径自己 panic —— 比原始错误更难查。
fn body_preview(body: &str) -> &str {
    const PREVIEW_BYTES: usize = 512;
    &body[..body.floor_char_boundary(PREVIEW_BYTES.min(body.len()))]
}

/// 响应体里「第一个完整 JSON 值结束在哪里」，以及它前后各一段原文。
///
/// `trailing characters at line 1 column N` 这类错的 N 是位置而不是长度，实测见过
/// N=1007 —— 只给开头 512 字节等于没给，恰好看不到出错点。而 N 的单位（字节还是
/// 字符）serde 没写死，自己换算容易错，所以这里不猜：用流式解析问它「第一个键值
/// 在哪结束」——`StreamDeserializer::byte_offset` 是**精确字节数** —— 再把那段窗口
/// 取出来。serde 报 trailing，就是那里后面还有东西。
///
/// 正常 body、或整个 body 连第一个值都解析不出来（语法错在中间）时返回 `None`，
/// 这时开头的预览已经够用。
fn trailing_boundary(body: &str) -> Option<String> {
    /// 边界前后各取多少字节。
    const WINDOW_BYTES: usize = 200;
    let mut values = serde_json::Deserializer::from_str(body).into_iter::<serde_json::Value>();
    values.next()?.ok()?;
    let boundary = values.byte_offset();
    if boundary >= body.len() {
        return None;
    }
    // 边界本身一定是字符边界（serde 停在值的末尾），两端各自回退到最近的字符边界。
    let start = body.floor_char_boundary(boundary.saturating_sub(WINDOW_BYTES));
    let end = body.floor_char_boundary((boundary + WINDOW_BYTES).min(body.len()));
    Some(format!(
        "first JSON value ends at byte {boundary}: {}<|>{}",
        &body[start..boundary],
        &body[boundary..end]
    ))
}

/// 拼错误信息里的响应体诊断：开头预览，外加（若是 trailing 类畸形）出错边界处的窗口。
fn body_diagnostic(body: &str) -> String {
    match trailing_boundary(body) {
        Some(boundary) => format!("{} | {boundary}", body_preview(body)),
        None => body_preview(body).to_string(),
    }
}

/// Parse a standard OpenAI-compatible JSON response into a ChatResponse
pub fn parse_chat_completions_chat_response(
    response: &str,
    provider_name: &str,
    stream_reasoning_strategy: StreamReasoningStrategy,
) -> Result<ChatResponse, LlmConnectorError> {
    let raw: ChatCompletionsResponse = serde_json::from_str(response).map_err(|e| {
        // 带上响应体：像「trailing characters at line 1 column 1007」这种错，
        // 光看 serde 的位置信息无法判断上游到底回了什么形状的 body。
        LlmConnectorError::ParseError(format!(
            "{}: {} | body: {}",
            provider_name,
            e,
            body_diagnostic(response)
        ))
    })?;

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

    /// 解析失败必须能看出上游回了什么形状的 body；预览又必须是有界且不切字符。
    #[test]
    fn test_parse_error_carries_a_bounded_char_boundary_safe_body_preview() {
        // 形状取自线上实测错误：`9Router: trailing characters at line 1 column 975`
        // —— 一个完整的 JSON 对象后面还粘着别的东西。
        let body = format!("{{\"ok\":1}}{}", "中".repeat(400));
        let error = parse_chat_completions_chat_response(
            &body,
            "9Router",
            StreamReasoningStrategy::SeparateField,
        )
        .expect_err("非法 body 必须报错");
        let message = error.to_string();

        assert!(message.contains("9Router"), "{message}");
        assert!(message.contains("trailing characters"), "{message}");
        assert!(message.contains("| body: {\"ok\":1}"), "{message}");
        // 512 字节预算落在「中」中间（3 字节对齐到 510），预览必须回退到边界；
        // 同时整条消息要有界，不能把整个响应体塞进日志。
        assert!(message.len() < 1024, "预览必须有界: {} 字节", message.len());
    }

    /// trailing 类错误的边界常常远超 512 字节的头部预览 —— 预览必须够得着它。
    #[test]
    fn test_parse_error_preview_reaches_a_boundary_past_the_head_preview() {
        // 形状与位置取自线上实测：`9Router: trailing characters at line 1 column 1007`。
        // 头部预览只到 512 字节，正好看不到 1007，等于没给。
        let filler = "中".repeat(300); // 900 字节，把边界推到头部预览之外
        let body = format!(
            "{{\"id\":\"x\",\"choices\":[{{\"message\":{{\"content\":\"{filler}\"}}}}]}}{{\"again\":true}}"
        );
        let boundary = body
            .find("{\"again\"")
            .expect("trailing object starts here");
        assert!(
            boundary > 512,
            "边界必须落在头部预览之外，否则这条测试是空转的（{boundary}）"
        );

        let error = parse_chat_completions_chat_response(
            &body,
            "9Router",
            StreamReasoningStrategy::SeparateField,
        )
        .expect_err("非法 body 必须报错");
        let message = error.to_string();

        assert!(
            message.contains("first JSON value ends at byte"),
            "{message}"
        );
        // 边界**之后**的原文是头部预览永远给不出的部分 —— 正是「上游多回了什么」。
        assert!(
            message.contains("\"again\""),
            "必须能看见边界之后的内容: {message}"
        );
    }

    /// 中间语法错（连第一个值都解析不出来）时给不出边界，只能靠头部预览，且仍须有界。
    #[test]
    fn test_parse_error_without_a_parseable_first_value_falls_back_to_the_head() {
        let body = format!("{{\"ok\": {}}}", "中".repeat(400));
        let error = parse_chat_completions_chat_response(
            &body,
            "9Router",
            StreamReasoningStrategy::SeparateField,
        )
        .expect_err("非法 body 必须报错");
        let message = error.to_string();

        assert!(message.contains("| body: {\"ok\": "), "{message}");
        assert!(
            !message.contains("first JSON value ends at byte"),
            "{message}"
        );
        assert!(message.len() < 1024, "预览必须有界: {} 字节", message.len());
    }

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
    let raw: ChatCompletionsEmbedResponse = serde_json::from_str(response).map_err(|e| {
        LlmConnectorError::ParseError(format!(
            "{}: {} | body: {}",
            provider_name,
            e,
            body_diagnostic(response)
        ))
    })?;

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
