//! 本地后端的上下文用量展示。
//!
//! agent_runtime 的 [`Session`](agent_runtime::runtime::Session) 记录「用了多少
//! token」(见 `context_tokens`);这里补上另一半——「窗口多大」。provider
//! 响应里没有窗口信息,只能按模型名查一张尽力而为的表:命中给百分比,没命中
//! 老老实实只显示 token 数,绝不猜。
//!
//! 窗口大小由各 provider 的传输层决定,navop 的 provider 层拿不到,
//! 所以百分比是 best-effort;账户限额面板
//! (Claude OAuth / Codex 的 rate-limit lanes)则整个不适用——navop 的 provider
//! 没有这类账户端点,刻意不实现。

/// 常见模型的上下文窗口(输入侧,best-effort)。
///
/// 只收高置信度的主流型号;命不中就返回 `None`,由调用方退化为纯 token 数。
/// 匹配按小写子串做——模型 id 通常带日期后缀(`claude-sonnet-4-5-20250929`),
/// 前缀表能同时吃到带不带后缀的写法。**新增条目前先核实官方文档**;表过时
/// 的代价是百分比不准,宁缺毋滥。
const CONTEXT_WINDOWS: &[(&str, u64)] = &[
    // Anthropic Claude:3.x / 4.x 均为 200k。
    ("claude", 200_000),
    // OpenAI:gpt-4o / gpt-4-turbo 128k;gpt-4.1 1M;gpt-5 400k;o 系列 200k。
    ("gpt-4.1", 1_000_000),
    ("gpt-4o", 128_000),
    ("gpt-4-turbo", 128_000),
    ("gpt-5", 400_000),
    ("o1", 200_000),
    ("o3", 200_000),
    ("o4", 200_000),
    // DeepSeek V3 / V3.x 系列。
    ("deepseek", 128_000),
    // 通义千问:qwen3 256k;qwen-max/plus/turbo 128k。
    ("qwen3", 256_000),
    ("qwen", 128_000),
    // 智谱 GLM-4 / 4.5 / 4.6。
    ("glm-4", 200_000),
    // 字节 Doubao 1.5 pro。
    ("doubao-1-5-pro", 256_000),
    // 月之暗面 Kimi K2。
    ("kimi-k2", 128_000),
    ("moonshot", 128_000),
    // Google Gemini 2.x / 3 系列。
    ("gemini", 1_000_000),
    // MiniMax M1 / M2。
    ("minimax", 1_000_000),
];

/// 按模型名尽力而为地解析上下文窗口;未收录返回 `None`。
///
/// 顺序敏感:`qwen3` 必须排在 `qwen` 前,否则会被宽前缀吃掉。
pub fn model_context_window(model: &str) -> Option<u64> {
    let model = model.to_ascii_lowercase();
    CONTEXT_WINDOWS
        .iter()
        .find(|(prefix, _)| model.contains(prefix))
        .map(|(_, window)| *window)
}

/// 本地用量文案:`used/window tokens`;窗口未知时只有 `used tokens`。
///
/// 与 ACP 侧的 `format_acp_usage` 同款裸数字格式(不做 k/m 缩写),两边
/// 摆在同一个 chip 位置时观感一致。
pub fn format_local_usage(tokens: u64, window: Option<u64>) -> String {
    match window {
        Some(window) if window > 0 => format!("{}/{} tokens", tokens, window),
        _ => format!("{tokens} tokens"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_models_resolve_their_window() {
        assert_eq!(
            model_context_window("claude-sonnet-4-5-20250929"),
            Some(200_000)
        );
        assert_eq!(model_context_window("Claude-3-5-Haiku"), Some(200_000));
        assert_eq!(model_context_window("gpt-4o-mini"), Some(128_000));
        assert_eq!(model_context_window("gpt-5-codex"), Some(400_000));
        assert_eq!(model_context_window("deepseek-chat"), Some(128_000));
        assert_eq!(model_context_window("qwen3-max"), Some(256_000));
        assert_eq!(model_context_window("qwen-max"), Some(128_000));
    }

    #[test]
    fn specific_prefix_wins_over_the_generic_one() {
        // qwen3(256k)不能被更宽的 qwen(128k)前缀吞掉。
        assert_eq!(model_context_window("qwen3-235b"), Some(256_000));
    }

    #[test]
    fn unknown_model_reports_no_window() {
        assert_eq!(model_context_window("my-private-finetune"), None);
        assert_eq!(model_context_window(""), None);
    }

    #[test]
    fn format_shows_the_window_when_known() {
        assert_eq!(
            format_local_usage(1500, Some(128_000)),
            "1500/128000 tokens"
        );
    }

    #[test]
    fn format_falls_back_to_tokens_without_a_window() {
        assert_eq!(format_local_usage(1500, None), "1500 tokens");
        // 窗口为 0 视同未知——避免除零式的误导百分比。
        assert_eq!(format_local_usage(1500, Some(0)), "1500 tokens");
    }
}
