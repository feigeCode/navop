//! Server-Sent Events (SSE) streaming utilities
//!
//! This module provides robust streaming utilities for handling various LLM provider response formats.
//! It supports:
//! - Standard SSE (Server-Sent Events) with double-newline separators
//! - Non-standard SSE with single-newline separators (e.g. Zhipu)
//! - NDJSON (Newline Delimited JSON) (e.g. Ollama)
//! - Automatic format detection

use {
    crate::error::LlmConnectorError,
    futures_util::{Stream, StreamExt},
    serde_json::Value,
    std::pin::Pin,
};

/// Stream format type
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamFormat {
    /// Standard SSE (double newline separator)
    Sse,
    /// Line-delimited JSON (single newline separator)
    NdJson,
    /// Auto-detect based on content
    Auto,
}

/// Protocol-aware parsing mode for stream chunk payloads.
#[cfg(feature = "streaming")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamingParseMode {
    /// Try OpenAI shape first, then strict Ollama fallback.
    Auto,
    /// Only accept OpenAI-compatible streaming chunks.
    OpenAIOnly,
    /// Allow strict Ollama chunk fallback after OpenAI parse fails.
    OllamaStrict,
}

#[cfg(feature = "streaming")]
struct StreamNormalizationState {
    accumulated_tool_calls: std::collections::HashMap<usize, crate::types::ToolCall>,
    think_buffer: String,
    in_think_block: bool,
}

/// Create a robust stream from a reqwest response
///
/// This function automatically handles different streaming formats and normalizes them
/// into a stream of JSON strings.
pub fn create_text_stream(
    response: reqwest::Response,
    format: StreamFormat,
) -> Pin<Box<dyn Stream<Item = Result<String, LlmConnectorError>> + Send>> {
    let stream = response.bytes_stream();

    // Use a scanning state to handle partial chunks and format detection
    struct ScanState {
        buffer: String,
        detected_format: Option<StreamFormat>,
    }

    let events_stream = stream
        .scan(
            ScanState {
                buffer: String::new(),
                detected_format: if format == StreamFormat::Auto {
                    None
                } else {
                    Some(format)
                },
            },
            move |state, chunk_result| {
                let mut out: Vec<Result<String, LlmConnectorError>> = Vec::new();
                match chunk_result {
                    Ok(chunk) => {
                        // Normalize line endings
                        let chunk_str = String::from_utf8_lossy(&chunk).replace("\r\n", "\n");
                        state.buffer.push_str(&chunk_str);

                        // Auto-detect format if not yet detected
                        if state.detected_format.is_none() {
                            if state.buffer.contains("data:") {
                                state.detected_format = Some(StreamFormat::Sse);
                            } else if state.buffer.contains('\n')
                                && state.buffer.trim().starts_with('{')
                            {
                                state.detected_format = Some(StreamFormat::NdJson);
                            }
                        }

                        match state.detected_format {
                            Some(StreamFormat::Sse) => {
                                // SSE processing (split by \n\n)
                                // Handle edge case where \n\n might be split across chunks
                                while let Some(boundary_idx) = state.buffer.find("\n\n") {
                                    let event_str: String =
                                        state.buffer.drain(..boundary_idx + 2).collect();

                                    // Extract data lines
                                    let mut data_lines = Vec::new();
                                    for line in event_str.split('\n') {
                                        let line = line.trim();
                                        if let Some(payload) = line.strip_prefix("data:") {
                                            let payload = payload.trim();
                                            if !payload.is_empty() && payload != "[DONE]" {
                                                data_lines.push(payload.to_string());
                                            }
                                        }
                                    }

                                    if !data_lines.is_empty() {
                                        out.push(Ok(data_lines.join("\n")));
                                    }
                                }
                            }
                            Some(StreamFormat::NdJson) => {
                                // NDJSON processing (split by \n)
                                while let Some(boundary_idx) = state.buffer.find('\n') {
                                    let line: String =
                                        state.buffer.drain(..boundary_idx + 1).collect();
                                    let trimmed = line.trim();

                                    // Handle "data:" prefix if present (Zhipu style)
                                    let payload = if let Some(p) = trimmed.strip_prefix("data:") {
                                        p.trim()
                                    } else {
                                        trimmed
                                    };

                                    if !payload.is_empty() && payload != "[DONE]" {
                                        out.push(Ok(payload.to_string()));
                                    }
                                }
                            }
                            None => {
                                // Not enough data to detect format yet, wait for more
                            }
                            _ => {
                                // Should not happen
                            }
                        }
                    }
                    Err(e) => {
                        out.push(Err(LlmConnectorError::NetworkError(e.to_string())));
                    }
                }
                std::future::ready(Some(out))
            },
        )
        .flat_map(futures_util::stream::iter);

    Box::pin(events_stream)
}

