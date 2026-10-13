use std::pin::Pin;
use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use futures::Stream;
use llm_connector::builder::LlmClientBuilder;
use llm_connector::types::{ChatRequest, ChatResponse, Message, Role, StreamingResponse};
use llm_connector::{GenericProvider, HttpClient, LlmClient, OpenAIProtocol};

use super::types::{ProviderConfig, ProviderType};

pub type ChatStream = Pin<Box<dyn Stream<Item = Result<StreamingResponse>> + Send>>;

const OPENAI_BASE_URL: &str = "https://api.openai.com/v1";
const ANTHROPIC_BASE_URL: &str = "https://api.anthropic.com";
const ALIYUN_BASE_URL: &str = "https://dashscope.aliyuncs.com";
const ALIYUN_COMPATIBLE_BASE_URL: &str = "https://dashscope.aliyuncs.com/compatible-mode/v1";
const ZHIPU_BASE_URL: &str = "https://open.bigmodel.cn";
const OLLAMA_BASE_URL: &str = "http://localhost:11434";
const VOLCENGINE_BASE_URL: &str = "https://ark.cn-beijing.volces.com/api/v3";
const MOONSHOT_BASE_URL: &str = "https://api.moonshot.cn/v1";
const DEEPSEEK_BASE_URL: &str = "https://api.deepseek.com";
const GOOGLE_BASE_URL: &str = "https://generativelanguage.googleapis.com/v1beta";
const OPENROUTER_BASE_URL: &str = "https://openrouter.ai/api/v1";
/// MiniMax 国内站；国际站为 https://api.minimax.io/v1，可由用户改 api_base 切换。
const MINIMAX_BASE_URL: &str = "https://api.minimaxi.com/v1";
/// 硅基流动国内站；国际站为 https://api.siliconflow.com/v1。
const SILICONFLOW_BASE_URL: &str = "https://api.siliconflow.cn/v1";
/// 腾讯混元 OpenAI 兼容端点（单 API Key），非 TC3 签名协议的 hunyuan.tencentcloudapi.com。
const HUNYUAN_BASE_URL: &str = "https://api.hunyuan.cloud.tencent.com/v1";
const XIAOMI_BASE_URL: &str = "https://api.xiaomimimo.com/v1";
const XINFERENCE_BASE_URL: &str = "http://127.0.0.1:9997/v1";
/// Idle timeout: aborts a request only after this long without any bytes, so
/// long streaming responses (thinking/CoT, tool loops) are not cut off.
pub const LLM_CLIENT_TIMEOUT_SECS: u64 = 120;

pub use llm_connector::types::{
    ChatRequest as LlmChatRequest, Message as LlmMessage, Role as LlmRole,
};

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(&self, request: &ChatRequest) -> Result<String>;
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream>;
    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse> {
        let content = self.chat(request).await?;
        Ok(ChatResponse {
            content,
            ..Default::default()
        })
    }

    async fn models(&self) -> Result<Vec<String>>;
    fn provider_name(&self) -> &str;
}

pub struct LlmConnector {
    client: LlmClient,
    provider_type: ProviderType,
}

impl LlmConnector {
    pub fn from_config(config: &ProviderConfig) -> Result<Self> {
        Self::from_config_with_proxy(config, None)
    }

    pub fn from_config_with_proxy(
        config: &ProviderConfig,
        proxy_url: Option<&str>,
    ) -> Result<Self> {
        Self::from_config_with_proxy_and_timeout(config, proxy_url, LLM_CLIENT_TIMEOUT_SECS)
    }

    /// 与 [`Self::from_config_with_proxy`] 相同，但显式指定请求空闲超时（秒）。
    pub fn from_config_with_proxy_and_timeout(
        config: &ProviderConfig,
        proxy_url: Option<&str>,
        request_timeout_secs: u64,
    ) -> Result<Self> {
        let client = client_from_config(
            config,
            ClientOptions {
                proxy_url,
                timeout_secs: request_timeout_secs,
            },
        )?;

        Ok(Self {
            client,
            provider_type: config.provider_type,
        })
    }

    pub fn build_request(&self, config: &ProviderConfig, messages: Vec<Message>) -> ChatRequest {
        let mut request = ChatRequest {
            model: config.model.clone(),
            messages,
            ..Default::default()
        };

        if let Some(max_tokens) = config.max_tokens {
            request.max_tokens = Some(max_tokens as u32);
        }

        if let Some(temperature) = config.temperature {
            request.temperature = Some(temperature);
        }

        request
    }
}

