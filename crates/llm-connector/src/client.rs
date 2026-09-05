//! V2 Unified Client - Next-generation LLM client interface
//!
//! This module provides a unified client interface supporting all LLM service providers.

use crate::core::Provider;
use crate::error::LlmConnectorError;
use crate::types::{
    ChatRequest, ChatResponse, EmbedRequest, EmbedResponse, ResponsesRequest, ResponsesResponse,
};
use std::sync::Arc;

#[cfg(feature = "streaming")]
use crate::types::{ChatStream, ResponsesStream};

/// Unified LLM Client
///
/// This client provides a unified interface to access various LLM services,
/// using V2 architecture's clean abstraction layer.
///
/// # Example
/// ```rust,no_run
/// use llm_connector::{LlmClient, types::{ChatRequest, Message, Role}};
///
/// #[tokio::main]
/// async fn main() -> Result<(), Box<dyn std::error::Error>> {
///     // Create OpenAI client
///     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
///
///     // Create request
///     let request = ChatRequest {
///         model: "gpt-4".to_string(),
///         messages: vec![Message::text(Role::User, "Hello, how are you?")],
///         ..Default::default()
///     };
///
///     // Send request
///     let response = client.chat(&request).await?;
///     println!("Response: {}", response.content);
///
///     Ok(())
/// }
/// ```
pub struct LlmClient {
    provider: Arc<dyn Provider>,
}

impl LlmClient {
    /// Create a builder for fluent client construction
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::builder()
    ///     .openai("sk-...")
    ///     .base_url("https://api.deepseek.com")
    ///     .timeout(60)
    ///     .build()
    ///     .unwrap();
    /// ```
    pub fn builder() -> crate::builder::LlmClientBuilder {
        crate::builder::LlmClientBuilder::new()
    }

    /// Create client from any Provider
    pub fn from_provider(provider: Arc<dyn Provider>) -> Self {
        Self { provider }
    }