/// Legacy SSE events parser (kept for backward compatibility)
#[inline]
pub fn sse_events(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<String, LlmConnectorError>> + Send>> {
    create_text_stream(response, StreamFormat::Sse)
}

/// Legacy JSON lines events parser (kept for backward compatibility)
#[inline]
pub fn json_lines_events(
    response: reqwest::Response,
) -> Pin<Box<dyn Stream<Item = Result<String, LlmConnectorError>> + Send>> {
    create_text_stream(response, StreamFormat::NdJson)
}

/// Parse a single SSE line and extract the JSON payload
///
/// # Returns
/// - `Ok(Some(Value))` if line contains valid JSON data
/// - `Ok(None)` if line is empty, comment, or "[DONE]"
/// - `Err` if line contains invalid JSON
pub fn parse_sse_line(line: &str) -> Result<Option<Value>, LlmConnectorError> {
    let line = line.trim();
    if line.is_empty() || line.starts_with(':') {
        return Ok(None);
    }

    if let Some(payload) = line.strip_prefix("data:") {
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" {
            return Ok(None);
        }

        let value: Value = serde_json::from_str(payload).map_err(|e| {
            LlmConnectorError::ParseError(format!("Failed to parse SSE JSON: {}", e))
        })?;
        Ok(Some(value))
    } else {
        Ok(None)
    }
}

/// Convert HTTP response to StreamingResponse stream with automatic format detection
#[cfg(feature = "streaming")]
pub fn sse_to_streaming_response(response: reqwest::Response) -> crate::types::ChatStream {
    sse_to_streaming_response_with_mode(response, StreamingParseMode::Auto)
}

/// Convert HTTP response to StreamingResponse stream with protocol-aware parsing mode.
#[cfg(feature = "streaming")]
pub fn sse_to_streaming_response_with_mode(
    response: reqwest::Response,
    parse_mode: StreamingParseMode,
) -> crate::types::ChatStream {
    sse_to_streaming_response_with_options(
        response,
        parse_mode,
        crate::protocols::common::capabilities::StreamReasoningStrategy::EmbeddedThinkTags,
    )
}

/// Convert HTTP response to StreamingResponse stream with protocol-aware parsing mode
/// and optional reasoning tag normalization.
#[cfg(feature = "streaming")]
pub fn sse_to_streaming_response_with_options(
    response: reqwest::Response,
    parse_mode: StreamingParseMode,
    stream_reasoning_strategy: crate::protocols::common::capabilities::StreamReasoningStrategy,
) -> crate::types::ChatStream {
    // Use Auto detection by default
    let string_stream = create_text_stream(response, StreamFormat::Auto);

    // State for accumulating tool_calls and normalizing think blocks across chunks
    let response_stream = string_stream.scan(
        StreamNormalizationState {
            accumulated_tool_calls: std::collections::HashMap::new(),
            think_buffer: String::new(),
            in_think_block: false,
        },
        move |state, result| {
            let processed = result.and_then(|json_str| {
                let mut streaming_response = parse_streaming_payload(&json_str, parse_mode)?;

                if stream_reasoning_strategy
                    == crate::protocols::common::capabilities::StreamReasoningStrategy::EmbeddedThinkTags
                {
                    normalize_think_tags_across_stream(&mut streaming_response, state);
                }

                // Populate convenience fields
                populate_convenience_fields(&mut streaming_response);

                // Accumulate tool calls
                accumulate_tool_calls(&mut streaming_response, &mut state.accumulated_tool_calls);

                Ok(streaming_response)
            });

            std::future::ready(Some(processed))
        },
    );

    Box::pin(response_stream)
}

