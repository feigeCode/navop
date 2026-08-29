//! HTTP Client Implementation - V2 Architecture
//!
//! Provides unified HTTP communication layer, supporting standard and streaming requests.

use crate::error::LlmConnectorError;
use reqwest::Client;
use serde::Serialize;
use std::borrow::Cow;
use std::collections::HashMap;
use std::time::Duration;

/// HTTP Client
///
/// Encapsulates all HTTP communication details, including authentication, timeout, proxy configuration, etc.
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    base_url: String,
    headers: HashMap<String, String>,
}

impl HttpClient {
    /// Create new HTTP client
    ///
    /// Default timeout: 60 seconds (suitable for most requests including streaming)
    ///
    /// **Important**: System proxy is **disabled** by default to avoid unexpected timeout issues.
    /// If you need to use a proxy, use `with_config()` and explicitly set the proxy parameter.
    pub fn new(base_url: &str) -> Result<Self, LlmConnectorError> {
        let client = Client::builder()
            .timeout(Duration::from_secs(120)) // Increased from 60 to 120 seconds for thinking/CoT
            .no_proxy() // Disable system proxy by default to avoid timeout issues
            .build()
            .map_err(|e| {
                LlmConnectorError::ConfigError(format!("Failed to create HTTP client: {}", e))
            })?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            headers: HashMap::new(),
        })
    }

    /// Create HTTP client with custom configuration
    ///
    /// # Parameters
    /// - `base_url`: Base URL for the API
    /// - `timeout_secs`: Optional timeout in seconds (default: 60 seconds)
    /// - `proxy`: Optional proxy URL
    ///
    /// # Proxy Behavior
    /// - If `proxy` is `None`: System proxy is **disabled** (no proxy used)
    /// - If `proxy` is `Some(url)`: The specified proxy is used for all protocols (HTTP/HTTPS)
    ///
    /// **Note**: System proxy is disabled by default to avoid unexpected timeout issues.
    /// This is different from reqwest's default behavior which enables system proxy.
    pub fn with_config(
        base_url: &str,
        timeout_secs: Option<u64>,
        proxy: Option<&str>,
    ) -> Result<Self, LlmConnectorError> {
        let mut builder = Client::builder();

        // Set timeout (default 120 seconds for thinking/CoT compatibility)
        if let Some(timeout) = timeout_secs {
            builder = builder.timeout(Duration::from_secs(timeout));
        } else {
            builder = builder.timeout(Duration::from_secs(120)); // Increased from 60 to 120 seconds
        }

        // Set proxy or disable system proxy
        if let Some(proxy_url) = proxy {
            // Use explicit proxy
            let proxy = reqwest::Proxy::all(proxy_url)
                .map_err(|e| LlmConnectorError::ConfigError(format!("Invalid proxy URL: {}", e)))?;
            builder = builder.proxy(proxy);
        } else {
            // Disable system proxy to avoid timeout issues
            builder = builder.no_proxy();
        }

        let client = builder.build().map_err(|e| {
            LlmConnectorError::ConfigError(format!("Failed to create HTTP client: {}", e))
        })?;

        Ok(Self {
            client,
            base_url: base_url.trim_end_matches('/').to_string(),
            headers: HashMap::new(),
        })
    }

    /// Add request headers
    pub fn with_headers(mut self, headers: HashMap<String, String>) -> Self {
        for (key, value) in headers {
            self.headers
                .insert(key, sanitize_header_value(&value).into_owned());
        }
        self
    }

    /// Add single request header
    pub fn with_header(mut self, key: String, value: String) -> Self {
        self.headers
            .insert(key, sanitize_header_value(&value).into_owned());
        self
    }

    /// Get base URL
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Send GET request
    pub async fn get(&self, url: &str) -> Result<reqwest::Response, LlmConnectorError> {
        let mut request = self.client.get(url);

        // Add all configured request headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        request.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmConnectorError::TimeoutError(format!("GET request timeout: {}", e))
            } else if e.is_connect() {
                LlmConnectorError::ConnectionError(format!("GET connection failed: {}", e))
            } else {
                LlmConnectorError::NetworkError(format!("GET request failed: {}", e))
            }
        })
    }

    /// Send POST request
    pub async fn post<T: Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response, LlmConnectorError> {
        let mut request = self.client.post(url).json(body);

        // Add all configured request headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        request.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmConnectorError::TimeoutError(format!("POST request timeout: {}", e))
            } else if e.is_connect() {
                LlmConnectorError::ConnectionError(format!("POST connection failed: {}", e))
            } else {
                LlmConnectorError::NetworkError(format!("POST request failed: {}", e))
            }
        })
    }

    /// Send streaming POST request
    ///
    /// Note: Streaming requests use the same timeout as configured in the client.
    /// For long-running streams, consider using `with_config()` to set a longer timeout.
    ///
    /// Recommended timeout for streaming: 60-300 seconds depending on expected response length.
    #[cfg(feature = "streaming")]
    pub async fn stream<T: Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response, LlmConnectorError> {
        let mut request = self.client.post(url).json(body);

        // Add streaming-specific headers
        request = request.header("Accept", "text/event-stream");
        request = request.header("Cache-Control", "no-cache");
        request = request.header("Connection", "keep-alive");

        // Add all configured request headers
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        request.send().await
            .map_err(|e| {
                if e.is_timeout() {
                    LlmConnectorError::TimeoutError(format!("Stream request timeout: {}. Consider increasing timeout for long-running streams.", e))
                } else if e.is_connect() {
                    LlmConnectorError::ConnectionError(format!("Stream connection failed: {}", e))
                } else {
                    LlmConnectorError::NetworkError(format!("Stream request failed: {}", e))
                }
            })
    }

    /// Send POST request with custom headers
    pub async fn post_with_custom_headers<T: Serialize>(
        &self,
        url: &str,
        body: &T,
        custom_headers: &HashMap<String, String>,
    ) -> Result<reqwest::Response, LlmConnectorError> {
        let mut request = self.client.post(url).json(body);

        // Add custom headers first
        for (key, value) in custom_headers {
            request = request.header(key, sanitize_header_value(value).as_ref());
        }

        // Then add configured headers (may override custom headers)
        for (key, value) in &self.headers {
            request = request.header(key, value);
        }

        request.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmConnectorError::TimeoutError(format!("POST request timeout: {}", e))
            } else if e.is_connect() {
                LlmConnectorError::ConnectionError(format!("POST connection failed: {}", e))
            } else {
                LlmConnectorError::NetworkError(format!("POST request failed: {}", e))
            }
        })
    }

    /// Send POST request with header overrides (overrides take precedence over client headers)
    ///
    /// Used for per-request API key, base URL, and custom header overrides (e.g. X-Trace-Id).
    pub async fn post_with_overrides<T: Serialize>(
        &self,
        url: &str,
        body: &T,
        overrides: &HashMap<String, String>,
    ) -> Result<reqwest::Response, LlmConnectorError> {
        // Construct final headers map to avoid duplicates
        let mut final_headers = reqwest::header::HeaderMap::new();

        // 1. Add base headers
        for (key, value) in &self.headers {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                final_headers.insert(header_name, header_value(value));
            }
        }

        // 2. Apply overrides (overwrite existing keys)
        for (key, value) in overrides {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                final_headers.insert(header_name, header_value(value));
            }
        }

        let request = self.client.post(url).json(body).headers(final_headers);

        // Debug outbound request if enabled
        #[cfg(debug_assertions)]
        if std::env::var("LLM_DEBUG_OUTBOUND").is_ok() {
            println!("[LLM-DEBUG] POST {}", url);
            // Print request headers
            // We need to clone the request to inspect it, but reqwest::RequestBuilder doesn't support cloning easily in this context
            // So we'll rely on what we just built
            // Note: This debug block is a best-effort logging
        }

        request.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmConnectorError::TimeoutError(format!("POST request timeout: {}", e))
            } else if e.is_connect() {
                LlmConnectorError::ConnectionError(format!("POST connection failed: {}", e))
            } else {
                LlmConnectorError::NetworkError(format!("POST request failed: {}", e))
            }
        })
    }

    /// Send streaming POST request with header overrides (overrides take precedence)
    #[cfg(feature = "streaming")]
    pub async fn stream_with_overrides<T: Serialize>(
        &self,
        url: &str,
        body: &T,
        overrides: &HashMap<String, String>,
    ) -> Result<reqwest::Response, LlmConnectorError> {
        // Construct final headers map
        let mut final_headers = reqwest::header::HeaderMap::new();

        // 1. Add default streaming headers
        final_headers.insert(
            reqwest::header::ACCEPT,
            reqwest::header::HeaderValue::from_static("text/event-stream"),
        );
        final_headers.insert(
            reqwest::header::CACHE_CONTROL,
            reqwest::header::HeaderValue::from_static("no-cache"),
        );
        final_headers.insert(
            reqwest::header::CONNECTION,
            reqwest::header::HeaderValue::from_static("keep-alive"),
        );

        // 2. Add base headers
        for (key, value) in &self.headers {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                final_headers.insert(header_name, header_value(value));
            }
        }

        // 3. Apply overrides (overwrite existing keys)
        for (key, value) in overrides {
            if let Ok(header_name) = reqwest::header::HeaderName::from_bytes(key.as_bytes()) {
                final_headers.insert(header_name, header_value(value));
            }
        }

        let request = self.client.post(url).json(body).headers(final_headers);

        // Debug outbound request if enabled
        #[cfg(debug_assertions)]
        if std::env::var("LLM_DEBUG_OUTBOUND").is_ok() {
            println!("[LLM-DEBUG] STREAM POST {}", url);
        }

        request.send().await.map_err(|e| {
            if e.is_timeout() {
                LlmConnectorError::TimeoutError(format!(
                    "Stream request timeout: {}. Consider increasing timeout for long-running streams.",
                    e
                ))
            } else if e.is_connect() {
                LlmConnectorError::ConnectionError(format!("Stream connection failed: {}", e))
            } else {
                LlmConnectorError::NetworkError(format!("Stream request failed: {}", e))
            }
        })
    }
}

