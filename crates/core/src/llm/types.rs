use serde::{Deserialize, Serialize};

pub use llm_connector::types::ReasoningEffort;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ProviderType {
    OpenAI,
    Anthropic,
    Aliyun,
    Zhipu,
    Ollama,
    Volcengine,
    Moonshot,
    DeepSeek,
    Google,
    /// 聚合网关：一个 key 覆盖多家模型（含 OpenAI / Anthropic / Google 系）。
    OpenRouter,
    /// MiniMax（稀宇科技）OpenAI 兼容端点。
    MiniMax,
    /// 硅基流动 OpenAI 兼容端点（国内站）。
    SiliconFlow,
    /// 腾讯混元 OpenAI 兼容端点（单 API Key，非 TC3 签名协议）。
    TencentHunyuan,
    /// 小米 MiMo。
    Xiaomi,
    /// 本地 / 自托管 Xinference（OpenAI 兼容，默认无需 API Key）。
    Xinference,
    /// OpenCode Go 订阅：OpenAI 兼容端点，要求稳定的 `x-opencode-session` 请求头。
    OpenCodeGo,
    AzureOpenAI,
    OpenAICompatible,
}

impl ProviderType {
    pub fn as_str(&self) -> &'static str {
        match self {
            ProviderType::OpenAI => "openai",
            ProviderType::Anthropic => "anthropic",
            ProviderType::Aliyun => "aliyun",
            ProviderType::Zhipu => "zhipu",
            ProviderType::Ollama => "ollama",
            ProviderType::Volcengine => "volcengine",
            ProviderType::Moonshot => "moonshot",
            ProviderType::DeepSeek => "deepseek",
            ProviderType::Google => "google",
            ProviderType::OpenRouter => "openrouter",
            ProviderType::MiniMax => "minimax",
            ProviderType::SiliconFlow => "siliconflow",
            ProviderType::TencentHunyuan => "tencent_hunyuan",
            ProviderType::Xiaomi => "xiaomi",
            ProviderType::Xinference => "xinference",
            ProviderType::OpenCodeGo => "opencode_go",
            ProviderType::AzureOpenAI => "azure_openai",
            ProviderType::OpenAICompatible => "openai_compatible",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "openai" => Some(ProviderType::OpenAI),
            "anthropic" => Some(ProviderType::Anthropic),
            "aliyun" => Some(ProviderType::Aliyun),
            "zhipu" => Some(ProviderType::Zhipu),
            "ollama" => Some(ProviderType::Ollama),
            "volcengine" => Some(ProviderType::Volcengine),
            "moonshot" => Some(ProviderType::Moonshot),
            "deepseek" => Some(ProviderType::DeepSeek),
            "google" => Some(ProviderType::Google),
            "openrouter" => Some(ProviderType::OpenRouter),
            "minimax" => Some(ProviderType::MiniMax),
            "siliconflow" => Some(ProviderType::SiliconFlow),
            "tencent_hunyuan" => Some(ProviderType::TencentHunyuan),
            "xiaomi" => Some(ProviderType::Xiaomi),
            "xinference" => Some(ProviderType::Xinference),
            "opencode_go" => Some(ProviderType::OpenCodeGo),
            "azure_openai" => Some(ProviderType::AzureOpenAI),
            "openai_compatible" => Some(ProviderType::OpenAICompatible),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ProviderType::OpenAI => "OpenAI",
            ProviderType::Anthropic => "Anthropic",
            ProviderType::Aliyun => "Aliyun (DashScope)",
            ProviderType::Zhipu => "Zhipu (GLM)",
            ProviderType::Ollama => "Ollama",
            ProviderType::Volcengine => "Volcengine",
            ProviderType::Moonshot => "Moonshot",
            ProviderType::DeepSeek => "DeepSeek",
            ProviderType::Google => "Google (Gemini)",
            ProviderType::OpenRouter => "OpenRouter",
            ProviderType::MiniMax => "MiniMax",
            ProviderType::SiliconFlow => "SiliconFlow",
            ProviderType::TencentHunyuan => "Tencent Hunyuan",
            ProviderType::Xiaomi => "Xiaomi (MiMo)",
            ProviderType::Xinference => "Xinference",
            ProviderType::OpenCodeGo => "OpenCode Go",
            ProviderType::AzureOpenAI => "Azure OpenAI",
            ProviderType::OpenAICompatible => "OpenAI Compatible",
        }
    }

    pub fn all() -> Vec<Self> {
        vec![
            ProviderType::OpenAI,
            ProviderType::Anthropic,
            ProviderType::Aliyun,
            ProviderType::Zhipu,
            ProviderType::Ollama,
            ProviderType::Volcengine,
            ProviderType::Moonshot,
            ProviderType::DeepSeek,
            ProviderType::Google,
            ProviderType::OpenRouter,
            ProviderType::MiniMax,
            ProviderType::SiliconFlow,
            ProviderType::TencentHunyuan,
            ProviderType::Xiaomi,
            ProviderType::Xinference,
            ProviderType::OpenCodeGo,
            ProviderType::AzureOpenAI,
            ProviderType::OpenAICompatible,
        ]
    }

    /// 是否需要用户提供 API Key。只有本地部署的 Ollama / Xinference 可以不填。
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, ProviderType::Ollama | ProviderType::Xinference)
    }
}