#[cfg(feature = "streaming")]
fn parse_streaming_payload(
    json_str: &str,
    parse_mode: StreamingParseMode,
) -> Result<crate::types::StreamingResponse, crate::error::LlmConnectorError> {
    use crate::types::StreamingResponse;

    // First try OpenAI-compatible chunk format.
    if let Ok(mut response) = serde_json::from_str::<StreamingResponse>(json_str) {
        if let Ok(raw) = serde_json::from_str::<Value>(json_str) {
            response.populate_reasoning_synonyms(&raw);
        }
        normalize_openai_compatible_streaming_choices(&mut response);
        return Ok(response);
    }

    // If caller only accepts OpenAI-style chunks, fail fast.
    if parse_mode == StreamingParseMode::OpenAIOnly {
        return Err(crate::error::LlmConnectorError::ParseError(format!(
            "Failed to parse streaming response as OpenAI-compatible chunk. Content: {}",
            json_str
        )));
    }

    // Fallback for Ollama /api/chat NDJSON chunk format.
    let raw: Value = serde_json::from_str(json_str).map_err(|e| {
        crate::error::LlmConnectorError::ParseError(format!(
            "Failed to parse streaming response: {}. Content: {}",
            e, json_str
        ))
    })?;

    if let Some(response) = parse_ollama_chunk(&raw, parse_mode) {
        return Ok(response);
    }

    Err(crate::error::LlmConnectorError::ParseError(format!(
        "Failed to parse streaming response: unsupported chunk format. Content: {}",
        json_str
    )))
}

#[cfg(feature = "streaming")]
fn parse_ollama_chunk(
    raw: &Value,
    parse_mode: StreamingParseMode,
) -> Option<crate::types::StreamingResponse> {
    use crate::types::{
        Delta, FunctionCall, Role, StreamingChoice, StreamingResponse, ToolCall, Usage,
    };

    if parse_mode == StreamingParseMode::OpenAIOnly || !is_strict_ollama_chunk(raw) {
        return None;
    }

    let model = raw.get("model")?.as_str()?.to_string();
    let message = raw.get("message")?.as_object()?;

    let role = message
        .get("role")
        .and_then(|v| v.as_str())
        .and_then(|r| match r {
            "system" => Some(Role::System),
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            "tool" => Some(Role::Tool),
            _ => None,
        });

    let content = message
        .get("content")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let tool_calls = message.get("tool_calls").and_then(|i| {
        i.as_array().map(|i| {
            i.iter()
                .map(|i| {
                    let f = i.get("function");
                    ToolCall {
                        call_type: "function".into(),
                        id: i
                            .get("id")
                            .and_then(|i| i.as_str())
                            .unwrap_or_default()
                            .into(),
                        index: f
                            .and_then(|i| i.get("index"))
                            .and_then(|i| i.as_u64())
                            .map(|i| i as _),
                        function: f
                            .and_then(|i| {
                                if let Some(name) = i.get("name")
                                    && let Some(arguments) = i.get("arguments")
                                {
                                    Some(FunctionCall {
                                        name: name.as_str().unwrap_or_default().into(),
                                        arguments: arguments.to_string(),
                                        ..Default::default()
                                    })
                                } else {
                                    None
                                }
                            })
                            .unwrap_or_default(),
                        ..Default::default()
                    }
                })
                .collect()
        })
    });

    let delta = Delta {
        role,
        content: if content.is_empty() {
            None
        } else {
            Some(content.clone())
        },
        tool_calls,
        reasoning_content: message
            .get("reasoning_content")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        reasoning: message
            .get("reasoning")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        thought: message
            .get("thought")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        thinking: message
            .get("thinking")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
        thinking_signature: message
            .get("thinking_signature")
            .and_then(|v| v.as_str())
            .map(ToString::to_string),
    };

    let done = raw.get("done").and_then(|v| v.as_bool()).unwrap_or(false);
    let finish_reason = if done {
        Some(
            raw.get("done_reason")
                .and_then(|v| v.as_str())
                .unwrap_or("stop")
                .to_string(),
        )
    } else {
        None
    };

    let usage = if done {
        let prompt_tokens = raw
            .get("prompt_eval_count")
            .and_then(|v| v.as_u64())
            .unwrap_or(0) as u32;
        let completion_tokens = raw.get("eval_count").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
        if prompt_tokens > 0 || completion_tokens > 0 {
            Some(Usage {
                prompt_tokens,
                completion_tokens,
                total_tokens: prompt_tokens + completion_tokens,
                ..Default::default()
            })
        } else {
            None
        }
    } else {
        None
    };

    let created = raw
        .get("created_at")
        .and_then(|v| v.as_str())
        .and_then(|s| chrono::DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.timestamp() as u64)
        .unwrap_or_else(|| chrono::Utc::now().timestamp() as u64);

    let mut response = StreamingResponse {
        id: format!("ollama-{}", created),
        object: "chat.completion.chunk".to_string(),
        created,
        model,
        choices: vec![StreamingChoice {
            index: 0,
            delta,
            finish_reason,
            logprobs: None,
        }],
        content,
        reasoning_content: None,
        usage,
        system_fingerprint: None,
    };

    response.populate_reasoning_synonyms(raw);
    Some(response)
}

