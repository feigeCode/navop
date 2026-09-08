//! V2 Service Provider Module
//!
//! This module contains all service provider implementations.
//! It focuses on providing a unified way to create and use different LLM services.

use crate::core::{GenericProvider, HttpClient, Protocol, Provider};
use crate::error::LlmConnectorError;
#[cfg(feature = "tencent")]
use crate::protocols::TencentNativeProtocol;
#[cfg(feature = "tencent")]
use crate::protocols::common::capabilities::ProviderCapabilities;
use crate::protocols::{
    AliyunProtocol, AnthropicProtocol, GoogleProtocol, OllamaProtocol, OpenAIProtocol,
    ZhipuProtocol,
};
use crate::types::{ChatRequest, ChatResponse, EmbedRequest, EmbedResponse};
use async_trait::async_trait;
#[cfg(feature = "tencent")]
use chrono::Utc;
use std::any::Any;
use std::collections::HashMap;

#[cfg(feature = "streaming")]
use crate::types::ChatStream;

// ============================================================================
// OpenAI Provider
// ============================================================================

pub type OpenAIProvider = GenericProvider<OpenAIProtocol>;

pub fn openai(api_key: &str, base_url: &str) -> Result<OpenAIProvider, LlmConnectorError> {
    openai_with_config(api_key, base_url, None, None)
}

