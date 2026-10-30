use llm_connector::types::ReasoningEffort;

/// GPT-6 系列推理模型（`gpt-6.1-sol`、`gpt-6-sol` 等）在 `/v1/chat/completions` 上
/// 默认就开启推理，而「推理」与「函数工具」在该端点上互斥：上游会直接拒绝并返回
/// `Function tools with reasoning_effort are not supported for <model> in
/// /v1/chat/completions`，同时提示改用 `/v1/responses` 或把 `reasoning_effort`
/// 设为 `none`（见 #487）。
///
/// navop 的模型桥接只走 chat/completions（`invoke_responses` 未接入），所以带工具的
/// 请求必须显式把推理关掉；不带工具的普通对话保持上游默认行为（继续推理）。
const TOOL_INCOMPATIBLE_REASONING_MODEL_PREFIXES: &[&str] = &["gpt-6"];

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

/// 带工具的请求在此类模型上要显式传 `reasoning_effort: "none"`，否则上游拒绝请求。
pub(super) fn reasoning_effort_for_request(
    model: &str,
    has_tools: bool,
) -> Option<ReasoningEffort> {
    (has_tools && requires_reasoning_off_for_tools(model)).then_some(ReasoningEffort::None)
}

fn requires_reasoning_off_for_tools(model: &str) -> bool {
    let id = normalized_model_id(model);
    TOOL_INCOMPATIBLE_REASONING_MODEL_PREFIXES
        .iter()
        .any(|prefix| id.starts_with(prefix))
}

/// 兼容 `provider/model` 前缀写法（如 `9router/gpt-6.1-sol`），只按模型 id 判断。
fn normalized_model_id(model: &str) -> String {
    let id = model.rsplit('/').next().unwrap_or(model);
    id.trim().to_ascii_lowercase()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpt_6_models_require_reasoning_off_when_tools_are_present() {
        for model in ["gpt-6.1-sol", "gpt-6-sol", "gpt-6-astra", "gpt-6-luna"] {
            assert_eq!(
                Some(ReasoningEffort::None),
                reasoning_effort_for_request(model, true),
                "{model} 带工具时必须显式关闭推理"
            );
        }
    }

    #[test]
    fn model_id_prefix_and_case_are_normalized() {
        assert_eq!(
            Some(ReasoningEffort::None),
            reasoning_effort_for_request("9router/GPT-6.1-Sol", true),
            "带 provider 前缀/大写的模型 id 也要命中"
        );
    }

    #[test]
    fn plain_chat_keeps_upstream_default_reasoning() {
        assert_eq!(
            None,
            reasoning_effort_for_request("gpt-6.1-sol", false),
            "没有工具时应保留默认推理，不主动改成 none"
        );
    }

    #[test]
    fn other_models_are_left_untouched() {
        for model in ["gpt-5.5", "gpt-5.6-sol", "gpt-4.1", "deepseek-v4-flash"] {
            assert_eq!(
                None,
                reasoning_effort_for_request(model, true),
                "{model} 不应被注入 reasoning_effort"
            );
        }
    }
}