#[cfg(feature = "streaming")]
fn is_strict_ollama_chunk(raw: &Value) -> bool {
    let message = match raw.get("message").and_then(|v| v.as_object()) {
        Some(m) => m,
        None => return false,
    };

    if raw.get("model").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    if raw.get("done").and_then(|v| v.as_bool()).is_none() {
        return false;
    }
    if message.get("role").and_then(|v| v.as_str()).is_none() {
        return false;
    }
    if !message
        .get("content")
        .map(|v| v.is_string())
        .unwrap_or(false)
    {
        return false;
    }

    // Require at least one Ollama-specific marker to avoid accidental misclassification.
    raw.get("created_at").and_then(|v| v.as_str()).is_some()
        || raw.get("done_reason").and_then(|v| v.as_str()).is_some()
        || raw
            .get("prompt_eval_count")
            .and_then(|v| v.as_u64())
            .is_some()
        || raw.get("eval_count").and_then(|v| v.as_u64()).is_some()
        || raw.get("total_duration").and_then(|v| v.as_u64()).is_some()
        || raw.get("remote_model").and_then(|v| v.as_str()).is_some()
        || raw.get("remote_host").and_then(|v| v.as_str()).is_some()
}

#[cfg(feature = "streaming")]
fn populate_convenience_fields(response: &mut crate::types::StreamingResponse) {
    if response.content.is_empty()
        && let Some(choice) = response.choices.first()
    {
        let content_to_use = choice.delta.content.as_ref().filter(|s| !s.is_empty());

        if let Some(content) = content_to_use {
            response.content = content.clone();
        }
    }
}

#[cfg(feature = "streaming")]
fn normalize_openai_compatible_streaming_choices(response: &mut crate::types::StreamingResponse) {
    for choice in &mut response.choices {
        let normalized =
            crate::protocols::common::openai_compatible::normalize_openai_compatible_content(
                choice.delta.content.take(),
                choice.delta.reasoning_content.take(),
                crate::protocols::common::capabilities::StreamReasoningStrategy::EmbeddedThinkTags,
            );

        if !normalized.content.is_empty() {
            choice.delta.content = Some(normalized.content);
        }

        if normalized.reasoning.is_some() {
            choice.delta.reasoning_content = normalized.reasoning;
        }
    }

    if response.reasoning_content.is_none()
        && let Some(reasoning) = response
            .choices
            .iter()
            .find_map(|choice| choice.delta.reasoning_content.clone())
    {
        response.reasoning_content = Some(reasoning);
    }
}