/// Builds a validated [`reqwest::header::HeaderValue`] from a raw string value.
///
/// The value is sanitized to printable ASCII first so the request can never be
/// rejected by providers that proxy over gRPC-gateway, which refuses request
/// headers whose values contain non-printable ASCII (for example surfacing as
/// `header key "grpcgateway-user-agent" contains value with non-printable ASCII
/// characters`). Sanitizing also prevents reqwest from failing request
/// construction for the same reason.
fn header_value(value: &str) -> reqwest::header::HeaderValue {
    reqwest::header::HeaderValue::from_str(&sanitize_header_value(value))
        .expect("sanitized header value must be valid")
}

/// Replaces bytes that are invalid in an HTTP header value with `?`.
///
/// Header values must be printable US-ASCII; multi-byte UTF-8 and control
/// characters (except nothing here) are replaced. Existing printable values are
/// returned unchanged.
fn sanitize_header_value(value: &str) -> Cow<'_, str> {
    if value.bytes().all(is_printable_ascii) {
        return Cow::Borrowed(value);
    }
    let sanitized: String = value
        .bytes()
        .map(|b| if is_printable_ascii(b) { char::from(b) } else { '?' })
        .collect();
    Cow::Owned(sanitized)
}

fn is_printable_ascii(byte: u8) -> bool {
    (0x20..=0x7E).contains(&byte)
}

