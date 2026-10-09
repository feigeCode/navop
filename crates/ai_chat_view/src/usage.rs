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

/// 上下文用量的一次读数,供 composer 里的圆环 gauge 使用。
///
/// 两个数据源共用这一种形状:本地后端从 runtime session 的
/// `context_tokens` + 模型窗口表推出;ACP 侧直接用 agent 报的
/// `used`/`size`。窗口未知时 `window` 为 `None`,圆环退化成
/// 不确定态——只显示数字,不画一个骗人的百分比。
#[derive(Clone, Debug, PartialEq)]
pub struct ContextUsage {
    /// 已消耗的 token。
    pub used: u64,
    /// 窗口大小;`None` 表示无从得知(不猜)。
    pub window: Option<u64>,
    /// agent 报的费用文案(如 `$0.0124`);本地后端没有这项。
    pub cost: Option<String>,
}

/// 上下文占用分档,决定圆环颜色。
///
/// 阈值取 70% / 90%:前者是「该留意了」,后者是「快压缩了」。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UsagePressure {
    /// < 70%。
    Calm,
    /// 70% – 90%。
    Elevated,
    /// >= 90%。接近客户端自动压缩/截断的区间。
    Critical,
}

impl ContextUsage {
    pub fn new(used: u64, window: Option<u64>) -> Self {
        Self {
            used,
            window,
            cost: None,
        }
    }

    pub fn with_cost(mut self, cost: Option<impl Into<String>>) -> Self {
        self.cost = cost.map(Into::into);
        self
    }

    /// 占用比例(0.0–100.0);窗口未知或为 0 时返回 `None`。
    pub fn percent(&self) -> Option<f32> {
        match self.window {
            Some(window) if window > 0 => {
                Some((self.used as f64 / window as f64 * 100.0).clamp(0.0, 100.0) as f32)
            }
            _ => None,
        }
    }

    pub fn pressure(&self) -> UsagePressure {
        match self.percent() {
            Some(percent) if percent >= 90.0 => UsagePressure::Critical,
            Some(percent) if percent >= 70.0 => UsagePressure::Elevated,
            _ => UsagePressure::Calm,
        }
    }

    /// 圆环中心/旁边的短文案:窗口已知给百分比,未知给压缩过的 token 数。
    pub fn gauge_label(&self) -> String {
        match self.percent() {
            Some(percent) => format!("{}%", percent.round() as u32),
            None => format_token_count(self.used),
        }
    }

    /// 悬停/无障碍用的完整读数。
    pub fn detail_text(&self) -> String {
        let mut text = match self.window {
            Some(window) if window > 0 => format!("{}/{} tokens", self.used, window),
            _ => format!("{} tokens", self.used),
        };
        if let Some(cost) = self.cost.as_deref() {
            text.push_str(" · ");
            text.push_str(cost);
        }
        text
    }
}

/// token 数压缩成 3–4 字符:`999` / `12.3k` / `1.2M`。
///
/// 圆环旁边的地方只有这么宽;精确数字放 tooltip。
pub fn format_token_count(tokens: u64) -> String {
    match tokens {
        0..=9_999 => tokens.to_string(),
        10_000..=999_999 => format!("{:.1}k", tokens as f64 / 1_000.0),
        _ => format!("{:.1}M", tokens as f64 / 1_000_000.0),
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

    #[test]
    fn usage_percent_needs_a_known_window() {
        assert_eq!(ContextUsage::new(50_000, Some(200_000)).percent(), Some(25.0));
        // 窗口未知/为 0:不猜百分比。
        assert_eq!(ContextUsage::new(1500, None).percent(), None);
        assert_eq!(ContextUsage::new(1500, Some(0)).percent(), None);
        // 溢出窗口时夹到 100,不让圆环画过一整圈。
        assert_eq!(ContextUsage::new(300_000, Some(200_000)).percent(), Some(100.0));
    }

    #[test]
    fn pressure_tracks_the_70_and_90_boundaries() {
        let at = |used: u64| ContextUsage::new(used, Some(100)).pressure();
        assert_eq!(at(69), UsagePressure::Calm);
        assert_eq!(at(70), UsagePressure::Elevated);
        assert_eq!(at(89), UsagePressure::Elevated);
        assert_eq!(at(90), UsagePressure::Critical);
        // 窗口未知时永远不报警——没数据就不吓人。
        assert_eq!(ContextUsage::new(999_999, None).pressure(), UsagePressure::Calm);
    }

    #[test]
    fn gauge_label_prefers_percent_and_falls_back_to_tokens() {
        assert_eq!(ContextUsage::new(50_000, Some(200_000)).gauge_label(), "25%");
        assert_eq!(ContextUsage::new(12_345, None).gauge_label(), "12.3k");
        assert_eq!(ContextUsage::new(842, None).gauge_label(), "842");
    }

    #[test]
    fn detail_text_appends_cost_only_when_reported() {
        assert_eq!(
            ContextUsage::new(1500, Some(128_000)).detail_text(),
            "1500/128000 tokens"
        );
        assert_eq!(
            ContextUsage::new(1500, Some(128_000))
                .with_cost(Some("$0.0124"))
                .detail_text(),
            "1500/128000 tokens · $0.0124"
        );
    }

    #[test]
    fn token_count_compacts_only_when_it_has_to() {
        assert_eq!(format_token_count(0), "0");
        assert_eq!(format_token_count(9_999), "9999");
        assert_eq!(format_token_count(10_000), "10.0k");
        assert_eq!(format_token_count(999_999), "1000.0k");
        assert_eq!(format_token_count(1_200_000), "1.2M");
    }
}
