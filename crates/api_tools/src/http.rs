//! HTTP 请求准备与执行（移植自 verve 的 `src/http/client.rs`，做了精简）。
//!
//! 只保留接口测试所需的最小集合：{{var}} 变量替换、URL 规范化、请求头、
//! raw 请求体、超时执行与 JSON 响应美化。

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use anyhow::Result;
use base64::Engine as _;
use futures::AsyncReadExt as _;
use gpui::http_client::{AsyncBody, Builder, HttpClient, HttpRequestExt, Method, RedirectPolicy};

use crate::variable_resolver::VariableResolver;

/// 支持的 HTTP 方法。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RequestMethod {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
    Trace,
}

impl RequestMethod {
    pub const ALL: &'static [RequestMethod] = &[
        RequestMethod::Get,
        RequestMethod::Post,
        RequestMethod::Put,
        RequestMethod::Delete,
        RequestMethod::Patch,
        RequestMethod::Head,
        RequestMethod::Options,
        RequestMethod::Trace,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RequestMethod::Get => "GET",
            RequestMethod::Post => "POST",
            RequestMethod::Put => "PUT",
            RequestMethod::Delete => "DELETE",
            RequestMethod::Patch => "PATCH",
            RequestMethod::Head => "HEAD",
            RequestMethod::Options => "OPTIONS",
            RequestMethod::Trace => "TRACE",
        }
    }

    /// 树节点徽标用的短标签（如 "DEL"、"OPT"）。
    pub fn badge_label(self) -> &'static str {
        match self {
            RequestMethod::Delete => "DEL",
            RequestMethod::Options => "OPT",
            RequestMethod::Trace => "TRC",
            other => other.label(),
        }
    }
}

impl std::fmt::Display for RequestMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

/// form-data 字段类型。
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    #[default]
    Text,
    File,
}

/// 一条请求头 / 参数 / Cookie / 请求体键值对。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct KeyValue {
    pub key: String,
    pub value: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub field_type: FieldType,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file_path: Option<String>,
}

fn default_true() -> bool {
    true
}

impl KeyValue {
    pub fn new(key: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            value: value.into(),
            enabled: true,
            field_type: FieldType::Text,
            file_path: None,
        }
    }
}

/// 请求体类型。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BodyType {
    #[default]
    None,
    FormData,
    Urlencoded,
    Raw,
}

impl BodyType {
    pub const ALL: &'static [BodyType] = &[
        BodyType::None,
        BodyType::FormData,
        BodyType::Urlencoded,
        BodyType::Raw,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BodyType::None => "None",
            BodyType::FormData => "form-data",
            BodyType::Urlencoded => "x-www-form-urlencoded",
            BodyType::Raw => "raw",
        }
    }
}

/// Raw 请求体的语言。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RawLanguage {
    #[default]
    Json,
    Xml,
    Text,
    Html,
    Javascript,
}

impl RawLanguage {
    pub const ALL: &'static [RawLanguage] = &[
        RawLanguage::Json,
        RawLanguage::Xml,
        RawLanguage::Text,
        RawLanguage::Html,
        RawLanguage::Javascript,
    ];

    pub fn label(self) -> &'static str {
        match self {
            RawLanguage::Json => "JSON",
            RawLanguage::Xml => "XML",
            RawLanguage::Text => "Text",
            RawLanguage::Html => "HTML",
            RawLanguage::Javascript => "JavaScript",
        }
    }

    pub fn content_type(self) -> &'static str {
        match self {
            RawLanguage::Json => "application/json",
            RawLanguage::Xml => "application/xml",
            RawLanguage::Text => "text/plain",
            RawLanguage::Html => "text/html",
            RawLanguage::Javascript => "application/javascript",
        }
    }
}

/// 鉴权方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthType {
    #[default]
    None,
    Bearer,
    Basic,
    ApiKey,
}