#[cfg(feature = "streaming")]
fn split_think_segments_incremental(
    incoming: &str,
    carry: &mut String,
    in_think_block: &mut bool,
) -> (String, Option<String>) {
    const THINK_OPEN: &str = "<think>";
    const THINK_CLOSE: &str = "</think>";

    fn longest_suffix_prefix_len(haystack: &str, needle: &str) -> usize {
        let max_len = haystack.len().min(needle.len().saturating_sub(1));
        (1..=max_len)
            .rev()
            .find(|&len| haystack.ends_with(&needle[..len]))
            .unwrap_or(0)
    }

    carry.push_str(incoming);

    let mut visible = String::new();
    let mut reasoning = String::new();

    loop {
        if *in_think_block {
            if let Some(idx) = carry.find(THINK_CLOSE) {
                reasoning.push_str(&carry[..idx]);
                carry.drain(..idx + THINK_CLOSE.len());
                *in_think_block = false;
                continue;
            }

            let keep = longest_suffix_prefix_len(carry, THINK_CLOSE);
            if carry.len() > keep {
                let emit_len = carry.len() - keep;
                reasoning.push_str(&carry[..emit_len]);
                carry.drain(..emit_len);
            }
            break;
        }

        if let Some(idx) = carry.find(THINK_OPEN) {
            visible.push_str(&carry[..idx]);
            carry.drain(..idx + THINK_OPEN.len());
            *in_think_block = true;
            continue;
        }

        let keep = longest_suffix_prefix_len(carry, THINK_OPEN);
        if carry.len() > keep {
            let emit_len = carry.len() - keep;
            visible.push_str(&carry[..emit_len]);
            carry.drain(..emit_len);
        }
        break;
    }

    let reasoning = if reasoning.is_empty() {
        None
    } else {
        Some(reasoning)
    };

    (visible, reasoning)
}

#[cfg(feature = "streaming")]
fn normalize_think_tags_across_stream(
    response: &mut crate::types::StreamingResponse,
    state: &mut StreamNormalizationState,
) {
    for choice in &mut response.choices {
        if let Some(content) = choice.delta.content.take() {
            let (visible, reasoning) = split_think_segments_incremental(
                &content,
                &mut state.think_buffer,
                &mut state.in_think_block,
            );

            if !visible.is_empty() {
                choice.delta.content = Some(visible);
            }

            if let Some(reasoning_piece) = reasoning {
                let merged = match choice.delta.reasoning_content.take() {
                    Some(existing) if !existing.is_empty() => existing + &reasoning_piece,
                    _ => reasoning_piece,
                };
                choice.delta.reasoning_content = Some(merged);
            }
        }
    }

    if response.reasoning_content.is_none()
        && let Some(reasoning) = response
            .choices
            .iter()
            .find_map(|choice| choice.delta.reasoning_content.clone())
    {
        response.reasoning_content = Some(reasoning);
    }
}

#[cfg(feature = "streaming")]
fn accumulate_tool_calls(
    response: &mut crate::types::StreamingResponse,
    accumulated: &mut std::collections::HashMap<usize, crate::types::ToolCall>,
) {
    if let Some(choice) = response.choices.first_mut()
        && let Some(delta_tool_calls) = &choice.delta.tool_calls
    {
        for delta_call in delta_tool_calls {
            let index = delta_call.index.unwrap_or(0);

            accumulated
                .entry(index)
                .and_modify(|existing| existing.merge_delta(delta_call))
                .or_insert_with(|| delta_call.clone());
        }

        let complete_calls: Vec<crate::types::ToolCall> = accumulated
            .values()
            .filter(|call| call.is_complete())
            .cloned()
            .collect();

        if !complete_calls.is_empty() {
            choice.delta.tool_calls = Some(complete_calls);
        } else {
            choice.delta.tool_calls = None;
        }
    }
}