/// 构建单个 provider 客户端所需的网络选项。
#[derive(Clone, Copy)]
struct ClientOptions<'a> {
    proxy_url: Option<&'a str>,
    timeout_secs: u64,
}

fn client_from_config(config: &ProviderConfig, options: ClientOptions<'_>) -> Result<LlmClient> {
    match config.provider_type {
        ProviderType::OpenAI => build_client(
            LlmClient::builder().openai(required_api_key(config, "OpenAI")?),
            provider_base_url(config, OPENAI_BASE_URL),
            options,
        ),
        ProviderType::Anthropic => build_client(
            LlmClient::builder().anthropic(required_api_key(config, "Anthropic")?),
            provider_base_url(config, ANTHROPIC_BASE_URL),
            options,
        ),
        ProviderType::Aliyun => aliyun_client(config, options),
        ProviderType::Zhipu => build_client(
            LlmClient::builder().zhipu(required_api_key(config, "Zhipu")?),
            provider_base_url(config, ZHIPU_BASE_URL),
            options,
        ),
        ProviderType::Ollama => build_client(
            LlmClient::builder().ollama(),
            provider_base_url(config, OLLAMA_BASE_URL),
            options,
        ),
        ProviderType::Volcengine => build_client(
            LlmClient::builder().volcengine(required_api_key(config, "Volcengine")?),
            provider_base_url(config, VOLCENGINE_BASE_URL),
            options,
        ),
        ProviderType::Moonshot => build_client(
            LlmClient::builder().moonshot(required_api_key(config, "Moonshot")?),
            provider_base_url(config, MOONSHOT_BASE_URL),
            options,
        ),
        ProviderType::DeepSeek => build_client(
            LlmClient::builder().deepseek(required_api_key(config, "DeepSeek")?),
            provider_base_url(config, DEEPSEEK_BASE_URL),
            options,
        ),
        ProviderType::Google => build_client(
            LlmClient::builder().google(required_api_key(config, "Google")?),
            provider_base_url(config, GOOGLE_BASE_URL),
            options,
        ),
        // OpenRouter：service_name 复用 llm-connector 已内置的 openrouter 能力位。
        ProviderType::OpenRouter => openai_compatible_client(
            required_api_key(config, "OpenRouter")?,
            provider_base_url(config, OPENROUTER_BASE_URL),
            "openrouter",
            options,
        ),
        // MiniMax：复用 minimax 能力位（文本块模式，不带多模态）。
        ProviderType::MiniMax => openai_compatible_client(
            required_api_key(config, "MiniMax")?,
            provider_base_url(config, MINIMAX_BASE_URL),
            "minimax",
            options,
        ),
        ProviderType::SiliconFlow => openai_compatible_client(
            required_api_key(config, "SiliconFlow")?,
            provider_base_url(config, SILICONFLOW_BASE_URL),
            "siliconflow",
            options,
        ),
        ProviderType::TencentHunyuan => openai_compatible_client(
            required_api_key(config, "Tencent Hunyuan")?,
            provider_base_url(config, HUNYUAN_BASE_URL),
            "hunyuan",
            options,
        ),
        ProviderType::Xiaomi => build_client(
            LlmClient::builder().xiaomi(required_api_key(config, "Xiaomi MiMo")?),
            provider_base_url(config, XIAOMI_BASE_URL),
            options,
        ),
        ProviderType::Xinference => build_client(
            LlmClient::builder().xinference(),
            provider_base_url(config, XINFERENCE_BASE_URL),
            options,
        ),
        ProviderType::AzureOpenAI => azure_openai_client(config, options),
        ProviderType::OpenAICompatible => openai_compatible_client(
            required_api_key(config, "OpenAI Compatible")?,
            required_base_url(config, "OpenAI Compatible")?,
            &config.name,
            options,
        ),
    }
}

fn build_client(
    mut builder: LlmClientBuilder,
    base_url: &str,
    options: ClientOptions<'_>,
) -> Result<LlmClient> {
    builder = builder.base_url(base_url).timeout(options.timeout_secs);
    if let Some(proxy_url) = options.proxy_url {
        builder = builder.proxy(proxy_url);
    }
    Ok(builder.build()?)
}

fn aliyun_client(config: &ProviderConfig, options: ClientOptions<'_>) -> Result<LlmClient> {
    let api_key = required_api_key(config, "Aliyun")?;
    if aliyun_prefers_compatible_mode(config) {
        openai_compatible_client(api_key, aliyun_base_url(config), &config.name, options)
    } else {
        build_client(
            LlmClient::builder().aliyun(api_key),
            provider_base_url(config, ALIYUN_BASE_URL),
            options,
        )
    }
}