impl AuthType {
    pub const ALL: &'static [AuthType] = &[
        AuthType::None,
        AuthType::Bearer,
        AuthType::Basic,
        AuthType::ApiKey,
    ];

    pub fn label(self) -> &'static str {
        match self {
            AuthType::None => "No Auth",
            AuthType::Bearer => "Bearer Token",
            AuthType::Basic => "Basic Auth",
            AuthType::ApiKey => "API Key",
        }
    }
}

/// API Key 的注入位置。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuthTarget {
    #[default]
    Header,
    Query,
}

impl AuthTarget {
    pub const ALL: &'static [AuthTarget] = &[AuthTarget::Header, AuthTarget::Query];

    pub fn label(self) -> &'static str {
        match self {
            AuthTarget::Header => "Header",
            AuthTarget::Query => "Query",
        }
    }
}

/// 鉴权配置。
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct AuthConfig {
    #[serde(default)]
    pub auth_type: AuthType,
    #[serde(default)]
    pub token: String,
    #[serde(default)]
    pub username: String,
    #[serde(default)]
    pub password: String,
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub add_to: AuthTarget,
}

/// 一个已解析、可直接发送的请求。
#[derive(Debug, Clone)]
pub struct PreparedRequest {
    pub method: RequestMethod,
    pub url: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}

/// HTTP 响应快照（status == 0 表示传输层错误，错误信息在 `error` 中）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct HttpResponse {
    pub status: u16,
    pub status_text: String,
    pub time_ms: u64,
    pub size: u64,
    pub headers: Vec<KeyValue>,
    pub raw_body: String,
    pub body: String,
    pub is_json: bool,
    pub streaming: bool,
    pub error: Option<String>,
}

/// 把 `{{key}}` 占位符替换为变量池中的值。
pub fn substitute(input: &str, vars: &BTreeMap<String, String>) -> String {
    VariableResolver::new(vars).substitute(input)
}

/// 规范化 URL：去掉首部多余斜杠；缺省协议时补指定 scheme。
pub fn normalize_url_with_default(url: &str, scheme: &str) -> String {
    let trimmed = url.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    if trimmed.contains("://") {
        return trimmed.to_string();
    }
    let body = trimmed.trim_start_matches('/');
    format!("{scheme}://{body}")
}

/// 规范化 HTTP URL：去掉首部多余斜杠；缺省协议时补 `http://`。
#[allow(dead_code)]
pub fn normalize_url(url: &str) -> String {
    normalize_url_with_default(url, "http")
}

#[allow(dead_code)]
fn resolve_url(
    raw_url: &str,
    resolver: &mut VariableResolver<'_>,
    vars: &BTreeMap<String, String>,
    default_scheme: &str,
) -> String {
    let mut url = resolver.substitute(raw_url);
    if !url.contains("://")
        && let Some(base) = vars.get("__folder_base_url__")
    {
        let base = resolver.substitute(base);
        let base = base.trim().trim_end_matches('/');
        let path = url.trim_start_matches('/');
        if !base.is_empty() {
            url = format!("{base}/{path}");
        }
    }
    normalize_url_with_default(&url, default_scheme)
}

/// 解析变量、目录 Base URL 和默认 scheme。
#[allow(dead_code)]
pub fn resolve_url_with_default(
    raw_url: &str,
    vars: &BTreeMap<String, String>,
    default_scheme: &str,
) -> String {
    let mut resolver = VariableResolver::new(vars);
    resolve_url(raw_url, &mut resolver, vars, default_scheme)
}

/// 准备一个请求：替换变量、规范化 URL、应用请求头、序列化 raw 请求体。
#[allow(dead_code)]
pub fn prepare(
    method: RequestMethod,
    raw_url: &str,
    headers: &[KeyValue],
    body_raw: &str,
    vars: &BTreeMap<String, String>,
) -> Result<PreparedRequest> {
    let mut resolver = VariableResolver::new(vars);
    let url = resolve_url(raw_url, &mut resolver, vars, "http");

    let out_headers: Vec<(String, String)> = headers
        .iter()
        .filter(|h| h.enabled && !h.key.trim().is_empty())
        .map(|h| (resolver.substitute(&h.key), resolver.substitute(&h.value)))
        .collect();

    let body = resolver.substitute(body_raw).into_bytes();

    Ok(PreparedRequest {
        method,
        url,
        headers: out_headers,
        body,
    })
}

