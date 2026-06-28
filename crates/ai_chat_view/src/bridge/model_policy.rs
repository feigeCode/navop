pub(super) fn should_disable_function_calling_for_model(provider_name: &str, model: &str) -> bool {
    let _ = (provider_name, model);
    false
}

pub(super) fn should_disable_tool_choice_for_model(provider_name: &str, model: &str) -> bool {
    let _ = model;
    provider_name.eq_ignore_ascii_case("ollama")
}

pub(super) fn should_stream_tools_via_completion(provider_name: &str, _model: &str) -> bool {
    provider_name.eq_ignore_ascii_case("ollama")
}
