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

/// Default idle timeout between reads. It is applied per body read and resets on
/// every chunk, so a healthy stream is never aborted merely for running long;
/// only a stall with no bytes for this long fails the request.
const DEFAULT_READ_TIMEOUT: Duration = Duration::from_secs(120);

/// Bound the connection phase so an unreachable host cannot hang forever.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);

fn build_http_client(
    read_timeout: Duration,
    proxy: Option<&str>,
) -> Result<Client, LlmConnectorError> {
    let mut builder = Client::builder()
        .connect_timeout(CONNECT_TIMEOUT)
        // IMPORTANT: never use `ClientBuilder::timeout()` here. It is a *total*
        // deadline that also covers the response body, so it would abort long
        // LLM streams (thinking/CoT, tool loops) mid-flight. `read_timeout` is
        // the correct guard: it only fires after this long without any bytes.
        .read_timeout(read_timeout);

    match proxy {
        Some(proxy_url) => {
            let proxy = reqwest::Proxy::all(proxy_url)
                .map_err(|e| LlmConnectorError::ConfigError(format!("Invalid proxy URL: {}", e)))?;
            builder = builder.proxy(proxy);
        }
        None => {
            // Disable system proxy to avoid unexpected timeout/panic issues.
            builder = builder.no_proxy();
        }
    }

    builder
        .build()
        .map_err(|e| LlmConnectorError::ConfigError(format!("Failed to create HTTP client: {}", e)))
}

impl HttpClient {
    /// Create new HTTP client
    ///
    /// Default idle timeout: 120 seconds. It only aborts a stream that stalls,
    /// never one that keeps streaming.
    ///
    /// **Important**: System proxy is **disabled** by default to avoid unexpected timeout issues.
    /// If you need to use a proxy, use `with_config()` and explicitly set the proxy parameter.
    pub fn new(base_url: &str) -> Result<Self, LlmConnectorError> {
        let client = build_http_client(DEFAULT_READ_TIMEOUT, None)?;

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
    /// - `timeout_secs`: Optional idle timeout in seconds (default: 120 seconds)
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
        let read_timeout = timeout_secs
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_READ_TIMEOUT);
        let client = build_http_client(read_timeout, proxy)?;

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
    /// Note: the configured timeout is an idle (per-read) timeout. A stream that
    /// keeps emitting bytes is never aborted, no matter how long it runs; only a
    /// stall longer than the timeout fails it.
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
                    LlmConnectorError::TimeoutError(format!("Stream request timed out after a period with no data: {}. Consider increasing the idle timeout for slow providers.", e))
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
                    "Stream request timed out after a period with no data: {}. Consider increasing the idle timeout for slow providers.",
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
        .map(|b| {
            if is_printable_ascii(b) {
                char::from(b)
            } else {
                '?'
            }
        })
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

    use futures::StreamExt;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// Starts a raw HTTP server that streams `chunks` SSE frames with `gap`
    /// between them. When `finish` is false it stalls (no bytes) instead of
    /// closing, to exercise the idle timeout.
    async fn spawn_sse_server(gap: Duration, chunks: usize, finish: bool) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind test server");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.expect("accept");
            let mut request = [0u8; 4096];
            let _ = socket.read(&mut request).await;
            socket
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
                )
                .await
                .expect("write headers");
            for _ in 0..chunks {
                tokio::time::sleep(gap).await;
                let body = "data: {\"x\":1}\n\n";
                let frame = format!("{:X}\r\n{body}\r\n", body.len());
                socket
                    .write_all(frame.as_bytes())
                    .await
                    .expect("write frame");
            }
            if finish {
                let _ = socket.write_all(b"0\r\n\r\n").await;
            } else {
                tokio::time::sleep(Duration::from_secs(3)).await;
            }
            let _ = socket.shutdown().await;
        });
        format!("http://{addr}")
    }

    async fn open_stream(base: &str, idle_timeout_secs: u64) -> reqwest::Response {
        let client = HttpClient::with_config(base, Some(idle_timeout_secs), None).expect("client");
        client
            .stream(
                &format!("{base}/v1/chat/completions"),
                &serde_json::json!({ "stream": true }),
            )
            .await
            .expect("stream request should connect")
    }

    #[tokio::test]
    async fn stream_survives_longer_than_the_idle_timeout() {
        // 5 frames * 300ms = 1.5s total, which exceeds the 1s idle timeout.
        // Only a total-timeout client would abort this; an idle-timeout client
        // keeps going because bytes keep arriving.
        let base = spawn_sse_server(Duration::from_millis(300), 5, true).await;
        let response = open_stream(&base, 1).await;
        let mut stream = response.bytes_stream();
        let mut bytes = 0usize;
        while let Some(chunk) = stream.next().await {
            bytes += chunk
                .expect("healthy stream must not be aborted by a total timeout")
                .len();
        }
        assert_eq!(bytes, 5 * "data: {\"x\":1}\n\n".len());
    }

    #[tokio::test]
    async fn stream_aborts_after_a_real_stall() {
        let base = spawn_sse_server(Duration::from_millis(100), 1, false).await;
        let response = open_stream(&base, 1).await;
        let mut stream = response.bytes_stream();
        let first = stream.next().await.expect("first frame").expect("ok");
        assert!(!first.is_empty());
        let stalled = stream
            .next()
            .await
            .expect("stalled stream should yield an error, not end cleanly");
        assert!(
            stalled.is_err(),
            "a stalled stream must time out: {stalled:?}"
        );
    }
}
