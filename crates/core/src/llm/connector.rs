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
const LLM_CLIENT_TIMEOUT_SECS: u64 = 120;

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
        let client = client_from_config(config, proxy_url)?;

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

fn client_from_config(config: &ProviderConfig, proxy_url: Option<&str>) -> Result<LlmClient> {
    match config.provider_type {
        ProviderType::OpenAI => build_client(
            LlmClient::builder().openai(required_api_key(config, "OpenAI")?),
            provider_base_url(config, OPENAI_BASE_URL),
            proxy_url,
        ),
        ProviderType::Anthropic => build_client(
            LlmClient::builder().anthropic(required_api_key(config, "Anthropic")?),
            provider_base_url(config, ANTHROPIC_BASE_URL),
            proxy_url,
        ),
        ProviderType::Aliyun => aliyun_client(config, proxy_url),
        ProviderType::Zhipu => build_client(
            LlmClient::builder().zhipu(required_api_key(config, "Zhipu")?),
            provider_base_url(config, ZHIPU_BASE_URL),
            proxy_url,
        ),
        ProviderType::Ollama => build_client(
            LlmClient::builder().ollama(),
            provider_base_url(config, OLLAMA_BASE_URL),
            proxy_url,
        ),
        ProviderType::Volcengine => build_client(
            LlmClient::builder().volcengine(required_api_key(config, "Volcengine")?),
            provider_base_url(config, VOLCENGINE_BASE_URL),
            proxy_url,
        ),
        ProviderType::Moonshot => build_client(
            LlmClient::builder().moonshot(required_api_key(config, "Moonshot")?),
            provider_base_url(config, MOONSHOT_BASE_URL),
            proxy_url,
        ),
        ProviderType::DeepSeek => build_client(
            LlmClient::builder().deepseek(required_api_key(config, "DeepSeek")?),
            provider_base_url(config, DEEPSEEK_BASE_URL),
            proxy_url,
        ),
        ProviderType::Google => build_client(
            LlmClient::builder().google(required_api_key(config, "Google")?),
            provider_base_url(config, GOOGLE_BASE_URL),
            proxy_url,
        ),
        ProviderType::AzureOpenAI => azure_openai_client(config, proxy_url),
        ProviderType::OpenAICompatible => openai_compatible_client(
            required_api_key(config, "OpenAI Compatible")?,
            required_base_url(config, "OpenAI Compatible")?,
            &config.name,
            proxy_url,
        ),
        ProviderType::OnetCli => {
            anyhow::bail!(
                "OnetCli provider should be created via ProviderManager, not LlmConnector"
            )
        }
    }
}

fn build_client(
    mut builder: LlmClientBuilder,
    base_url: &str,
    proxy_url: Option<&str>,
) -> Result<LlmClient> {
    builder = builder.base_url(base_url).timeout(LLM_CLIENT_TIMEOUT_SECS);
    if let Some(proxy_url) = proxy_url {
        builder = builder.proxy(proxy_url);
    }
    Ok(builder.build()?)
}

fn aliyun_client(config: &ProviderConfig, proxy_url: Option<&str>) -> Result<LlmClient> {
    let api_key = required_api_key(config, "Aliyun")?;
    if aliyun_prefers_compatible_mode(config) {
        openai_compatible_client(api_key, aliyun_base_url(config), &config.name, proxy_url)
    } else {
        build_client(
            LlmClient::builder().aliyun(api_key),
            provider_base_url(config, ALIYUN_BASE_URL),
            proxy_url,
        )
    }
}

fn openai_compatible_client(
    api_key: &str,
    base_url: &str,
    service_name: &str,
    proxy_url: Option<&str>,
) -> Result<LlmClient> {
    if let Some(proxy_url) = proxy_url {
        let provider = llm_connector::providers::openai_compatible_with_config(
            api_key,
            base_url,
            service_name,
            Some(LLM_CLIENT_TIMEOUT_SECS),
            Some(proxy_url),
        )?;
        return Ok(LlmClient::from_provider(Arc::new(provider)));
    }

    build_client(
        LlmClient::builder().openai_compatible(api_key, service_name),
        base_url,
        None,
    )
}

fn azure_openai_client(config: &ProviderConfig, proxy_url: Option<&str>) -> Result<LlmClient> {
    let api_key = required_api_key(config, "Azure OpenAI")?;
    let endpoint = required_base_url(config, "Azure OpenAI")?;
    let api_version = config
        .api_version
        .as_deref()
        .unwrap_or("2024-02-15-preview");

    if let Some(proxy_url) = proxy_url {
        let protocol = OpenAIProtocol::with_service(api_key, "azure-openai");
        let client =
            HttpClient::with_config(endpoint, Some(LLM_CLIENT_TIMEOUT_SECS), Some(proxy_url))?
                .with_header("api-key".to_string(), api_key.to_string())
                .with_header("api-version".to_string(), api_version.to_string());
        let provider = GenericProvider::new(protocol, client);
        return Ok(LlmClient::from_provider(Arc::new(provider)));
    }

    build_client(
        LlmClient::builder().azure_openai(api_key, endpoint, api_version),
        endpoint,
        None,
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
}
