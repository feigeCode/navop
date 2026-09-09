//! Builder pattern for LlmClient
//!
//! Provides a fluent API for constructing `LlmClient` instances with optional configuration.
//!
//! # Examples
//!
//! ```rust,no_run
//! use llm_connector::LlmClient;
//!
//! // Simple usage
//! let client = LlmClient::builder()
//!     .openai("sk-...")
//!     .build()
//!     .unwrap();
//!
//! // With custom configuration
//! let client = LlmClient::builder()
//!     .openai("sk-...")
//!     .base_url("https://api.deepseek.com")
//!     .timeout(60)
//!     .build()
//!     .unwrap();
//!
//! // DeepSeek shorthand
//! let client = LlmClient::builder()
//!     .deepseek("sk-...")
//!     .timeout(120)
//!     .build()
//!     .unwrap();
//!
//! // Ollama (no API key needed)
//! let client = LlmClient::builder()
//!     .ollama()
//!     .base_url("http://192.168.1.100:11434")
//!     .build()
//!     .unwrap();
//! ```

use crate::client::LlmClient;
use crate::error::LlmConnectorError;

/// Target provider for the builder
#[derive(Debug, Clone)]
enum ProviderKind {
    OpenAI,
    Anthropic,
    Aliyun,
    Zhipu,
    ZhipuOpenAI,
    Ollama,
    Xinference,
    DeepSeek,
    Moonshot,
    Volcengine,
    Google,
    Xiaomi,
    LongcatAnthropic,
    OpenAICompatible {
        service_name: String,
    },
    AzureOpenAI {
        endpoint: String,
        api_version: String,
    },
    #[cfg(feature = "tencent")]
    Tencent {
        secret_key: String,
    },
}

/// Fluent builder for `LlmClient`
///
/// # Example
/// ```rust,no_run
/// use llm_connector::LlmClient;
///
/// let client = LlmClient::builder()
///     .openai("sk-...")
///     .base_url("https://custom-endpoint.com")
///     .timeout(30)
///     .build()
///     .unwrap();
/// ```
pub struct LlmClientBuilder {
    provider: Option<ProviderKind>,
    api_key: Option<String>,
    base_url: Option<String>,
    timeout_secs: Option<u64>,
    proxy: Option<String>,
}

impl LlmClientBuilder {
    pub(crate) fn new() -> Self {
        Self {
            provider: None,
            api_key: None,
            base_url: None,
            timeout_secs: None,
            proxy: None,
        }
    }

    // ========================================================================
    // Provider selection methods
    // ========================================================================

    /// Use OpenAI as the provider
    pub fn openai(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::OpenAI);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Anthropic as the provider
    pub fn anthropic(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Anthropic);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Aliyun DashScope as the provider
    pub fn aliyun(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Aliyun);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Zhipu GLM as the provider
    pub fn zhipu(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Zhipu);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Zhipu GLM (OpenAI compatible mode) as the provider
    pub fn zhipu_openai(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::ZhipuOpenAI);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Ollama as the provider (no API key needed)
    pub fn ollama(mut self) -> Self {
        self.provider = Some(ProviderKind::Ollama);
        self
    }

    /// Use Xinference as the provider (OpenAI-compatible, no API key by default)
    pub fn xinference(mut self) -> Self {
        self.provider = Some(ProviderKind::Xinference);
        self
    }