    /// Create OpenAI client
    ///
    /// # Parameters
    /// - `api_key`: OpenAI API key
    /// - `base_url`: Base URL (e.g., "https://api.openai.com/v1")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::openai("sk-...", "https://api.openai.com/v1").unwrap();
    /// ```
    pub fn openai(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Azure OpenAI client
    ///
    /// # Parameters
    /// - `api_key`: Azure OpenAI API key
    /// - `endpoint`: Azure OpenAI endpoint
    /// - `api_version`: API version
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::azure_openai(
    ///     "your-api-key",
    ///     "https://your-resource.openai.azure.com",
    ///     "2024-02-15-preview"
    /// ).unwrap();
    /// ```
    pub fn azure_openai(
        api_key: &str,
        endpoint: &str,
        api_version: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::azure_openai(api_key, endpoint, api_version)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Aliyun DashScope client
    ///
    /// # Parameters
    /// - `api_key`: Aliyun DashScope API key
    /// - `base_url`: Base URL (e.g., "https://dashscope.aliyuncs.com")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::aliyun("sk-...", "https://dashscope.aliyuncs.com").unwrap();
    /// ```
    pub fn aliyun(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::aliyun(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Anthropic Claude client
    ///
    /// # Parameters
    /// - `api_key`: Anthropic API key (Format: sk-ant-...)
    /// - `base_url`: Base URL (e.g., "https://api.anthropic.com")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::anthropic("sk-ant-...", "https://api.anthropic.com").unwrap();
    /// ```
    pub fn anthropic(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::anthropic(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Zhipu GLM client
    ///
    /// # Parameters
    /// - `api_key`: Zhipu GLM API key
    /// - `base_url`: Base URL (e.g., "https://open.bigmodel.cn")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::zhipu("your-api-key", "https://open.bigmodel.cn").unwrap();
    /// ```
    pub fn zhipu(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::zhipu(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Zhipu GLM client (OpenAI compatible mode)
    ///
    /// # Parameters
    /// - `api_key`: Zhipu GLM API key
    /// - `base_url`: Base URL (e.g., "https://open.bigmodel.cn")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::zhipu_openai_compatible("your-api-key", "https://open.bigmodel.cn").unwrap();
    /// ```
    pub fn zhipu_openai_compatible(
        api_key: &str,
        base_url: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::zhipu_openai_compatible(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Ollama client
    ///
    /// # Parameters
    /// - `base_url`: Ollama service URL (e.g., "http://localhost:11434")
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::ollama("http://localhost:11434").unwrap();
    /// ```
    pub fn ollama(base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::ollama(base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Xinference client (OpenAI-compatible API, no auth by default)
    ///
    /// # Parameters
    /// - `base_url`: Xinference OpenAI-compatible base URL (e.g., "http://127.0.0.1:9997/v1")
    pub fn xinference(base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::xinference(base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Xinference client with API key
    pub fn xinference_with_api_key(
        api_key: &str,
        base_url: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::xinference_with_api_key(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create OpenAI-compatible service client
    ///
    /// # Parameters
    /// - `api_key`: API key
    /// - `base_url`: Service base URL
    /// - `service_name`: Service name
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// // DeepSeek
    /// let deepseek = LlmClient::openai_compatible(
    ///     "sk-...",
    ///     "https://api.deepseek.com",
    ///     "deepseek"
    /// ).unwrap();
    ///
    /// // Moonshot
    /// let moonshot = LlmClient::openai_compatible(
    ///     "sk-...",
    ///     "https://api.moonshot.cn",
    ///     "moonshot"
    /// ).unwrap();
    ///
    /// // LongCat (OpenAI format)
    /// let longcat = LlmClient::openai_compatible(
    ///     "ak_...",
    ///     "https://api.longcat.chat/openai",
    ///     "longcat"
    /// ).unwrap();
    /// ```
    pub fn openai_compatible(
        api_key: &str,
        base_url: &str,
        service_name: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible(api_key, base_url, service_name)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create LongCat Anthropic format client
    ///
    /// LongCat's Anthropic endpoint uses Bearer authentication instead of standard x-api-key authentication
    ///
    /// # Parameters
    /// - `api_key`: LongCat API key (Format: ak_...)
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::longcat_anthropic("ak_...", "https://api.longcat.chat/anthropic").unwrap();
    /// ```
    pub fn longcat_anthropic(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::anthropic_compatible(api_key, base_url, "longcat")?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create LongCat Anthropic client with custom configuration
    pub fn longcat_anthropic_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        // Longcat uses Bearer auth, so we use a custom HttpClient construction here or a specialized helper
        let protocol = crate::protocols::AnthropicProtocol::new(api_key);
        let client = crate::core::HttpClient::with_config(base_url, timeout_secs, proxy)?
            .with_header("Authorization".to_string(), format!("Bearer {}", api_key))
            .with_header("anthropic-version".to_string(), "2023-06-01".to_string());

        let provider = crate::core::GenericProvider::new(protocol, client);
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Volcengine client
    ///
    /// Volcengine uses OpenAI compatible API format, but with different endpoint paths
    ///
    /// # Parameters
    /// - `api_key`: Volcengine API key (UUID format)
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::volcengine("your-volcengine-api-key", "https://ark.cn-beijing.volces.com/api/v3").unwrap();
    /// ```
    pub fn volcengine(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible(api_key, base_url, "volcengine")?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Volcengine client with custom configuration
    pub fn volcengine_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::openai_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Tencent Hunyuan client
    ///
    /// Tencent Hunyuan uses Native API v3 (TC3-HMAC-SHA256)
    ///
    /// # Parameters
    /// - `secret_id`: Tencent Cloud SecretID
    /// - `secret_key`: Tencent Cloud SecretKey
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::tencent("AKID...", "SecretKey...", "hunyuan.tencentcloudapi.com").unwrap();
    /// ```
    #[cfg(feature = "tencent")]
    pub fn tencent(
        secret_id: &str,
        secret_key: &str,
        base_url: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::tencent(secret_id, secret_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Tencent Hunyuan client with custom configuration
    #[cfg(feature = "tencent")]
    pub fn tencent_with_config(
        secret_id: &str,
        secret_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::tencent_with_config(
            secret_id,
            secret_key,
            base_url,
            timeout_secs,
            proxy,
        )?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Moonshot（Moonshot）client
    ///
    /// Moonshot uses OpenAI-compatible API format
    ///
    /// # Parameters
    /// - `api_key`: Moonshot API key (format: sk-...)
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::moonshot("sk-...", "https://api.moonshot.cn/v1").unwrap();
    /// ```
    pub fn moonshot(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible(api_key, base_url, "moonshot")?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Moonshot client with custom configuration
    pub fn moonshot_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible_with_config(
            api_key,
            base_url,
            "moonshot",
            timeout_secs,
            proxy,
        )?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create DeepSeek client
    ///
    /// DeepSeek uses OpenAI-compatible API format, supports reasoning models
    ///
    /// # Parameters
    /// - `api_key`: DeepSeek API key (format: sk-...)
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::deepseek("sk-...", "https://api.deepseek.com").unwrap();
    /// ```
    pub fn deepseek(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible(api_key, base_url, "deepseek")?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create DeepSeek client with custom configuration
    pub fn deepseek_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible_with_config(
            api_key,
            base_url,
            "deepseek",
            timeout_secs,
            proxy,
        )?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Xiaomi MiMo client
    ///
    /// Xiaomi MiMo uses OpenAI-compatible API format
    ///
    /// # Parameters
    /// - `api_key`: Xiaomi MiMo API key
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::xiaomi("your-api-key", "https://api.xiaomimimo.com/v1").unwrap();
    /// ```
    pub fn xiaomi(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::openai_compatible(api_key, base_url, "xiaomi")?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Xiaomi MiMo client with custom configuration
    pub fn xiaomi_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::openai_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    // ============================================================================
    // Advanced Constructors - Custom Configuration
    // ============================================================================

    /// Create OpenAI client with custom configuration
    pub fn openai_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::openai_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Aliyun client with custom configuration
    pub fn aliyun_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::aliyun_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Aliyun international client
    pub fn aliyun_international(api_key: &str, region: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::aliyun_international(api_key, region)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Aliyun private cloud client
    pub fn aliyun_private(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::aliyun_private(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Anthropic client with custom configuration
    pub fn anthropic_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::anthropic_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Anthropic Vertex AI client
    pub fn anthropic_vertex(
        project_id: &str,
        location: &str,
        access_token: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::anthropic_vertex(project_id, location, access_token)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Anthropic AWS Bedrock client
    pub fn anthropic_bedrock(
        region: &str,
        access_key: &str,
        secret_key: &str,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::anthropic_bedrock(region, access_key, secret_key)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Zhipu client with custom configuration
    pub fn zhipu_with_config(
        api_key: &str,
        openai_compatible: bool,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::zhipu_with_config(
            api_key,
            openai_compatible,
            base_url,
            timeout_secs,
            proxy,
        )?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Zhipu enterprise client
    pub fn zhipu_enterprise(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::zhipu_enterprise(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Ollama client with custom configuration
    pub fn ollama_with_config(
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::ollama_with_config(base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Xinference client with custom configuration (no auth by default)
    pub fn xinference_with_config(
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::xinference_with_config(base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Google client
    ///
    /// # Parameters
    /// - `api_key`: Google API key
    /// - `base_url`: Base URL (e.g., "https://generativelanguage.googleapis.com/v1beta")
    pub fn google(api_key: &str, base_url: &str) -> Result<Self, LlmConnectorError> {
        let provider = crate::providers::google(api_key, base_url)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Create Google client with custom configuration
    pub fn google_with_config(
        api_key: &str,
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let provider =
            crate::providers::google_with_config(api_key, base_url, timeout_secs, proxy)?;
        Ok(Self::from_provider(Arc::new(provider)))
    }

    /// Get provider name
    pub fn provider_name(&self) -> &str {
        self.provider.name()
    }

    pub fn supported_providers() -> &'static [&'static str] {
        &[
            "openai",
            "aliyun",
            "anthropic",
            "zhipu",
            "ollama",
            "xinference",
            "volcengine",
            "longcat_anthropic",
            "azure_openai",
            "openai_compatible",
            "google",
            "xiaomi",
            #[cfg(feature = "tencent")]
            "tencent",
        ]
    }

    // ...
    /// # Returns
    /// Chat response
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    /// use llm_connector::types::{ChatRequest, Message};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
    ///
    ///     let request = ChatRequest {
    ///         model: "gpt-4".to_string(),
    ///         messages: vec![Message::user("Hello!")],
    ///         ..Default::default()
    ///     };
    ///
    ///     let response = client.chat(&request).await?;
    ///     println!("Response: {}", response.content);
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError> {
        self.provider.chat(request).await
    }

    /// Send OpenAI Responses API request
    pub async fn invoke_responses(
        &self,
        request: &ResponsesRequest,
    ) -> Result<ResponsesResponse, LlmConnectorError> {
        self.provider.invoke_responses(request).await
    }

    /// Send streaming chat completion request
    ///
    /// # Parameters
    /// - `request`: Chat request
    ///
    /// # Returns
    /// Chat stream
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    /// use llm_connector::types::{ChatRequest, Message};
    /// use futures_util::StreamExt;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
    ///
    ///     let request = ChatRequest {
    ///         model: "gpt-4".to_string(),
    ///         messages: vec![Message::user("Hello!")],
    ///         stream: Some(true),
    ///         ..Default::default()
    ///     };
    ///
    ///     let mut stream = client.chat_stream(&request).await?;
    ///     while let Some(chunk) = stream.next().await {
    ///         let chunk = chunk?;
    ///         if let Some(content) = chunk.get_content() {
    ///             print!("{}", content);
    ///         }
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    #[cfg(feature = "streaming")]
    pub async fn chat_stream(
        &self,
        request: &ChatRequest,
    ) -> Result<ChatStream, LlmConnectorError> {
        self.provider.chat_stream(request).await
    }

    /// Send streaming OpenAI Responses API request
    #[cfg(feature = "streaming")]
    pub async fn invoke_responses_stream(
        &self,
        request: &ResponsesRequest,
    ) -> Result<ResponsesStream, LlmConnectorError> {
        self.provider.invoke_responses_stream(request).await
    }

    /// Generate embeddings
    ///
    /// # Parameters
    /// - `request`: Embedding request
    ///
    /// # Returns
    /// Embedding response
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    /// use llm_connector::types::EmbedRequest;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
    ///
    ///     let request = EmbedRequest::new(
    ///         "text-embedding-3-small",
    ///         "Hello, world!"
    ///     );
    ///
    ///     let response = client.embed(&request).await?;
    ///     println!("Embedding length: {}", response.data[0].embedding.len());
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn embed(&self, request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError> {
        self.provider.embed(request).await
    }

    /// Get available models list
    ///
    /// # Returns
    /// List of model names
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = LlmClient::openai("sk-...", "https://api.openai.com/v1")?;
    ///
    ///     let models = client.models().await?;
    ///     for model in models {
    ///         println!("Available model: {}", model);
    ///     }
    ///
    ///     Ok(())
    /// }
    /// ```
    pub async fn models(&self) -> Result<Vec<String>, LlmConnectorError> {
        self.provider.models().await
    }

    /// Get reference to underlying provider (for special feature access)
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// let client = LlmClient::openai("sk-...", "https://api.openai.com/v1").unwrap();
    /// let provider = client.provider();
    ///
    /// // Can perform type conversion to access provider-specific features
    /// ```
    pub fn provider(&self) -> &dyn Provider {
        self.provider.as_ref()
    }

    pub fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        self.provider.capabilities()
    }

    // ============================================================================
    // Type-safe Provider conversion methods
    // ============================================================================

    /// Try to convert client to OllamaProvider
    ///
    /// # Returns
    /// If underlying Provider is OllamaProvider, returns Some reference, otherwise returns None
    ///
    /// # Example
    /// ```rust,no_run
    /// use llm_connector::LlmClient;
    ///
    /// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
    /// let client = LlmClient::ollama("http://localhost:11434")?;
    /// if let Some(_ollama) = client.as_ollama() {
    ///     // Can access Ollama-specific features
    /// }
    /// # Ok(())
    /// # }
    /// ```
    pub fn as_ollama(&self) -> Option<&crate::providers::OllamaProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::OllamaProvider>()
    }

    /// Try to convert client to OpenAIProvider
    pub fn as_openai(&self) -> Option<&crate::providers::OpenAIProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::OpenAIProvider>()
    }

    /// Try to convert client to AliyunProvider
    pub fn as_aliyun(&self) -> Option<&crate::providers::AliyunProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::AliyunProvider>()
    }

    /// Try to convert client to AnthropicProvider
    pub fn as_anthropic(&self) -> Option<&crate::providers::AnthropicProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::AnthropicProvider>()
    }

    /// Try to convert client to ZhipuProvider
    pub fn as_zhipu(&self) -> Option<&crate::providers::ZhipuProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::ZhipuProvider>()
    }

    /// Create a mock client for testing (no real API calls)
    ///
    /// # Parameters
    /// - `content`: The content string the mock will always return
    ///
    /// # Example
    /// ```rust
    /// use llm_connector::{LlmClient, ChatRequest, Message};
    ///
    /// #[tokio::main]
    /// async fn main() -> Result<(), Box<dyn std::error::Error>> {
    ///     let client = LlmClient::mock("Hello from mock!");
    ///
    ///     let request = ChatRequest::new("any-model")
    ///         .add_message(Message::user("Hi"));
    ///
    ///     let response = client.chat(&request).await?;
    ///     assert_eq!(response.content, "Hello from mock!");
    ///     Ok(())
    /// }
    /// ```
    pub fn mock(content: impl Into<String>) -> Self {
        let provider = crate::providers::mock::MockProvider::new(content);
        Self::from_provider(Arc::new(provider))
    }

    /// Try to convert client to MockProvider (for test assertions)
    pub fn as_mock(&self) -> Option<&crate::providers::mock::MockProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::mock::MockProvider>()
    }

    /// Try to convert client to GoogleProvider
    pub fn as_google(&self) -> Option<&crate::providers::GoogleProvider> {
        self.provider
            .as_any()
            .downcast_ref::<crate::providers::GoogleProvider>()
    }
}

impl Clone for LlmClient {
    fn clone(&self) -> Self {
        Self {
            provider: Arc::clone(&self.provider),
        }
    }
}

impl std::fmt::Debug for LlmClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LlmClient")
            .field("provider", &self.provider.name())
            .finish()
    }
}
