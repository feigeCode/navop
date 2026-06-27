pub(super) fn should_disable_function_calling_for_model(provider_name: &str, model: &str) -> bool {
    let provider = provider_name.to_ascii_lowercase();
    let model = model.to_ascii_lowercase();
    let is_deepseek = provider == "deepseek" || model.contains("deepseek");
    let is_thinking_model = model.contains("v4")
        || model.contains("reasoner")
        || model.contains("r1")
        || model.contains("thinking");

    is_deepseek && is_thinking_model
}

pub(super) fn should_disable_tool_choice_for_model(provider_name: &str, _model: &str) -> bool {
    provider_name.eq_ignore_ascii_case("ollama")
}

pub(super) fn should_stream_tools_via_completion(provider_name: &str, _model: &str) -> bool {
    provider_name.eq_ignore_ascii_case("ollama")
}