    /// Use DeepSeek as the provider
    pub fn deepseek(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::DeepSeek);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Moonshot as the provider
    pub fn moonshot(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Moonshot);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Volcengine as the provider
    pub fn volcengine(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Volcengine);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Google as the provider
    pub fn google(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Google);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Xiaomi MiMo as the provider
    pub fn xiaomi(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::Xiaomi);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use LongCat Anthropic as the provider
    pub fn longcat_anthropic(mut self, api_key: &str) -> Self {
        self.provider = Some(ProviderKind::LongcatAnthropic);
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use any OpenAI-compatible service as the provider
    ///
    /// # Parameters
    /// - `api_key`: API key
    /// - `service_name`: Name of the service (for logging/identification)
    pub fn openai_compatible(mut self, api_key: &str, service_name: &str) -> Self {
        self.provider = Some(ProviderKind::OpenAICompatible {
            service_name: service_name.to_string(),
        });
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Azure OpenAI as the provider
    ///
    /// # Parameters
    /// - `api_key`: Azure OpenAI API key
    /// - `endpoint`: Azure endpoint URL
    /// - `api_version`: API version string
    pub fn azure_openai(mut self, api_key: &str, endpoint: &str, api_version: &str) -> Self {
        self.provider = Some(ProviderKind::AzureOpenAI {
            endpoint: endpoint.to_string(),
            api_version: api_version.to_string(),
        });
        self.api_key = Some(api_key.to_string());
        self
    }

    /// Use Tencent Hunyuan as the provider
    ///
    /// # Parameters
    /// - `secret_id`: Tencent Cloud SecretID
    /// - `secret_key`: Tencent Cloud SecretKey
    #[cfg(feature = "tencent")]
    pub fn tencent(mut self, secret_id: &str, secret_key: &str) -> Self {
        self.provider = Some(ProviderKind::Tencent {
            secret_key: secret_key.to_string(),
        });
        self.api_key = Some(secret_id.to_string());
        self
    }

    // ========================================================================
    // Configuration methods
    // ========================================================================

    /// Set custom base URL
    pub fn base_url(mut self, url: &str) -> Self {
        self.base_url = Some(url.to_string());
        self
    }

    /// Set request timeout in seconds
    pub fn timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = Some(secs);
        self
    }

    /// Set HTTP proxy
    pub fn proxy(mut self, proxy_url: &str) -> Self {
        self.proxy = Some(proxy_url.to_string());
        self
    }

    // ========================================================================
    // Build
    // ========================================================================

    /// Build the `LlmClient`
    pub fn build(self) -> Result<LlmClient, LlmConnectorError> {
        let provider = self.provider.ok_or_else(|| {
            LlmConnectorError::InvalidRequest(
                "No provider specified. Call .openai(), .deepseek(), etc. before .build()".into(),
            )
        })?;

        let api_key = self.api_key.as_deref().unwrap_or("");
        let base_url = self.base_url.as_deref().ok_or_else(|| {
            LlmConnectorError::InvalidRequest(
                "No base_url specified. Call .base_url() before .build()".into(),
            )
        })?;
        let timeout = self.timeout_secs;
        let proxy = self.proxy.as_deref();

        match provider {
            ProviderKind::OpenAI => {
                LlmClient::openai_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Anthropic => {
                LlmClient::anthropic_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Aliyun => {
                LlmClient::aliyun_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Zhipu => {
                LlmClient::zhipu_with_config(api_key, false, base_url, timeout, proxy)
            }
            ProviderKind::ZhipuOpenAI => {
                LlmClient::zhipu_with_config(api_key, true, base_url, timeout, proxy)
            }
            ProviderKind::Ollama => LlmClient::ollama_with_config(base_url, timeout, proxy),
            ProviderKind::Xinference => LlmClient::xinference_with_config(base_url, timeout, proxy),
            ProviderKind::DeepSeek => {
                LlmClient::deepseek_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Moonshot => {
                LlmClient::moonshot_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Volcengine => {
                LlmClient::volcengine_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Google => {
                LlmClient::google_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::Xiaomi => {
                LlmClient::xiaomi_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::LongcatAnthropic => {
                LlmClient::longcat_anthropic_with_config(api_key, base_url, timeout, proxy)
            }
            ProviderKind::OpenAICompatible { service_name } => {
                LlmClient::openai_compatible(api_key, base_url, &service_name)
            }
            ProviderKind::AzureOpenAI {
                endpoint,
                api_version,
            } => LlmClient::azure_openai(api_key, &endpoint, &api_version),
            #[cfg(feature = "tencent")]
            ProviderKind::Tencent { secret_key } => {
                LlmClient::tencent_with_config(api_key, &secret_key, base_url, timeout, proxy)
            }
        }
    }
}
