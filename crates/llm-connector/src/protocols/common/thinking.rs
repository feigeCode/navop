use crate::protocols::common::capabilities::{ProviderCapabilities, ReasoningRequestStrategy};
use crate::types::{ChatRequest, ReasoningEffort};

#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ReasoningRequestParts {
    pub enable_thinking: Option<bool>,
    pub thinking_budget: Option<u32>,
    pub reasoning_effort: Option<ReasoningEffort>,
}

pub fn map_reasoning_request_parts_with_strategy(
    request: &ChatRequest,
    reasoning_request_strategy: ReasoningRequestStrategy,
) -> ReasoningRequestParts {
    let enable_thinking = match reasoning_request_strategy {
        ReasoningRequestStrategy::EnableThinking
        | ReasoningRequestStrategy::EnableThinkingWithBudget => request.enable_thinking,
        _ => None,
    };

    let thinking_budget = match reasoning_request_strategy {
        ReasoningRequestStrategy::ThinkingBudget
        | ReasoningRequestStrategy::EnableThinkingWithBudget => request.thinking_budget,
        _ => None,
    };

    let reasoning_effort = match reasoning_request_strategy {
        ReasoningRequestStrategy::ReasoningEffort => request.reasoning_effort,
        _ => None,
    };

    ReasoningRequestParts {
        enable_thinking,
        thinking_budget,
        reasoning_effort,
    }
}

pub fn map_reasoning_request_parts(
    request: &ChatRequest,
    capabilities: ProviderCapabilities,
) -> ReasoningRequestParts {
    map_reasoning_request_parts_with_strategy(request, capabilities.reasoning_request_strategy)
}

#[cfg(test)]
mod tests {
    use super::{map_reasoning_request_parts, map_reasoning_request_parts_with_strategy};
    use crate::protocols::common::capabilities::{ProviderCapabilities, ReasoningRequestStrategy};
    use crate::types::{ChatRequest, Message, ReasoningEffort};

    #[test]
    fn test_reasoning_effort_strategy_maps_only_reasoning_effort() {
        let request = ChatRequest::new("gpt-4.1")
            .add_message(Message::user("hello"))
            .with_enable_thinking(true)
            .with_thinking_budget(2048)
            .with_reasoning_effort(ReasoningEffort::High);

        let parts = map_reasoning_request_parts(&request, ProviderCapabilities::openai());

        assert_eq!(parts.enable_thinking, None);
        assert_eq!(parts.thinking_budget, None);
        assert_eq!(parts.reasoning_effort, Some(ReasoningEffort::High));
    }

    #[test]
    fn test_enable_thinking_strategy_maps_only_enable_flag() {
        let request = ChatRequest::new("qwen-plus")
            .add_message(Message::user("hello"))
            .with_enable_thinking(true)
            .with_thinking_budget(1024)
            .with_reasoning_effort(ReasoningEffort::Medium);

        let parts = map_reasoning_request_parts(&request, ProviderCapabilities::aliyun());

        assert_eq!(parts.enable_thinking, Some(true));
        assert_eq!(parts.thinking_budget, None);
        assert_eq!(parts.reasoning_effort, None);
    }

    #[test]
    fn test_thinking_budget_strategy_maps_only_budget() {
        let request = ChatRequest::new("claude-3.7-sonnet")
            .add_message(Message::user("hello"))
            .with_enable_thinking(true)
            .with_thinking_budget(4096);

        let parts = map_reasoning_request_parts(&request, ProviderCapabilities::anthropic());

        assert_eq!(parts.enable_thinking, None);
        assert_eq!(parts.thinking_budget, Some(4096));
        assert_eq!(parts.reasoning_effort, None);
    }

    #[test]
    fn test_strategy_helper_maps_without_provider_capabilities_wrapper() {
        let request = ChatRequest::new("gpt-4.1")
            .add_message(Message::user("hello"))
            .with_enable_thinking(true)
            .with_thinking_budget(2048)
            .with_reasoning_effort(ReasoningEffort::Medium);

        let parts = map_reasoning_request_parts_with_strategy(
            &request,
            ReasoningRequestStrategy::ReasoningEffort,
        );

        assert_eq!(parts.enable_thinking, None);
        assert_eq!(parts.thinking_budget, None);
        assert_eq!(parts.reasoning_effort, Some(ReasoningEffort::Medium));
    }
}
