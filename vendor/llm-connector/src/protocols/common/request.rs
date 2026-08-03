//! Common Request Assembly Logic

use crate::error::LlmConnectorError;
use crate::protocols::common::capabilities::EmptyAssistantToolContentStrategy;
use crate::types::{
    DocumentSource, ImageSource, Message, MessageBlock, Role,
};

fn serialize_empty_assistant_tool_content(
    strategy: EmptyAssistantToolContentStrategy,
) -> serde_json::Value {
    match strategy {
        EmptyAssistantToolContentStrategy::Null => serde_json::Value::Null,
        EmptyAssistantToolContentStrategy::EmptyString => serde_json::json!(""),
        EmptyAssistantToolContentStrategy::EmptyArray => serde_json::json!([]),
    }
}

fn should_use_empty_assistant_tool_content_override(msg: &Message) -> bool {
    msg.role == Role::Assistant && msg.tool_calls.is_some() && msg.content.is_empty()
}

fn partition_openai_visible_blocks(content: &[MessageBlock]) -> (Vec<MessageBlock>, String) {
    let mut thinking_agg = String::new();
    let mut visible = Vec::new();
    for block in content {
        if let MessageBlock::Thinking { thinking, .. } = block {
            if !thinking_agg.is_empty() {
                thinking_agg.push('\n');
            }
            thinking_agg.push_str(thinking);
        } else {
            visible.push(block.clone());
        }
    }
    (visible, thinking_agg)
}

fn openai_export_reasoning_content(msg: &Message, thinking_from_blocks: &str) -> Option<String> {
    let mut out = String::new();
    if let Some(s) = msg.reasoning_content.as_deref() {
        if !s.is_empty() {
            out.push_str(s);
        }
    } else {
        for opt in [
            msg.reasoning.as_deref(),
            msg.thought.as_deref(),
            msg.thinking.as_deref(),
        ] {
            if let Some(s) = opt.filter(|t| !t.is_empty()) {
                if !out.is_empty() {
                    out.push('\n');
                }
                out.push_str(s);
            }
        }
    }
    if !thinking_from_blocks.is_empty() {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(thinking_from_blocks);
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Convert a single [`MessageBlock`] to an OpenAI-compatible JSON value.
///
/// - `Text` and `ImageUrl` variants are already in OpenAI format and are
///   serialized directly.
/// - `Image` (Anthropic format) is converted to `{"type":"image_url",
///   "image_url":{"url":"data:...;base64,..."}}`.
/// - `Document` / `DocumentUrl` are not supported by the OpenAI Chat
///   Completions API; they are represented as text placeholders.
/// - `Thinking` blocks must be partitioned out by the caller before calling
///   this function.
fn block_to_openai_value(block: &MessageBlock) -> serde_json::Value {
    match block {
        MessageBlock::Text { .. } | MessageBlock::ImageUrl { .. } => {
            serde_json::to_value(block)
                .expect("MessageBlock serialization is infallible")
        }
        MessageBlock::Image { source } => match source {
            ImageSource::Base64 { media_type, data } => serde_json::json!({
                "type": "image_url",
                "image_url": {
                    "url": format!("data:{};base64,{}", media_type, data)
                }
            }),
            ImageSource::Url { url } => serde_json::json!({
                "type": "image_url",
                "image_url": { "url": url }
            }),
        },
        MessageBlock::Document { source } => match source {
            DocumentSource::Base64 { media_type, data } => serde_json::json!({
                "type": "text",
                "text": format!("[Document: {} (base64, {} chars)]", media_type, data.len())
            }),
        },
        MessageBlock::DocumentUrl { document_url } => serde_json::json!({
            "type": "text",
            "text": format!("[Document: {}]", document_url.url)
        }),
        MessageBlock::Thinking { .. } => {
            // Caller must partition Thinking blocks out before calling this.
            // Return Null as a defensive fallback.
            serde_json::Value::Null
        }
    }
}

/// Convert a slice of [`MessageBlock`] to an OpenAI-compatible content array.
fn blocks_to_openai_content(blocks: &[MessageBlock]) -> Vec<serde_json::Value> {
    blocks.iter().map(block_to_openai_value).collect()
}

/// Generic message converter for OpenAI-compatible protocols
pub fn openai_message_converter(messages: &[Message]) -> Vec<serde_json::Value> {
    openai_message_converter_with_strategy(
        messages,
        EmptyAssistantToolContentStrategy::EmptyArray,
    )
}

pub fn openai_message_converter_with_strategy(
    messages: &[Message],
    empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy,
) -> Vec<serde_json::Value> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };

            let (visible, thinking_from_blocks) = partition_openai_visible_blocks(&msg.content);

            let content = if should_use_empty_assistant_tool_content_override(msg) {
                serialize_empty_assistant_tool_content(empty_assistant_tool_content_strategy)
            } else if visible.len() == 1 && visible[0].is_text() {
                serde_json::json!(visible[0].as_text().unwrap())
            } else {
                serde_json::json!(blocks_to_openai_content(&visible))
            };

            let mut map = serde_json::Map::new();
            map.insert("role".to_string(), serde_json::json!(role));
            map.insert("content".to_string(), content);

            if let Some(ref tc) = msg.tool_calls {
                map.insert("tool_calls".to_string(), serde_json::json!(tc));
            }
            if let Some(ref id) = msg.tool_call_id {
                map.insert("tool_call_id".to_string(), serde_json::json!(id));
            }
            if let Some(ref name) = msg.name {
                map.insert("name".to_string(), serde_json::json!(name));
            }
            if let Some(rc) = openai_export_reasoning_content(msg, &thinking_from_blocks) {
                map.insert("reasoning_content".to_string(), serde_json::json!(rc));
            }

            serde_json::Value::Object(map)
        })
        .collect()
}