fn openai_compatible_client(
    api_key: &str,
    base_url: &str,
    service_name: &str,
    options: ClientOptions<'_>,
) -> Result<LlmClient> {
    if let Some(proxy_url) = options.proxy_url {
        let provider = llm_connector::providers::openai_compatible_with_config(
            api_key,
            base_url,
            service_name,
            Some(options.timeout_secs),
            Some(proxy_url),
        )?;
        return Ok(LlmClient::from_provider(Arc::new(provider)));
    }

    build_client(
        LlmClient::builder().openai_compatible(api_key, service_name),
        base_url,
        options,
    )
}

fn azure_openai_client(config: &ProviderConfig, options: ClientOptions<'_>) -> Result<LlmClient> {
    let api_key = required_api_key(config, "Azure OpenAI")?;
    let endpoint = required_base_url(config, "Azure OpenAI")?;
    let api_version = config
        .api_version
        .as_deref()
        .unwrap_or("2024-02-15-preview");

    if let Some(proxy_url) = options.proxy_url {
        let protocol = OpenAIProtocol::with_service(api_key, "azure-openai");
        let client =
            HttpClient::with_config(endpoint, Some(options.timeout_secs), Some(proxy_url))?
                .with_header("api-key".to_string(), api_key.to_string())
                .with_header("api-version".to_string(), api_version.to_string());
        let provider = GenericProvider::new(protocol, client);
        return Ok(LlmClient::from_provider(Arc::new(provider)));
    }

    build_client(
        LlmClient::builder().azure_openai(api_key, endpoint, api_version),
        endpoint,
        options,
    )
}

fn required_api_key<'a>(config: &'a ProviderConfig, provider_name: &str) -> Result<&'a str> {
    config
        .api_key
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("API key required for {provider_name}"))
}

fn required_base_url<'a>(config: &'a ProviderConfig, provider_name: &str) -> Result<&'a str> {
    config
        .api_base
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("Base URL required for {provider_name}"))
}

fn provider_base_url<'a>(config: &'a ProviderConfig, default_base_url: &'static str) -> &'a str {
    config.api_base.as_deref().unwrap_or(default_base_url)
}

/// 该 provider 类型开箱可用的默认 API Base URL，供设置表单预填。
///
/// 返回 `None` 表示没有通用默认值（自建 / 代理类服务、企业专属端点），
/// 必须由用户显式填写。这里的取值与 [`client_from_config`] 的兜底地址同源，
/// 由测试 `preset_provider_types_build_client_without_explicit_base_url` 守住一致性。
pub fn default_api_base(provider_type: ProviderType) -> Option<&'static str> {
    match provider_type {
        ProviderType::OpenAI => Some(OPENAI_BASE_URL),
        ProviderType::Anthropic => Some(ANTHROPIC_BASE_URL),
        ProviderType::Aliyun => Some(ALIYUN_BASE_URL),
        ProviderType::Zhipu => Some(ZHIPU_BASE_URL),
        ProviderType::Ollama => Some(OLLAMA_BASE_URL),
        ProviderType::Volcengine => Some(VOLCENGINE_BASE_URL),
        ProviderType::Moonshot => Some(MOONSHOT_BASE_URL),
        ProviderType::DeepSeek => Some(DEEPSEEK_BASE_URL),
        ProviderType::Google => Some(GOOGLE_BASE_URL),
        ProviderType::OpenRouter => Some(OPENROUTER_BASE_URL),
        ProviderType::MiniMax => Some(MINIMAX_BASE_URL),
        ProviderType::SiliconFlow => Some(SILICONFLOW_BASE_URL),
        ProviderType::TencentHunyuan => Some(HUNYUAN_BASE_URL),
        ProviderType::Xiaomi => Some(XIAOMI_BASE_URL),
        ProviderType::Xinference => Some(XINFERENCE_BASE_URL),
        ProviderType::AzureOpenAI | ProviderType::OpenAICompatible => None,
    }
}

fn aliyun_base_url(config: &ProviderConfig) -> &str {
    config
        .api_base
        .as_deref()
        .unwrap_or(ALIYUN_COMPATIBLE_BASE_URL)
}

fn aliyun_prefers_compatible_mode(config: &ProviderConfig) -> bool {
    config.model.starts_with("qwen3.5-")
        || config
            .api_base
            .as_deref()
            .map(|base_url| base_url.contains("/compatible-mode/"))
            .unwrap_or(false)
}