#[cfg(test)]
mod tests {
    #[cfg(feature = "streaming")]
    #[test]
    fn test_parse_ollama_chunk_with_thinking() {
        let chunk = r#"{"model":"kimi-k2.5:cloud","created_at":"2026-03-05T08:32:36.674615034Z","message":{"role":"assistant","content":"","thinking":"step-by-step"},"done":false}"#;

        let parsed = super::parse_streaming_payload(chunk, super::StreamingParseMode::OllamaStrict)
            .expect("should parse ollama chunk");
        assert_eq!(parsed.model, "kimi-k2.5:cloud");
        assert_eq!(parsed.choices.len(), 1);
        assert_eq!(
            parsed.choices[0].delta.thinking.as_deref(),
            Some("step-by-step")
        );
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_parse_ollama_done_chunk_with_usage() {
        let chunk = r#"{"model":"kimi-k2.5:cloud","created_at":"2026-03-05T08:32:36.674615034Z","message":{"role":"assistant","content":"done"},"done":true,"done_reason":"stop","prompt_eval_count":10,"eval_count":20}"#;

        let parsed = super::parse_streaming_payload(chunk, super::StreamingParseMode::OllamaStrict)
            .expect("should parse ollama done chunk");
        assert_eq!(parsed.choices[0].finish_reason.as_deref(), Some("stop"));
        assert_eq!(parsed.usage.as_ref().map(|u| u.total_tokens), Some(30));
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_openai_only_mode_rejects_ollama_chunk() {
        let chunk = r#"{"model":"kimi-k2.5:cloud","created_at":"2026-03-05T08:32:36.674615034Z","message":{"role":"assistant","content":""},"done":false}"#;

        let result = super::parse_streaming_payload(chunk, super::StreamingParseMode::OpenAIOnly);
        assert!(result.is_err());
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_reasoning_chunk_does_not_fill_convenience_content() {
        let chunk = r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1740000000,"model":"glm-4.5-flash","choices":[{"index":0,"delta":{"reasoning_content":"internal reasoning only"},"finish_reason":null}]}"#;

        let mut parsed =
            super::parse_streaming_payload(chunk, super::StreamingParseMode::OpenAIOnly)
                .expect("should parse openai-compatible reasoning chunk");

        super::populate_convenience_fields(&mut parsed);

        assert_eq!(parsed.content, "");
        assert_eq!(
            parsed.choices[0].delta.reasoning_content.as_deref(),
            Some("internal reasoning only")
        );
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_think_tags_are_stripped_from_streaming_content() {
        let chunk = r#"{"id":"chatcmpl-test","object":"chat.completion.chunk","created":1740000000,"model":"MiniMax-M2.5","choices":[{"index":0,"delta":{"content":"<think>internal reasoning</think>Visible answer"},"finish_reason":null}]}"#;

        let mut parsed =
            super::parse_streaming_payload(chunk, super::StreamingParseMode::OpenAIOnly)
                .expect("should parse minimax-like chunk");

        super::populate_convenience_fields(&mut parsed);

        assert_eq!(parsed.content, "Visible answer");
        assert_eq!(
            parsed.choices[0].delta.content.as_deref(),
            Some("Visible answer")
        );
        assert_eq!(
            parsed.choices[0].delta.reasoning_content.as_deref(),
            Some("internal reasoning")
        );
    }

    #[cfg(feature = "streaming")]
    #[test]
    fn test_think_tags_are_not_stripped_when_normalization_disabled() {
        let mut response = crate::types::StreamingResponse {
            id: "chatcmpl-test".to_string(),
            object: "chat.completion.chunk".to_string(),
            created: 1740000000,
            model: "glm-4.5-flash".to_string(),
            choices: vec![crate::types::StreamingChoice {
                index: 0,
                delta: crate::types::Delta {
                    role: None,
                    content: Some("<think>internal reasoning</think>Visible answer".to_string()),
                    tool_calls: None,
                    reasoning_content: None,
                    reasoning: None,
                    thought: None,
                    thinking: None,
                    thinking_signature: None,
                },
                finish_reason: None,
                logprobs: None,
            }],
            content: String::new(),
            reasoning_content: None,
            usage: None,
            system_fingerprint: None,
        };

        super::populate_convenience_fields(&mut response);

        assert_eq!(
            response.content,
            "<think>internal reasoning</think>Visible answer"
        );
        assert_eq!(
            response.choices[0].delta.content.as_deref(),
            Some("<think>internal reasoning</think>Visible answer")
        );
        assert_eq!(response.choices[0].delta.reasoning_content.as_deref(), None);
    }

    #[tokio::test]
    async fn test_sse_detection() {
        // Mock SSE response
        let _mock_response = "data: {\"test\":1}\n\ndata: {\"test\":2}\n\n";
        // In a real test we would need to mock reqwest::Response, but since we can't easily construct one,
        // we'll verify the logic in CreateTextStream via integration tests or by exposing the internal scanner.
    }
}