/// Downgrade message content for providers that only support text content
pub fn openai_message_converter_downgrade(
    messages: &[Message],
) -> Result<Vec<serde_json::Value>, LlmConnectorError> {
    openai_message_converter_downgrade_with_strategy(
        messages,
        EmptyAssistantToolContentStrategy::EmptyString,
    )
}

pub fn openai_message_converter_downgrade_with_strategy(
    messages: &[Message],
    empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy,
) -> Result<Vec<serde_json::Value>, LlmConnectorError> {
    messages
        .iter()
        .map(|msg| {
            let role = match msg.role {
                Role::User => "user",
                Role::Assistant => "assistant",
                Role::System => "system",
                Role::Tool => "tool",
            };

            let (visible, thinking_from_blocks) = partition_openai_visible_blocks(&msg.content);

            // Downgrade content logic
            let content = if should_use_empty_assistant_tool_content_override(msg) {
                match empty_assistant_tool_content_strategy {
                    EmptyAssistantToolContentStrategy::Null => serde_json::Value::Null,
                    EmptyAssistantToolContentStrategy::EmptyString
                    | EmptyAssistantToolContentStrategy::EmptyArray => serde_json::json!(""),
                }
            } else {
                let content_str = if visible.is_empty() && thinking_from_blocks.is_empty() {
                    String::new()
                } else {
                    let mut text_content = String::new();
                    for block in &visible {
                        if let Some(text) = block.as_text() {
                            text_content.push_str(text);
                        } else {
                            return Err(LlmConnectorError::InvalidRequest(format!(
                                "Provider does not support complex content blocks (found {:?})",
                                block
                            )));
                        }
                    }
                    text_content
                };

                serde_json::json!(content_str)
            };

            let mut map = serde_json::Map::new();
            map.insert("role".to_string(), serde_json::json!(role));
            map.insert("content".to_string(), content);

            if let Some(ref tc) = msg.tool_calls {
                map.insert("tool_calls".to_string(), serde_json::json!(tc));
            }
            if let Some(ref id) = msg.tool_call_id {
                map.insert("tool_call_id".to_string(), serde_json::json!(id));
            }
            if let Some(ref name) = msg.name {
                map.insert("name".to_string(), serde_json::json!(name));
            }
            if let Some(rc) = openai_export_reasoning_content(msg, &thinking_from_blocks) {
                map.insert("reasoning_content".to_string(), serde_json::json!(rc));
            }

            Ok(serde_json::Value::Object(map))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Message, MessageBlock, Role};

    #[test]
    fn openai_converter_moves_thinking_blocks_to_reasoning_content() {
        let messages = vec![Message {
            role: Role::User,
            content: vec![
                MessageBlock::thinking_unsigned("internal"),
                MessageBlock::text("hello"),
            ],
            ..Default::default()
        }];
        let out = openai_message_converter_with_strategy(
            &messages,
            EmptyAssistantToolContentStrategy::Null,
        );
        let m = out[0].as_object().unwrap();
        assert_eq!(m["reasoning_content"], "internal");
        assert_eq!(m["content"], "hello");
    }

    #[test]
    fn block_to_openai_value_text() {
        let block = MessageBlock::text("hello");
        let v = block_to_openai_value(&block);
        assert_eq!(v["type"], "text");
        assert_eq!(v["text"], "hello");
    }

    #[test]
    fn block_to_openai_value_image_base64() {
        let block = MessageBlock::image_base64("image/png", "abc");
        let v = block_to_openai_value(&block);
        assert_eq!(v["type"], "image_url");
        let url = v["image_url"]["url"].as_str().unwrap();
        assert!(url.starts_with("data:image/png;base64,"));
        assert!(url.ends_with("abc"));
    }

    #[test]
    fn block_to_openai_value_image_url() {
        let block = MessageBlock::image_url("https://example.com/x.jpg");
        let v = block_to_openai_value(&block);
        assert_eq!(v["type"], "image_url");
        assert_eq!(v["image_url"]["url"], "https://example.com/x.jpg");
    }

    #[test]
    fn block_to_openai_value_document_base64() {
        let block = MessageBlock::document_base64("application/pdf", "deadbeef");
        let v = block_to_openai_value(&block);
        assert_eq!(v["type"], "text");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("application/pdf"));
    }

    #[test]
    fn block_to_openai_value_document_url() {
        let block = MessageBlock::document_url("https://example.com/a.pdf");
        let v = block_to_openai_value(&block);
        assert_eq!(v["type"], "text");
        let text = v["text"].as_str().unwrap();
        assert!(text.contains("https://example.com/a.pdf"));
    }

    #[test]
    fn converter_produces_openai_format_for_image_block() {
        // Regression: Image (Anthropic format) must be converted to
        // OpenAI image_url format, not serialized as Anthropic "image".
        let messages = vec![Message {
            role: Role::User,
            content: vec![
                MessageBlock::text("Look:"),
                MessageBlock::image_base64("image/jpeg", "dGVzdA=="),
            ],
            ..Default::default()
        }];
        let out = openai_message_converter(&messages);
        let content = out[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 2);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        // Must NOT contain Anthropic "image" type
        let types: Vec<&str> = content.iter().filter_map(|v| v["type"].as_str()).collect();
        assert!(!types.contains(&"image"));
    }
}
