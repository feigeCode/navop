use serde::{Deserialize, Serialize};

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
            ProviderType::AzureOpenAI,
            ProviderType::OpenAICompatible,
        ]
    }

    /// 是否需要用户提供 API Key。只有本地部署的 Ollama / Xinference 可以不填。
    pub fn requires_api_key(&self) -> bool {
        !matches!(self, ProviderType::Ollama | ProviderType::Xinference)
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
    use super::ProviderType;

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
    }
}
