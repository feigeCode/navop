pub mod chat_history;
pub mod connector;
pub mod manager;
pub mod notifier;
pub mod onet_cli_provider;
pub mod storage;
pub mod types;

pub use connector::{ChatStream, LlmConnector, LlmProvider};
pub use manager::{GlobalProviderState, ProviderManager};
pub use onet_cli_provider::OnetCliLLMProvider;
pub use types::{ProviderConfig, ProviderType};

pub use llm_connector::types::{ChatRequest, Message, MessageBlock, Role, StreamingResponse};

use gpui::App;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct StreamTextParts<'a> {
    pub content: Option<&'a str>,
    pub reasoning: Option<&'a str>,
}

/// 提取流式响应中当前可展示的文本。
///
/// 优先使用 provider 返回的正文内容；若正文为空，则回退到 reasoning/thinking，
/// 以兼容 Ollama 下 Qwen 等只在 `thinking` 字段返回内容的模型。
pub fn extract_stream_text(response: &StreamingResponse) -> Option<&str> {
    response.get_content().or_else(|| {
        response
            .choices
            .iter()
            .find_map(|choice| choice.delta.reasoning_any().filter(|text| !text.is_empty()))
    })
}

/// 将流式响应中的正文和 reasoning/thinking 分开提取。
pub fn extract_stream_text_parts(response: &StreamingResponse) -> StreamTextParts<'_> {
    let reasoning = response
        .choices
        .iter()
        .find_map(|choice| choice.delta.reasoning_any().filter(|text| !text.is_empty()))
        .or(response
            .reasoning_content
            .as_deref()
            .filter(|text| !text.is_empty()));

    let content = response
        .choices
        .iter()
        .find_map(|choice| {
            choice
                .delta
                .content
                .as_deref()
                .filter(|text| !text.is_empty())
        })
        .or_else(|| {
            response
                .get_content()
                .filter(|text| reasoning != Some(*text))
        });

    StreamTextParts { content, reasoning }
}

pub fn init(cx: &mut App) {
    storage::init(cx);
    notifier::init(cx);
    let state = GlobalProviderState::new();
    cx.set_global(state);
}

#[cfg(test)]
mod tests {
    use super::{extract_stream_text, extract_stream_text_parts};
    use llm_connector::types::{Delta, StreamingChoice, StreamingResponse};

    #[test]
    fn extract_stream_text_prefers_content() {
        let response = StreamingResponse {
            content: "可见正文".to_string(),
            choices: vec![StreamingChoice {
                index: 0,
                delta: Delta {
                    content: Some("可见正文".to_string()),
                    thinking: Some("内部思考".to_string()),
                    ..Default::default()
                },
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        };

        assert_eq!(extract_stream_text(&response), Some("可见正文"));
    }

    #[test]
    fn extract_stream_text_falls_back_to_reasoning() {
        let response = StreamingResponse {
            choices: vec![StreamingChoice {
                index: 0,
                delta: Delta {
                    thinking: Some("推理内容".to_string()),
                    ..Default::default()
                },
                finish_reason: Some("length".to_string()),
                logprobs: None,
            }],
            ..Default::default()
        };

        assert_eq!(extract_stream_text(&response), Some("推理内容"));
    }

    #[test]
    fn extract_stream_text_parts_keeps_reasoning_separate() {
        let response = StreamingResponse {
            content: "正式回复".to_string(),
            choices: vec![StreamingChoice {
                index: 0,
                delta: Delta {
                    content: Some("正式回复".to_string()),
                    thinking: Some("内部思考".to_string()),
                    ..Default::default()
                },
                finish_reason: None,
                logprobs: None,
            }],
            ..Default::default()
        };

        let parts = extract_stream_text_parts(&response);

        assert_eq!(parts.content, Some("正式回复"));
        assert_eq!(parts.reasoning, Some("内部思考"));
    }

    #[test]
    fn extract_stream_text_parts_does_not_promote_reasoning_to_content() {
        let response = StreamingResponse {
            choices: vec![StreamingChoice {
                index: 0,
                delta: Delta {
                    thinking: Some("推理内容".to_string()),
                    ..Default::default()
                },
                finish_reason: Some("length".to_string()),
                logprobs: None,
            }],
            ..Default::default()
        };

        let parts = extract_stream_text_parts(&response);

        assert_eq!(parts.content, None);
        assert_eq!(parts.reasoning, Some("推理内容"));
    }
}