pub fn openai_with_config(
    api_key: &str,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<OpenAIProvider, LlmConnectorError> {
    let protocol = OpenAIProtocol::with_service(api_key, "openai");
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?;
    let auth_headers: HashMap<String, String> = protocol.auth_headers().into_iter().collect();
    let client = client.with_headers(auth_headers);
    Ok(GenericProvider::new(protocol, client))
}

pub fn azure_openai(
    api_key: &str,
    endpoint: &str,
    api_version: &str,
) -> Result<OpenAIProvider, LlmConnectorError> {
    let protocol = OpenAIProtocol::with_service(api_key, "azure-openai");
    let client = HttpClient::new(endpoint)?
        .with_header("api-key".to_string(), api_key.to_string())
        .with_header("api-version".to_string(), api_version.to_string());
    Ok(GenericProvider::new(protocol, client))
}

pub fn openai_compatible(
    api_key: &str,
    base_url: &str,
    service_name: &str,
) -> Result<OpenAIProvider, LlmConnectorError> {
    openai_compatible_with_config(api_key, base_url, service_name, None, None)
}

pub fn openai_compatible_with_config(
    api_key: &str,
    base_url: &str,
    service_name: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<OpenAIProvider, LlmConnectorError> {
    let protocol = OpenAIProtocol::with_service(api_key, service_name);
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?
        .with_header("Authorization".to_string(), format!("Bearer {}", api_key))
        .with_header(
            "User-Agent".to_string(),
            format!("llm-connector/{}", service_name),
        );
    Ok(GenericProvider::new(protocol, client))
}

pub fn xinference(base_url: &str) -> Result<OpenAIProvider, LlmConnectorError> {
    xinference_with_config(base_url, None, None)
}

pub fn xinference_with_api_key(
    api_key: &str,
    base_url: &str,
) -> Result<OpenAIProvider, LlmConnectorError> {
    openai_with_config(api_key, base_url, None, None)
}

pub fn xinference_with_config(
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<OpenAIProvider, LlmConnectorError> {
    // Xinference local deployments commonly run without auth.
    // Keep protocol shape OpenAI-compatible but do not inject Authorization by default.
    let protocol = OpenAIProtocol::with_service("", "xinference");
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?;
    Ok(GenericProvider::new(protocol, client))
}

pub fn validate_openai_key(api_key: &str) -> bool {
    api_key.starts_with("sk-") && api_key.len() > 20
}

// ============================================================================
// Aliyun Provider
// ============================================================================

pub type AliyunProvider = GenericProvider<AliyunProtocol>;

pub fn aliyun(api_key: &str, base_url: &str) -> Result<AliyunProvider, LlmConnectorError> {
    aliyun_with_config(api_key, base_url, None, None)
}

pub fn aliyun_with_config(
    api_key: &str,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<AliyunProvider, LlmConnectorError> {
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?
        .with_header("Authorization".to_string(), format!("Bearer {}", api_key));
    Ok(GenericProvider::new(AliyunProtocol::new(api_key), client))
}

pub fn aliyun_international(
    api_key: &str,
    region: &str,
) -> Result<AliyunProvider, LlmConnectorError> {
    aliyun(
        api_key,
        &format!("https://dashscope.{}.aliyuncs.com/api/v1", region),
    )
}

pub fn aliyun_private(api_key: &str, base_url: &str) -> Result<AliyunProvider, LlmConnectorError> {
    aliyun(api_key, base_url)
}

pub fn validate_aliyun_key(api_key: &str) -> bool {
    api_key.starts_with("sk-") && api_key.len() > 20
}

// ============================================================================
// Anthropic Provider
// ============================================================================

pub type AnthropicProvider = GenericProvider<AnthropicProtocol>;

pub fn anthropic(api_key: &str, base_url: &str) -> Result<AnthropicProvider, LlmConnectorError> {
    anthropic_with_config(api_key, base_url, None, None)
}

pub fn anthropic_with_config(
    api_key: &str,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<AnthropicProvider, LlmConnectorError> {
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?
        .with_header("x-api-key".to_string(), api_key.to_string())
        .with_header("anthropic-version".to_string(), "2023-06-01".to_string());
    Ok(GenericProvider::new(
        AnthropicProtocol::new(api_key),
        client,
    ))
}

pub fn anthropic_vertex(
    project_id: &str,
    location: &str,
    access_token: &str,
) -> Result<AnthropicProvider, LlmConnectorError> {
    let protocol = AnthropicProtocol::new(""); // Vertex AI does not require Anthropic API key
    let base_url = format!(
        "https://{}-aiplatform.googleapis.com/v1/projects/{}/locations/{}/publishers/anthropic",
        location, project_id, location
    );
    let client = HttpClient::new(&base_url)?.with_header(
        "Authorization".to_string(),
        format!("Bearer {}", access_token),
    );
    Ok(GenericProvider::new(protocol, client))
}

pub fn anthropic_bedrock(
    region: &str,
    _access_key_id: &str,
    _secret_access_key: &str,
) -> Result<AnthropicProvider, LlmConnectorError> {
    let protocol = AnthropicProtocol::new(""); // Bedrock does not require Anthropic API key
    let base_url = format!("https://bedrock-runtime.{}.amazonaws.com", region);
    let client = HttpClient::new(&base_url)?.with_header(
        "X-Amz-Target".to_string(),
        "BedrockRuntime_20231002.InvokeModel".to_string(),
    );
    Ok(GenericProvider::new(protocol, client))
}

pub fn anthropic_compatible(
    api_key: &str,
    base_url: &str,
    service_name: &str,
) -> Result<AnthropicProvider, LlmConnectorError> {
    let protocol = AnthropicProtocol::new(api_key);
    let client = HttpClient::new(base_url)?
        .with_header("Authorization".to_string(), format!("Bearer {}", api_key))
        .with_header("anthropic-version".to_string(), "2023-06-01".to_string())
        .with_header(
            "User-Agent".to_string(),
            format!("llm-connector/{}", service_name),
        );
    Ok(GenericProvider::new(protocol, client))
}

pub fn validate_anthropic_key(api_key: &str) -> bool {
    api_key.starts_with("sk-ant-") && api_key.len() > 10
}

// ============================================================================
// Zhipu Provider
// ============================================================================

pub type ZhipuProvider = GenericProvider<ZhipuProtocol>;

pub fn zhipu(api_key: &str, base_url: &str) -> Result<ZhipuProvider, LlmConnectorError> {
    zhipu_with_config(api_key, false, base_url, None, None)
}

pub fn zhipu_with_config(
    api_key: &str,
    openai_compatible: bool,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<ZhipuProvider, LlmConnectorError> {
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?
        .with_header("Authorization".to_string(), format!("Bearer {}", api_key));

    let protocol = if openai_compatible {
        ZhipuProtocol::new_openai_compatible(api_key)
    } else {
        ZhipuProtocol::new(api_key)
    };

    Ok(GenericProvider::new(protocol, client))
}

pub fn zhipu_openai_compatible(
    api_key: &str,
    base_url: &str,
) -> Result<ZhipuProvider, LlmConnectorError> {
    let client = HttpClient::new(base_url)?
        .with_header("Authorization".to_string(), format!("Bearer {}", api_key));
    Ok(GenericProvider::new(
        ZhipuProtocol::new_openai_compatible(api_key),
        client,
    ))
}

pub fn zhipu_enterprise(api_key: &str, base_url: &str) -> Result<ZhipuProvider, LlmConnectorError> {
    zhipu(api_key, base_url)
}

pub fn validate_zhipu_key(api_key: &str) -> bool {
    api_key.len() > 10
}

// ============================================================================
// Google Provider
// ============================================================================

pub type GoogleProvider = GenericProvider<GoogleProtocol>;

pub fn google(api_key: &str, base_url: &str) -> Result<GoogleProvider, LlmConnectorError> {
    google_with_config(api_key, base_url, None, None)
}

pub fn google_with_config(
    api_key: &str,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<GoogleProvider, LlmConnectorError> {
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?
        .with_header("x-goog-api-key".to_string(), api_key.to_string());
    Ok(GenericProvider::new(GoogleProtocol::new(), client))
}

// ============================================================================
// Ollama Provider
// ============================================================================

#[derive(Clone, Debug)]
pub struct OllamaProvider {
    inner: GenericProvider<OllamaProtocol>,
}

impl OllamaProvider {
    pub fn new(base_url: &str) -> Result<Self, LlmConnectorError> {
        let client = HttpClient::new(base_url)?;
        Ok(Self {
            inner: GenericProvider::new(OllamaProtocol::new(), client),
        })
    }

    pub fn with_config(
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let client = HttpClient::with_config(base_url, timeout_secs, proxy)?;
        Ok(Self {
            inner: GenericProvider::new(OllamaProtocol::new(), client),
        })
    }

    pub fn client(&self) -> &HttpClient {
        self.inner.client()
    }

    pub async fn pull_model(&self, model_name: &str) -> Result<(), LlmConnectorError> {
        let request = serde_json::json!({
            "name": model_name,
            "stream": false,
        });
        let url = format!("{}/api/pull", self.inner.client().base_url());
        let _ = self.inner.client().post(&url, &request).await?;
        Ok(())
    }

    pub async fn delete_model(&self, model_name: &str) -> Result<(), LlmConnectorError> {
        let request = serde_json::json!({ "name": model_name });
        let url = format!("{}/api/delete", self.inner.client().base_url());
        let _ = self.inner.client().post(&url, &request).await?;
        Ok(())
    }
}

#[async_trait]
impl Provider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    fn capabilities(&self) -> crate::protocols::common::capabilities::ProviderCapabilities {
        self.inner.capabilities()
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError> {
        self.inner.chat(request).await
    }
    #[cfg(feature = "streaming")]
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, LlmConnectorError> {
        self.inner.chat_stream(request).await
    }
    async fn models(&self) -> Result<Vec<String>, LlmConnectorError> {
        self.inner.models().await
    }
    async fn embed(&self, request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError> {
        self.inner.embed(request).await
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub fn ollama(base_url: &str) -> Result<OllamaProvider, LlmConnectorError> {
    OllamaProvider::new(base_url)
}

pub fn ollama_with_config(
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<OllamaProvider, LlmConnectorError> {
    OllamaProvider::with_config(base_url, timeout_secs, proxy)
}

// ============================================================================
// Tencent Provider
// ============================================================================

#[cfg(feature = "tencent")]
pub struct TencentProvider {
    protocol: TencentNativeProtocol,
    client: HttpClient,
}

#[cfg(feature = "tencent")]
impl TencentProvider {
    fn sign_request(&self, payload: &str) -> Result<Vec<(String, String)>, LlmConnectorError> {
        let now = Utc::now();
        let timestamp = now.timestamp();
        let date = now.format("%Y-%m-%d").to_string();
        let host = "hunyuan.tencentcloudapi.com";

        self.protocol
            .calculate_signature(host, payload, timestamp, &date)
            .map(|auth| {
                vec![
                    ("Authorization".to_string(), auth),
                    ("X-TC-Action".to_string(), "ChatCompletions".to_string()),
                    ("X-TC-Version".to_string(), "2023-09-01".to_string()),
                    ("X-TC-Timestamp".to_string(), timestamp.to_string()),
                    ("X-TC-Region".to_string(), "ap-guangzhou".to_string()),
                ]
            })
    }
}

#[cfg(feature = "tencent")]
#[async_trait]
impl Provider for TencentProvider {
    fn name(&self) -> &str {
        "tencent"
    }

    fn capabilities(&self) -> ProviderCapabilities {
        self.protocol.capabilities()
    }

    async fn chat(&self, request: &ChatRequest) -> Result<ChatResponse, LlmConnectorError> {
        let protocol_request = self.protocol.build_request(request)?;
        let body = serde_json::to_string(&protocol_request)
            .map_err(|e| LlmConnectorError::InvalidRequest(format!("Serialize error: {}", e)))?;
        let headers = self.sign_request(&body)?;
        let response = self
            .client
            .clone()
            .with_headers(headers.into_iter().collect())
            .post(
                &self
                    .protocol
                    .chat_endpoint(self.client.base_url(), &request.model),
                &protocol_request,
            )
            .await?;
        let status = response.status();
        let text = response
            .text()
            .await
            .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;
        if !status.is_success() {
            return Err(self.protocol.map_error(status.as_u16(), &text));
        }
        self.protocol.parse_response(&text)
    }

    #[cfg(feature = "streaming")]
    async fn chat_stream(&self, request: &ChatRequest) -> Result<ChatStream, LlmConnectorError> {
        let protocol_request = self.protocol.build_request(request)?;
        let body = serde_json::to_string(&protocol_request)
            .map_err(|e| LlmConnectorError::InvalidRequest(format!("Serialize error: {}", e)))?;
        let headers = self.sign_request(&body)?;
        let response = self
            .client
            .clone()
            .with_headers(headers.into_iter().collect())
            .stream(
                &self
                    .protocol
                    .chat_endpoint(self.client.base_url(), &request.model),
                &protocol_request,
            )
            .await?;
        let status = response.status();
        if !status.is_success() {
            let text = response
                .text()
                .await
                .map_err(|e| LlmConnectorError::NetworkError(e.to_string()))?;
            return Err(self.protocol.map_error(status.as_u16(), &text));
        }
        self.protocol.parse_stream_response(response).await
    }

    async fn models(&self) -> Result<Vec<String>, LlmConnectorError> {
        Ok(vec![
            "hunyuan-lite".to_string(),
            "hunyuan-standard".to_string(),
            "hunyuan-pro".to_string(),
        ])
    }

    async fn embed(&self, _request: &EmbedRequest) -> Result<EmbedResponse, LlmConnectorError> {
        Err(LlmConnectorError::UnsupportedOperation(
            "Tencent native embedding not implemented yet".to_string(),
        ))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[cfg(feature = "tencent")]
pub fn tencent(
    secret_id: &str,
    secret_key: &str,
    base_url: &str,
) -> Result<TencentProvider, LlmConnectorError> {
    tencent_with_config(secret_id, secret_key, base_url, None, None)
}

#[cfg(feature = "tencent")]
pub fn tencent_with_config(
    secret_id: &str,
    secret_key: &str,
    base_url: &str,
    timeout_secs: Option<u64>,
    proxy: Option<&str>,
) -> Result<TencentProvider, LlmConnectorError> {
    let protocol = TencentNativeProtocol::new(secret_id, secret_key);
    let client = HttpClient::with_config(base_url, timeout_secs, proxy)?;
    Ok(TencentProvider { protocol, client })
}

// ============================================================================
// Mock Provider
// ============================================================================

pub mod mock;
pub use mock::MockProvider;