#[async_trait]
impl LlmProvider for LlmConnector {
    async fn chat(&self, request: &ChatRequest) -> Result<String> {
        let response = self.client.chat(request).await?;
        Ok(response.content)
    }

    async fn chat_completion(&self, request: &ChatRequest) -> Result<ChatResponse> {
        Ok(self.client.chat(request).await?)
    }

    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream> {
        let stream = self.client.chat_stream(request).await?;
        Ok(Box::pin(futures::stream::StreamExt::map(
            stream,
            |result| result.map_err(|e| anyhow::anyhow!("{}", e)),
        )))
    }

    async fn models(&self) -> Result<Vec<String>> {
        let models = self.client.models().await?;
        Ok(models)
    }

    fn provider_name(&self) -> &str {
        self.provider_type.as_str()
    }
}

pub fn create_message(role: Role, content: impl Into<String>) -> Message {
    Message::text(role, content)
}

pub fn user_message(content: impl Into<String>) -> Message {
    create_message(Role::User, content)
}

pub fn assistant_message(content: impl Into<String>) -> Message {
    create_message(Role::Assistant, content)
}

pub fn system_message(content: impl Into<String>) -> Message {
    create_message(Role::System, content)
}

#[cfg(test)]
mod tests {
    use super::*;
    use llm_connector::types::ChatResponse;

    struct TextOnlyProvider;

    #[async_trait]
    impl LlmProvider for TextOnlyProvider {
        async fn chat(&self, _request: &ChatRequest) -> Result<String> {
            Ok("plain answer".to_string())
        }

        async fn chat_stream(&self, _request: &ChatRequest) -> Result<ChatStream> {
            Ok(Box::pin(futures::stream::empty()))
        }

        async fn models(&self) -> Result<Vec<String>> {
            Ok(vec!["text-only".to_string()])
        }

        fn provider_name(&self) -> &str {
            "text-only"
        }
    }

    #[tokio::test]
    async fn provider_default_chat_completion_wraps_text_response() {
        let provider = TextOnlyProvider;
        let request = ChatRequest {
            model: "text-only".to_string(),
            messages: vec![Message::text(Role::User, "hello")],
            ..Default::default()
        };

        let response: ChatResponse = provider
            .chat_completion(&request)
            .await
            .expect("default chat_completion should call chat");

        assert_eq!("plain answer", response.content);
        assert!(!response.has_tool_calls());
    }

    #[test]
    fn provider_base_url_prefers_configured_value() {
        let config = ProviderConfig {
            api_base: Some("https://custom.example.com".to_string()),
            ..Default::default()
        };

        assert_eq!(
            provider_base_url(&config, OPENAI_BASE_URL),
            "https://custom.example.com"
        );
    }

    #[test]
    fn provider_base_url_uses_default_when_config_missing() {
        let config = ProviderConfig::default();

        assert_eq!(provider_base_url(&config, OLLAMA_BASE_URL), OLLAMA_BASE_URL);
    }

    #[test]
    fn aliyun_prefers_compatible_mode_for_qwen35_models() {
        let config = ProviderConfig {
            provider_type: ProviderType::Aliyun,
            model: "qwen3.5-plus".to_string(),
            ..Default::default()
        };

        assert!(aliyun_prefers_compatible_mode(&config));
        assert_eq!(aliyun_base_url(&config), ALIYUN_COMPATIBLE_BASE_URL);
    }

    #[test]
    fn aliyun_prefers_compatible_mode_for_explicit_compatible_base_url() {
        let config = ProviderConfig {
            provider_type: ProviderType::Aliyun,
            api_base: Some("https://dashscope.aliyuncs.com/compatible-mode/v1".to_string()),
            model: "qwen-plus".to_string(),
            ..Default::default()
        };

        assert!(aliyun_prefers_compatible_mode(&config));
        assert_eq!(
            aliyun_base_url(&config),
            "https://dashscope.aliyuncs.com/compatible-mode/v1"
        );
    }

    #[test]
    fn aliyun_keeps_private_protocol_for_non_compatible_models() {
        let config = ProviderConfig {
            provider_type: ProviderType::Aliyun,
            api_base: Some("https://dashscope.aliyuncs.com".to_string()),
            model: "qwen-plus".to_string(),
            ..Default::default()
        };

        assert!(!aliyun_prefers_compatible_mode(&config));
    }

