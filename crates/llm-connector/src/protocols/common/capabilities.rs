#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderFamily {
    OpenAI,
    OpenAICompatible,
    Anthropic,
    Aliyun,
    Zhipu,
    Ollama,
    Google,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthKind {
    None,
    Bearer,
    ApiKeyHeader,
    Signature,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ContentBlockMode {
    Standard,
    TextOnly,
    NativeMessage,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamingProtocolKind {
    None,
    SseOpenAI,
    SseJsonEvent,
    NdJson,
    Custom,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ReasoningRequestStrategy {
    Unsupported,
    ReasoningEffort,
    EnableThinking,
    ThinkingBudget,
    EnableThinkingWithBudget,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamReasoningStrategy {
    None,
    SeparateField,
    EmbeddedThinkTags,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmptyAssistantToolContentStrategy {
    Null,
    EmptyString,
    EmptyArray,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProviderCapabilities {
    pub family: ProviderFamily,
    pub auth_kind: AuthKind,
    pub content_block_mode: ContentBlockMode,
    pub streaming_protocol: StreamingProtocolKind,
    pub supports_chat: bool,
    pub supports_streaming: bool,
    pub supports_embeddings: bool,
    pub supports_responses_api: bool,
    pub supports_tools: bool,
    pub supports_tool_choice: bool,
    pub supports_response_format: bool,
    pub reasoning_request_strategy: ReasoningRequestStrategy,
    pub stream_reasoning_strategy: StreamReasoningStrategy,
    pub empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy,
    pub supports_multimodal_input: bool,
    pub requires_region_routing: bool,
    pub region_key_scope_sensitive: bool,
}

impl ProviderCapabilities {
    pub const fn supports_reasoning_flag(&self) -> bool {
        matches!(
            self.reasoning_request_strategy,
            ReasoningRequestStrategy::EnableThinking
                | ReasoningRequestStrategy::EnableThinkingWithBudget
        )
    }

    pub const fn supports_reasoning_budget(&self) -> bool {
        matches!(
            self.reasoning_request_strategy,
            ReasoningRequestStrategy::ThinkingBudget
                | ReasoningRequestStrategy::EnableThinkingWithBudget
        )
    }

    pub const fn supports_reasoning_effort(&self) -> bool {
        matches!(
            self.reasoning_request_strategy,
            ReasoningRequestStrategy::ReasoningEffort
        )
    }

    pub const fn openai() -> Self {
        Self {
            family: ProviderFamily::OpenAI,
            auth_kind: AuthKind::Bearer,
            content_block_mode: ContentBlockMode::Standard,
            streaming_protocol: StreamingProtocolKind::SseOpenAI,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: true,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: true,
            reasoning_request_strategy: ReasoningRequestStrategy::ReasoningEffort,
            stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::Null,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn ollama() -> Self {
        Self {
            family: ProviderFamily::Ollama,
            auth_kind: AuthKind::None,
            content_block_mode: ContentBlockMode::NativeMessage,
            streaming_protocol: StreamingProtocolKind::NdJson,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: false,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::Unsupported,
            stream_reasoning_strategy: StreamReasoningStrategy::None,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn openai_compatible_text_only() -> Self {
        Self {
            family: ProviderFamily::OpenAICompatible,
            auth_kind: AuthKind::Bearer,
            content_block_mode: ContentBlockMode::TextOnly,
            streaming_protocol: StreamingProtocolKind::SseOpenAI,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: true,
            reasoning_request_strategy: ReasoningRequestStrategy::ReasoningEffort,
            stream_reasoning_strategy: StreamReasoningStrategy::EmbeddedThinkTags,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyString,
            supports_multimodal_input: false,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn zhipu_openai_compatible() -> Self {
        Self {
            family: ProviderFamily::Zhipu,
            auth_kind: AuthKind::Bearer,
            content_block_mode: ContentBlockMode::Standard,
            streaming_protocol: StreamingProtocolKind::SseOpenAI,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: false,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
            stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::Null,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn aliyun() -> Self {
        Self {
            family: ProviderFamily::Aliyun,
            auth_kind: AuthKind::Bearer,
            content_block_mode: ContentBlockMode::Standard,
            streaming_protocol: StreamingProtocolKind::SseJsonEvent,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
            stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: false,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn anthropic() -> Self {
        Self {
            family: ProviderFamily::Anthropic,
            auth_kind: AuthKind::ApiKeyHeader,
            content_block_mode: ContentBlockMode::NativeMessage,
            streaming_protocol: StreamingProtocolKind::SseJsonEvent,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: false,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::ThinkingBudget,
            stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn google() -> Self {
        Self {
            family: ProviderFamily::Google,
            auth_kind: AuthKind::ApiKeyHeader,
            content_block_mode: ContentBlockMode::NativeMessage,
            streaming_protocol: StreamingProtocolKind::SseJsonEvent,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
            stream_reasoning_strategy: StreamReasoningStrategy::SeparateField,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }

    pub const fn tencent() -> Self {
        Self {
            family: ProviderFamily::Custom,
            auth_kind: AuthKind::Signature,
            content_block_mode: ContentBlockMode::TextOnly,
            streaming_protocol: StreamingProtocolKind::SseJsonEvent,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: false,
            supports_responses_api: false,
            supports_tools: false,
            supports_tool_choice: false,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
            stream_reasoning_strategy: StreamReasoningStrategy::None,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: false,
            requires_region_routing: true,
            region_key_scope_sensitive: true,
        }
    }

    pub const fn openrouter() -> Self {
        Self {
            family: ProviderFamily::Custom,
            auth_kind: AuthKind::Signature,
            content_block_mode: ContentBlockMode::Standard,
            streaming_protocol: StreamingProtocolKind::SseOpenAI,
            supports_chat: true,
            supports_streaming: true,
            supports_embeddings: true,
            supports_responses_api: false,
            supports_tools: true,
            supports_tool_choice: true,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::EnableThinking,
            stream_reasoning_strategy: StreamReasoningStrategy::None,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::Null,
            supports_multimodal_input: true,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }
}

impl Default for ProviderCapabilities {
    fn default() -> Self {
        Self {
            family: ProviderFamily::Custom,
            auth_kind: AuthKind::Custom,
            content_block_mode: ContentBlockMode::Standard,
            streaming_protocol: StreamingProtocolKind::Custom,
            supports_chat: true,
            supports_streaming: false,
            supports_embeddings: false,
            supports_responses_api: false,
            supports_tools: false,
            supports_tool_choice: false,
            supports_response_format: false,
            reasoning_request_strategy: ReasoningRequestStrategy::Unsupported,
            stream_reasoning_strategy: StreamReasoningStrategy::None,
            empty_assistant_tool_content_strategy: EmptyAssistantToolContentStrategy::EmptyArray,
            supports_multimodal_input: false,
            requires_region_routing: false,
            region_key_scope_sensitive: false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EmptyAssistantToolContentStrategy, ProviderCapabilities, ReasoningRequestStrategy,
        StreamReasoningStrategy,
    };

    #[test]
    fn test_openai_capabilities_use_reasoning_effort_and_separate_stream_reasoning() {
        let capabilities = ProviderCapabilities::openai();

        assert_eq!(
            capabilities.reasoning_request_strategy,
            ReasoningRequestStrategy::ReasoningEffort
        );
        assert_eq!(
            capabilities.stream_reasoning_strategy,
            StreamReasoningStrategy::SeparateField
        );
        assert!(capabilities.supports_reasoning_effort());
        assert!(capabilities.supports_tool_choice);
        assert_eq!(
            capabilities.empty_assistant_tool_content_strategy,
            EmptyAssistantToolContentStrategy::Null
        );
    }

    #[test]
    fn test_anthropic_capabilities_use_budget_strategy() {
        let capabilities = ProviderCapabilities::anthropic();

        assert_eq!(
            capabilities.reasoning_request_strategy,
            ReasoningRequestStrategy::ThinkingBudget
        );
        assert!(capabilities.supports_reasoning_budget());
        assert!(!capabilities.supports_reasoning_flag());
        assert!(capabilities.supports_tool_choice);
    }

    #[test]
    fn test_openai_compatible_text_only_capabilities_use_embedded_think_tags() {
        let capabilities = ProviderCapabilities::openai_compatible_text_only();

        assert_eq!(
            capabilities.stream_reasoning_strategy,
            StreamReasoningStrategy::EmbeddedThinkTags
        );
        assert_eq!(
            capabilities.reasoning_request_strategy,
            ReasoningRequestStrategy::ReasoningEffort
        );
        assert_eq!(
            capabilities.empty_assistant_tool_content_strategy,
            EmptyAssistantToolContentStrategy::EmptyString
        );
    }
}