fn has_header(headers: &[(String, String)], name: &str) -> bool {
    headers
        .iter()
        .any(|(key, _)| key.eq_ignore_ascii_case(name))
}

fn push_header(headers: &mut Vec<(String, String)>, name: &str, value: String) {
    if !has_header(headers, name) {
        headers.push((name.to_string(), value));
    }
}

fn encoded_pairs(rows: &[KeyValue], resolver: &mut VariableResolver<'_>) -> String {
    rows.iter()
        .filter(|row| row.enabled && !row.key.trim().is_empty())
        .map(|row| {
            format!(
                "{}={}",
                url::form_urlencoded::byte_serialize(resolver.substitute(&row.key).as_bytes())
                    .collect::<String>(),
                url::form_urlencoded::byte_serialize(resolver.substitute(&row.value).as_bytes())
                    .collect::<String>()
            )
        })
        .collect::<Vec<_>>()
        .join("&")
}

fn append_query(url: &str, query: &str) -> String {
    if query.is_empty() {
        url.to_string()
    } else if url.contains('?') {
        format!("{url}&{query}")
    } else {
        format!("{url}?{query}")
    }
}

/// 完整准备请求：变量替换、URL 参数、请求头、Cookie、鉴权、请求体。
#[allow(dead_code)]
#[allow(clippy::too_many_arguments)]
pub fn prepare_full(
    method: RequestMethod,
    raw_url: &str,
    params: &[KeyValue],
    headers: &[KeyValue],
    cookies: &[KeyValue],
    auth: &AuthConfig,
    body_type: BodyType,
    raw_body: &str,
    body_rows: &[KeyValue],
    raw_language: RawLanguage,
    vars: &BTreeMap<String, String>,
) -> Result<PreparedRequest> {
    prepare_full_with_default_scheme(
        method,
        raw_url,
        params,
        headers,
        cookies,
        auth,
        body_type,
        raw_body,
        body_rows,
        raw_language,
        vars,
        "http",
    )
}