/// 供应商使用哪个 OpenAI 风格接口。
///
/// 部分端点（例如火山方舟的 `/responses`）只提供 Responses API，用户需要能显式
/// 选择走 `/chat/completions` 还是 `/responses`（issue #565）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ApiMode {
    /// 默认：`POST /chat/completions`（OpenAI 兼容事实标准）。
    #[default]
    ChatCompletions,
    /// `POST /responses`（OpenAI Responses API）。
    Responses,
}

impl ApiMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ApiMode::ChatCompletions => "chat_completions",
            ApiMode::Responses => "responses",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "chat_completions" | "chat" | "chat_completion" => Some(ApiMode::ChatCompletions),
            "responses" | "response" => Some(ApiMode::Responses),
            _ => None,
        }
    }

    pub fn display_name(&self) -> &'static str {
        match self {
            ApiMode::ChatCompletions => "Chat Completions",
            ApiMode::Responses => "Responses",
        }
    }

    pub fn all() -> [Self; 2] {
        [ApiMode::ChatCompletions, ApiMode::Responses]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderConfig {
    pub id: i64,
    pub name: String,
    pub provider_type: ProviderType,
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub api_version: Option<String>,
    /// Extra HTTP headers for OpenAI-compatible gateways, stored in display order.
    #[serde(default)]
    pub extra_headers: Vec<(String, String)>,
    /// 供应商级代理地址；`None` 表示跟随全局代理设置。
    #[serde(default)]
    pub proxy: Option<String>,
    /// 默认思考级别；`None` 表示不传该参数，跟随上游默认。
    #[serde(default)]
    pub reasoning_effort: Option<ReasoningEffort>,
    /// 该供应商走 `/chat/completions` 还是 `/responses`（issue #565）。
    #[serde(default)]
    pub api_mode: ApiMode,
    pub model: String,
    pub models: Vec<String>,
    pub max_tokens: Option<i32>,
    pub temperature: Option<f32>,
    pub enabled: bool,
    pub is_default: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Default for ProviderConfig {
    fn default() -> Self {
        Self {
            id: 0,
            name: String::new(),
            provider_type: ProviderType::OpenAI,
            api_key: None,
            api_base: None,
            api_version: None,
            extra_headers: Vec::new(),
            proxy: None,
            reasoning_effort: None,
            api_mode: ApiMode::ChatCompletions,
            model: String::new(),
            models: Vec::new(),
            max_tokens: None,
            temperature: None,
            enabled: true,
            is_default: false,
            created_at: 0,
            updated_at: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ApiMode, ProviderConfig, ProviderType};

    /// 新供应商默认走 chat/completions，老配置读回也不能改变行为。
    #[test]
    fn api_mode_defaults_to_chat_completions() {
        assert_eq!(ApiMode::ChatCompletions, ApiMode::default());
        assert_eq!(ApiMode::ChatCompletions, ProviderConfig::default().api_mode);
    }

    /// 存储里用 snake_case 字面量，来回都能认出来（issue #565）。
    #[test]
    fn api_mode_round_trips_storage_ids() {
        let cases = [
            (ApiMode::ChatCompletions, "chat_completions"),
            (ApiMode::Responses, "responses"),
        ];

        for (api_mode, storage_id) in cases {
            assert_eq!(storage_id, api_mode.as_str());
            assert_eq!(Some(api_mode), ApiMode::from_str(storage_id));
            assert!(
                !api_mode.display_name().is_empty(),
                "{storage_id} 缺少展示名"
            );
        }

        assert_eq!(None, ApiMode::from_str("unknown"));
        assert_eq!(2, ApiMode::all().len());
    }

    #[test]
    fn provider_config_deserializes_legacy_json_without_api_mode() {
        let mut value = serde_json::to_value(ProviderConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("api_mode");

        let config: ProviderConfig = serde_json::from_value(value).unwrap();
        assert_eq!(ApiMode::ChatCompletions, config.api_mode);
    }

    #[test]
    fn provider_config_serializes_api_mode_with_snake_case() {
        let config = ProviderConfig {
            api_mode: ApiMode::Responses,
            ..Default::default()
        };

        let value = serde_json::to_value(config).unwrap();
        assert_eq!(Some("responses"), value["api_mode"].as_str());
    }

    #[test]
    fn provider_config_defaults_to_no_extra_headers() {
        assert!(ProviderConfig::default().extra_headers.is_empty());
    }

    #[test]
    fn provider_config_deserializes_legacy_json_without_extra_headers() {
        let mut value = serde_json::to_value(ProviderConfig::default()).unwrap();
        value.as_object_mut().unwrap().remove("extra_headers");

        let config: ProviderConfig = serde_json::from_value(value).unwrap();
        assert!(config.extra_headers.is_empty());
    }

    #[test]
    fn opencode_go_is_an_api_key_provider_offered_to_users() {
        let provider_type = ProviderType::OpenCodeGo;

        assert_eq!("opencode_go", provider_type.as_str());
        assert_eq!(Some(provider_type), ProviderType::from_str("opencode_go"));
        assert_eq!("OpenCode Go", provider_type.display_name());
        assert!(provider_type.requires_api_key());
        assert!(ProviderType::all().contains(&provider_type));
    }

    /// 内置 Navop AI（`onet_cli`）已下线：历史配置值不再被识别，也不再出现在候选列表里。
    #[test]
    fn builtin_navop_ai_provider_is_no_longer_recognised() {
        assert_eq!(None, ProviderType::from_str("onet_cli"));
        assert!(
            !ProviderType::all()
                .iter()
                .any(|provider_type| provider_type.as_str() == "onet_cli"),
            "内置 Navop AI 不应再出现在 ProviderType::all() 中"
        );
    }

    /// 内置 provider 下线后，只剩本地部署的两家可以不填 API Key。
    #[test]
    fn only_local_providers_skip_api_key() {
        let without_api_key: Vec<ProviderType> = ProviderType::all()
            .into_iter()
            .filter(|provider_type| !provider_type.requires_api_key())
            .collect();

        assert_eq!(
            vec![ProviderType::Ollama, ProviderType::Xinference],
            without_api_key
        );
    }

    /// 新增供应商的存储 id 必须能原样往返：旧配置读回不丢、新配置写入可读。
    #[test]
    fn added_provider_types_round_trip_through_storage_ids() {
        let cases = [
            (ProviderType::OpenRouter, "openrouter"),
            (ProviderType::MiniMax, "minimax"),
            (ProviderType::SiliconFlow, "siliconflow"),
            (ProviderType::TencentHunyuan, "tencent_hunyuan"),
            (ProviderType::Xiaomi, "xiaomi"),
            (ProviderType::Xinference, "xinference"),
            (ProviderType::OpenCodeGo, "opencode_go"),
        ];

        for (provider_type, storage_id) in cases {
            assert_eq!(storage_id, provider_type.as_str());
            assert_eq!(Some(provider_type), ProviderType::from_str(storage_id));
            assert!(
                !provider_type.display_name().is_empty(),
                "{storage_id} 缺少展示名"
            );
            assert!(
                ProviderType::all().contains(&provider_type),
                "{storage_id} 未加入 all()"
            );
        }
    }

    /// 新增供应商必须出现在设置页的候选列表里，否则用户选不到（issue #379 的成因）。
    #[test]
    fn added_provider_types_are_offered_to_users() {
        let all = ProviderType::all();

        for provider_type in [
            ProviderType::OpenRouter,
            ProviderType::MiniMax,
            ProviderType::SiliconFlow,
            ProviderType::TencentHunyuan,
            ProviderType::Xiaomi,
            ProviderType::Xinference,
            ProviderType::OpenCodeGo,
        ] {
            assert!(
                all.contains(&provider_type),
                "{} 未出现在候选列表中",
                provider_type.as_str()
            );
        }
    }

    #[test]
    fn xinference_is_the_only_added_provider_without_api_key() {
        assert!(!ProviderType::Xinference.requires_api_key());
        assert!(ProviderType::OpenRouter.requires_api_key());
        assert!(ProviderType::SiliconFlow.requires_api_key());
        assert!(ProviderType::TencentHunyuan.requires_api_key());
        assert!(ProviderType::Xiaomi.requires_api_key());
        assert!(ProviderType::MiniMax.requires_api_key());
        assert!(ProviderType::OpenCodeGo.requires_api_key());
    }
}