impl std::fmt::Debug for HttpClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpClient")
            .field("base_url", &self.base_url)
            .field("headers_count", &self.headers.len())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_keeps_printable_ascii_unchanged() {
        assert_eq!(
            sanitize_header_value("Bearer sk-abc123 / ok"),
            Cow::Borrowed("Bearer sk-abc123 / ok")
        );
    }

    #[test]
    fn sanitize_replaces_non_printable_ascii() {
        assert_eq!(
            sanitize_header_value("llm-connector/商汤"),
            "llm-connector/??????"
        );
        assert_eq!(sanitize_header_value("line\nbreak"), "line?break");
        assert_eq!(sanitize_header_value("tab\there"), "tab?here");
    }

    #[test]
    fn sanitize_replaces_high_bytes_and_control_bytes() {
        assert_eq!(sanitize_header_value("\u{7f}"), "?");
        assert_eq!(sanitize_header_value("\u{00}"), "?");
        assert_eq!(sanitize_header_value("a\u{80}b"), "a??b");
    }

    #[test]
    fn header_value_from_sanitized_input_never_fails() {
        for raw in [
            "商汤",
            "Bearer sk-test",
            "line\nbreak",
            "tab\there",
            "no newline \r\n",
        ] {
            let value = header_value(raw);
            assert!(value.to_str().is_ok(), "raw = {raw:?}");
            assert!(
                value.to_str().unwrap().bytes().all(is_printable_ascii),
                "raw = {raw:?}"
            );
        }
    }

    #[test]
    fn with_header_stores_sanitized_value() {
        let client = HttpClient::new("https://api.example.com").unwrap();
        let client = client.with_header("X-Test".to_string(), "value\nwith\0bad".to_string());
        let stored = client.headers.get("X-Test").expect("header present");
        assert_eq!(stored, "value?with?bad");
    }
}