/// 完整准备请求，并允许实时协议指定缺省 scheme。
#[allow(clippy::too_many_arguments)]
pub fn prepare_full_with_default_scheme(
    method: RequestMethod,
    raw_url: &str,
    params: &[KeyValue],
    headers: &[KeyValue],
    cookies: &[KeyValue],
    auth: &AuthConfig,
    body_type: BodyType,
    raw_body: &str,
    body_rows: &[KeyValue],
    raw_language: RawLanguage,
    vars: &BTreeMap<String, String>,
    default_scheme: &str,
) -> Result<PreparedRequest> {
    let mut resolver = VariableResolver::new(vars);
    let mut url = resolve_url(raw_url, &mut resolver, vars, default_scheme);
    url = append_query(&url, &encoded_pairs(params, &mut resolver));

    if auth.auth_type == AuthType::ApiKey
        && auth.add_to == AuthTarget::Query
        && !auth.key.trim().is_empty()
    {
        let auth_query = format!(
            "{}={}",
            url::form_urlencoded::byte_serialize(resolver.substitute(&auth.key).as_bytes())
                .collect::<String>(),
            url::form_urlencoded::byte_serialize(resolver.substitute(&auth.value).as_bytes())
                .collect::<String>()
        );
        url = append_query(&url, &auth_query);
    }

    let mut out_headers: Vec<(String, String)> = headers
        .iter()
        .filter(|h| h.enabled && !h.key.trim().is_empty())
        .map(|h| (resolver.substitute(&h.key), resolver.substitute(&h.value)))
        .collect();

    if cookies
        .iter()
        .any(|c| c.enabled && !c.key.trim().is_empty())
    {
        let cookie = cookies
            .iter()
            .filter(|c| c.enabled && !c.key.trim().is_empty())
            .map(|c| {
                format!(
                    "{}={}",
                    resolver.substitute(&c.key).trim(),
                    resolver.substitute(&c.value).trim()
                )
            })
            .collect::<Vec<_>>()
            .join("; ");
        push_header(&mut out_headers, "Cookie", cookie);
    }

    match auth.auth_type {
        AuthType::Bearer => {
            if !auth.token.trim().is_empty() {
                push_header(
                    &mut out_headers,
                    "Authorization",
                    format!("Bearer {}", resolver.substitute(&auth.token)),
                );
            }
        }
        AuthType::Basic => {
            let user = resolver.substitute(&auth.username);
            let pass = resolver.substitute(&auth.password);
            let encoded =
                base64::engine::general_purpose::STANDARD.encode(format!("{user}:{pass}"));
            push_header(
                &mut out_headers,
                "Authorization",
                format!("Basic {encoded}"),
            );
        }
        AuthType::ApiKey if auth.add_to == AuthTarget::Header => {
            if !auth.key.trim().is_empty() {
                push_header(
                    &mut out_headers,
                    &resolver.substitute(&auth.key),
                    resolver.substitute(&auth.value),
                );
            }
        }
        AuthType::None | AuthType::ApiKey => {}
    }

    let body: Vec<u8> = match body_type {
        BodyType::None => Vec::new(),
        BodyType::Raw => {
            push_header(
                &mut out_headers,
                "Content-Type",
                raw_language.content_type().to_string(),
            );
            let body = resolver.substitute(raw_body);
            if raw_language == RawLanguage::Json && !body.trim().is_empty() {
                serde_json::from_str::<serde_json::Value>(&body)
                    .map_err(|error| anyhow::anyhow!("invalid JSON body: {error}"))?;
            }
            body.into_bytes()
        }
        BodyType::Urlencoded => {
            push_header(
                &mut out_headers,
                "Content-Type",
                "application/x-www-form-urlencoded".to_string(),
            );
            encoded_pairs(body_rows, &mut resolver).into_bytes()
        }
        BodyType::FormData => {
            let boundary = format!("----navop-api-tool-{}", uuid::Uuid::new_v4().simple());
            push_header(
                &mut out_headers,
                "Content-Type",
                format!("multipart/form-data; boundary={boundary}"),
            );
            crate::multipart::build_with_boundary_with_resolver(
                body_rows,
                &mut resolver,
                &boundary,
            )?
        }
    };

    Ok(PreparedRequest {
        method,
        url,
        headers: out_headers,
        body,
    })
}

/// 发送准备好的请求并捕获响应。任何错误都会转成 `HttpResponse`（status == 0）。
pub async fn execute(
    client: &dyn HttpClient,
    prepared: PreparedRequest,
    timeout_secs: u64,
) -> HttpResponse {
    let method = match prepared.method {
        RequestMethod::Get => Method::GET,
        RequestMethod::Post => Method::POST,
        RequestMethod::Put => Method::PUT,
        RequestMethod::Delete => Method::DELETE,
        RequestMethod::Patch => Method::PATCH,
        RequestMethod::Head => Method::HEAD,
        RequestMethod::Options => Method::OPTIONS,
        RequestMethod::Trace => Method::TRACE,
    };

    let mut builder = Builder::new()
        .uri(&prepared.url)
        .method(method)
        .follow_redirects(RedirectPolicy::FollowAll);
    for (k, v) in &prepared.headers {
        builder = builder.header(k.clone(), v.clone());
    }
    let req = match builder.body(AsyncBody::from(prepared.body.clone())) {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse {
                error: Some(format!("build request: {e}")),
                ..Default::default()
            };
        }
    };

    let start = Instant::now();
    let send_fut = client.send(req);
    let result = smol::future::or(
        async {
            smol::Timer::after(Duration::from_secs(timeout_secs.max(1))).await;
            Err(anyhow::anyhow!("request timed out after {timeout_secs}s"))
        },
        send_fut,
    )
    .await;

    let time_ms = start.elapsed().as_millis() as u64;

    let resp = match result {
        Ok(r) => r,
        Err(e) => {
            return HttpResponse {
                status: 0,
                status_text: "Error".into(),
                time_ms,
                error: Some(format!("{e}")),
                ..Default::default()
            };
        }
    };

    let status = resp.status().as_u16();
    let status_text = resp.status().canonical_reason().unwrap_or("").to_string();

    let mut headers: Vec<KeyValue> = Vec::new();
    for (name, value) in resp.headers().iter() {
        headers.push(KeyValue::new(
            name.as_str(),
            value.to_str().unwrap_or("<binary>"),
        ));
    }

    let mut body_buf = resp.into_body();
    let mut buf = Vec::new();
    let _ = body_buf.read_to_end(&mut buf).await;
    let size = buf.len() as u64;

    let is_json = headers
        .iter()
        .any(|h| h.key.eq_ignore_ascii_case("content-type") && is_json_content_type(&h.value));
    let raw_body = String::from_utf8_lossy(&buf).to_string();
    let body = if is_json {
        match serde_json::from_str::<serde_json::Value>(&raw_body) {
            Ok(v) => serde_json::to_string_pretty(&v).unwrap_or_else(|_| raw_body.clone()),
            Err(_) => raw_body.clone(),
        }
    } else {
        raw_body.clone()
    };

    HttpResponse {
        status,
        status_text,
        time_ms,
        size,
        headers,
        raw_body,
        body,
        is_json,
        streaming: false,
        error: None,
    }
}