    fn assert_invalid_proxy_error(result: Result<LlmConnector>) {
        let error = match result {
            Ok(_) => panic!("无效代理地址应阻止 LLM client 构建"),
            Err(error) => error,
        };
        assert!(
            error.to_string().contains("Invalid proxy URL"),
            "unexpected error: {error:#}"
        );
    }

    #[test]
    fn connector_proxy_rejects_invalid_proxy_for_openai_builder_path() {
        let config = ProviderConfig {
            provider_type: ProviderType::OpenAI,
            api_key: Some("sk-test".to_string()),
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        };

        assert_invalid_proxy_error(LlmConnector::from_config_with_proxy(
            &config,
            Some("http://"),
        ));
    }

    #[test]
    fn connector_proxy_rejects_invalid_proxy_for_openai_compatible_path() {
        let config = ProviderConfig {
            provider_type: ProviderType::OpenAICompatible,
            name: "custom".to_string(),
            api_key: Some("sk-test".to_string()),
            api_base: Some("https://llm.example.com/v1".to_string()),
            model: "custom-model".to_string(),
            ..Default::default()
        };

        assert_invalid_proxy_error(LlmConnector::from_config_with_proxy(
            &config,
            Some("http://"),
        ));
    }

    #[test]
    fn connector_proxy_rejects_invalid_proxy_for_azure_openai_path() {
        let config = ProviderConfig {
            provider_type: ProviderType::AzureOpenAI,
            api_key: Some("sk-test".to_string()),
            api_base: Some("https://example.openai.azure.com".to_string()),
            api_version: Some("2024-02-15-preview".to_string()),
            model: "gpt-4o-mini".to_string(),
            ..Default::default()
        };

        assert_invalid_proxy_error(LlmConnector::from_config_with_proxy(
            &config,
            Some("http://"),
        ));
    }

    #[test]
    fn connector_proxy_rejects_invalid_proxy_for_ollama_builder_path() {
        let config = ProviderConfig {
            provider_type: ProviderType::Ollama,
            model: "llama3.2".to_string(),
            ..Default::default()
        };

        assert_invalid_proxy_error(LlmConnector::from_config_with_proxy(
            &config,
            Some("http://"),
        ));
    }

    /// 新增供应商预置的默认地址。锁定取值，避免以后被无意改掉。
    #[test]
    fn added_providers_expose_preset_api_base_urls() {
        assert_eq!(
            Some("https://openrouter.ai/api/v1"),
            default_api_base(ProviderType::OpenRouter)
        );
        assert_eq!(
            Some("https://api.minimaxi.com/v1"),
            default_api_base(ProviderType::MiniMax)
        );
        assert_eq!(
            Some("https://api.siliconflow.cn/v1"),
            default_api_base(ProviderType::SiliconFlow)
        );
        assert_eq!(
            Some("https://api.hunyuan.cloud.tencent.com/v1"),
            default_api_base(ProviderType::TencentHunyuan)
        );
        assert_eq!(
            Some("https://api.xiaomimimo.com/v1"),
            default_api_base(ProviderType::Xiaomi)
        );
        assert_eq!(
            Some("http://127.0.0.1:9997/v1"),
            default_api_base(ProviderType::Xinference)
        );
    }

    /// 用户必须显式提供端点的类型不得有预置值，否则设置页会预填出一个错误地址。
    #[test]
    fn explicit_endpoint_providers_have_no_preset_api_base_url() {
        assert_eq!(None, default_api_base(ProviderType::OpenAICompatible));
        assert_eq!(None, default_api_base(ProviderType::AzureOpenAI));
    }

    /// 契约测试：凡是声明了预置地址的类型，都必须能在用户不填 base_url 的情况下建出 client。
    /// 这条同时守住「预置地址」与 `client_from_config` 分支的一致性。
    #[test]
    fn preset_provider_types_build_client_without_explicit_base_url() {
        let mut checked = 0;

        for provider_type in ProviderType::all() {
            if default_api_base(provider_type).is_none() {
                continue;
            }

            let config = ProviderConfig {
                provider_type,
                api_key: Some("test-key".to_string()),
                model: "test-model".to_string(),
                ..Default::default()
            };

            assert!(
                LlmConnector::from_config(&config).is_ok(),
                "{} 声明了预置地址却无法在未填 base_url 时构建 client",
                provider_type.as_str()
            );
            checked += 1;
        }

        // 防止测试因 all() 被清空而空转通过。
        assert!(checked >= 13, "只校验到 {checked} 个类型，疑似漏配");
    }
}