fn is_json_content_type(value: &str) -> bool {
    value.to_ascii_lowercase().contains("json")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars() -> BTreeMap<String, String> {
        let mut m = BTreeMap::new();
        m.insert("baseUrl".to_string(), "https://api.example.com".to_string());
        m
    }

    #[test]
    fn substitute_replaces_placeholders() {
        assert_eq!(
            substitute("{{baseUrl}}/users", &vars()),
            "https://api.example.com/users"
        );
        assert_eq!(
            substitute("{{ baseUrl }}/users", &vars()),
            "https://api.example.com/users"
        );
        assert_eq!(substitute("{{missing}}/x", &vars()), "{{missing}}/x");
    }

    #[test]
    fn substitute_expands_dynamic_variables() {
        let uuid = substitute("{{$uuid}}", &BTreeMap::new());
        assert!(uuid::Uuid::parse_str(&uuid).is_ok());

        let timestamp = substitute("{{$timestamp}}", &BTreeMap::new());
        assert!(timestamp.parse::<u64>().is_ok());

        let random = substitute("{{$random}}", &BTreeMap::new());
        assert_eq!(random.len(), 10);
        assert!(random.chars().all(|ch| ch.is_ascii_alphanumeric()));

        let sparkid = substitute("{{$sparkid}}", &BTreeMap::new());
        assert_eq!(sparkid.len(), 32);
        assert!(sparkid.chars().all(|ch| ch.is_ascii_hexdigit()));
    }

    #[test]
    fn key_value_defaults_to_enabled_when_field_is_missing() {
        let row: KeyValue =
            serde_json::from_str(r#"{"key":"X-Trace","value":"1"}"#).expect("valid key value");
        assert!(row.enabled);
        assert_eq!(row.field_type, FieldType::Text);
        assert!(row.file_path.is_none());
    }

    #[test]
    fn key_value_file_metadata_round_trips() {
        let row = KeyValue {
            key: "avatar".into(),
            value: String::new(),
            enabled: true,
            field_type: FieldType::File,
            file_path: Some("/tmp/avatar.png".into()),
        };

        let json = serde_json::to_string(&row).unwrap();
        let loaded: KeyValue = serde_json::from_str(&json).unwrap();

        assert_eq!(loaded, row);
    }

    #[test]
    fn json_content_type_detection_is_case_insensitive() {
        assert!(is_json_content_type("application/json"));
        assert!(is_json_content_type("Application/JSON; Charset=UTF-8"));
        assert!(is_json_content_type("application/problem+json"));
        assert!(!is_json_content_type("text/plain"));
        assert!(!HttpResponse::default().streaming);
    }

    #[test]
    fn normalize_url_adds_default_scheme_and_trims_slashes() {
        assert_eq!(normalize_url("www.baidu.com"), "http://www.baidu.com");
        assert_eq!(normalize_url("/api/users"), "http://api/users");
        assert_eq!(normalize_url("https://x.test/a"), "https://x.test/a");
        assert_eq!(normalize_url("  "), "");
    }

    #[test]
    fn normalize_url_supports_protocol_specific_default_scheme() {
        assert_eq!(
            normalize_url_with_default("example.test/socket", "ws"),
            "ws://example.test/socket"
        );
        assert_eq!(
            normalize_url_with_default("/example.test/socket", "ws"),
            "ws://example.test/socket"
        );
        assert_eq!(
            normalize_url_with_default("wss://example.test/socket", "ws"),
            "wss://example.test/socket"
        );
        assert_eq!(normalize_url_with_default("  ", "ws"), "");
    }

    #[test]
    fn resolve_url_applies_folder_base_url_only_to_relative_urls() {
        let vars = BTreeMap::from([
            (
                "__folder_base_url__".to_string(),
                " https://{{host}}/v1/ ".to_string(),
            ),
            ("host".to_string(), "api.example.test".to_string()),
            ("user".to_string(), "users".to_string()),
        ]);

        assert_eq!(
            resolve_url_with_default("/{{user}}", &vars, "http"),
            "https://api.example.test/v1/users"
        );
        assert_eq!(
            resolve_url_with_default("https://other.test/users", &vars, "http"),
            "https://other.test/users"
        );
        assert_eq!(
            resolve_url_with_default(
                "/users",
                &BTreeMap::from([("__folder_base_url__".to_string(), "  ".to_string(),)]),
                "http",
            ),
            "http://users"
        );
    }

    #[test]
    fn prepare_full_with_default_scheme_keeps_query_and_protocol_defaults() {
        let vars = BTreeMap::from([(
            "__folder_base_url__".to_string(),
            "https://api.example.test/v1".to_string(),
        )]);
        let params = vec![KeyValue::new("page", "2")];

        let http = prepare_full_with_default_scheme(
            RequestMethod::Get,
            "/users",
            &params,
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::None,
            "",
            &[],
            RawLanguage::Json,
            &vars,
            "http",
        )
        .unwrap();
        assert_eq!(http.url, "https://api.example.test/v1/users?page=2");

        let websocket = prepare_full_with_default_scheme(
            RequestMethod::Get,
            "example.test/socket",
            &[],
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::None,
            "",
            &[],
            RawLanguage::Json,
            &BTreeMap::new(),
            "ws",
        )
        .unwrap();
        assert_eq!(websocket.url, "ws://example.test/socket");

        let tcp = prepare_full_with_default_scheme(
            RequestMethod::Get,
            "example.test:9000",
            &[],
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::None,
            "",
            &[],
            RawLanguage::Json,
            &BTreeMap::new(),
            "tcp",
        )
        .unwrap();
        assert_eq!(tcp.url, "tcp://example.test:9000");
    }

    #[test]
    fn prepare_applies_vars_headers_and_body() {
        let headers = vec![KeyValue::new("X-Api-Key", "{{key}}")];
        let mut v = vars();
        v.insert("key".to_string(), "secret".to_string());
        let p = prepare(
            RequestMethod::Post,
            "{{baseUrl}}/login",
            &headers,
            r#"{"name":"{{name}}"}"#,
            &v,
        )
        .unwrap();
        assert_eq!(p.url, "https://api.example.com/login");
        assert_eq!(
            p.headers,
            vec![("X-Api-Key".to_string(), "secret".to_string())]
        );
        assert_eq!(p.body, br#"{"name":"{{name}}"}"#.to_vec());
    }

    #[test]
    fn prepare_skips_disabled_or_empty_headers() {
        let headers = vec![
            KeyValue::new("keep", "yes"),
            KeyValue {
                key: "skip".into(),
                value: "x".into(),
                enabled: false,
                ..KeyValue::default()
            },
            KeyValue::new("", "no-key"),
        ];
        let p = prepare(RequestMethod::Get, "https://x.test", &headers, "", &vars()).unwrap();
        assert_eq!(p.headers, vec![("keep".to_string(), "yes".to_string())]);
    }

    #[test]
    fn prepare_full_applies_params_auth_cookies_and_urlencoded_body() {
        let p = prepare_full(
            RequestMethod::Post,
            "{{baseUrl}}/login",
            &[KeyValue::new("from", "app")],
            &[KeyValue::new("X-Trace", "1")],
            &[KeyValue::new("session", "abc")],
            &AuthConfig {
                auth_type: AuthType::Bearer,
                token: "{{token}}".to_string(),
                ..AuthConfig::default()
            },
            BodyType::Urlencoded,
            "",
            &[KeyValue::new("user", "tom")],
            RawLanguage::Json,
            &vars(),
        )
        .unwrap();

        assert_eq!(p.url, "https://api.example.com/login?from=app");
        assert!(
            p.headers
                .contains(&("Cookie".to_string(), "session=abc".to_string()))
        );
        assert!(
            p.headers
                .contains(&("Authorization".to_string(), "Bearer {{token}}".to_string()))
        );
        assert!(p.headers.contains(&(
            "Content-Type".to_string(),
            "application/x-www-form-urlencoded".to_string()
        )));
        assert_eq!(p.body, b"user=tom".to_vec());
    }

    #[test]
    fn prepare_full_reuses_dynamic_variables_across_the_request() {
        let prepared = prepare_full(
            RequestMethod::Post,
            "https://example.test/{{$uuid}}",
            &[KeyValue::new("trace", "{{$uuid}}")],
            &[KeyValue::new("X-Request-Id", "{{$uuid}}")],
            &[],
            &AuthConfig::default(),
            BodyType::Raw,
            r#"{"requestId":"{{$uuid}}"}"#,
            &[],
            RawLanguage::Json,
            &BTreeMap::new(),
        )
        .expect("dynamic variable request should prepare");

        let path_value = prepared
            .url
            .strip_prefix("https://example.test/")
            .and_then(|value| value.split('?').next())
            .expect("URL should contain the dynamic path value");
        let query_value = prepared
            .url
            .split("trace=")
            .nth(1)
            .expect("URL should contain the dynamic query value");
        let header_value = prepared
            .headers
            .iter()
            .find_map(|(key, value)| (key == "X-Request-Id").then_some(value.as_str()))
            .expect("request ID header should exist");
        let body: serde_json::Value =
            serde_json::from_slice(&prepared.body).expect("body should be valid JSON");
        let body_value = body["requestId"]
            .as_str()
            .expect("body should contain requestId");

        assert_eq!(path_value, query_value);
        assert_eq!(path_value, header_value);
        assert_eq!(path_value, body_value);
    }

    #[test]
    fn prepare_full_validates_raw_json_after_substitution() {
        let valid = prepare_full(
            RequestMethod::Post,
            "https://example.com",
            &[],
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::Raw,
            r#"{"name":"{{name}}"}"#,
            &[],
            RawLanguage::Json,
            &BTreeMap::from([("name".to_string(), "navop".to_string())]),
        );
        assert!(valid.is_ok());

        let invalid = prepare_full(
            RequestMethod::Post,
            "https://example.com",
            &[],
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::Raw,
            r#"{"name":}"#,
            &[],
            RawLanguage::Json,
            &BTreeMap::new(),
        );
        assert!(
            invalid
                .unwrap_err()
                .to_string()
                .contains("invalid JSON body")
        );

        let text = prepare_full(
            RequestMethod::Post,
            "https://example.com",
            &[],
            &[],
            &[],
            &AuthConfig::default(),
            BodyType::Raw,
            "not-json",
            &[],
            RawLanguage::Text,
            &BTreeMap::new(),
        );
        assert!(text.is_ok());
    }
}
