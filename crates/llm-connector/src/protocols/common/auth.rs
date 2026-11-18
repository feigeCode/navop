//! Common Authentication Strategies

use std::collections::HashMap;

#[derive(Clone, Debug)]
pub enum AuthStrategy {
    None,
    Bearer {
        api_key: String,
    },
    ApiKeyHeader {
        api_key: String,
        header_name: String,
    },
}

#[derive(Clone, Debug, Default)]
pub struct HeaderPolicy {
    pub headers: Vec<(String, String)>,
}

#[derive(Clone, Debug, Default)]
pub struct RequestMetadataPolicy {
    pub header_overrides: HashMap<String, String>,
}

pub fn materialize_auth_headers(strategy: &AuthStrategy) -> Vec<(String, String)> {
    match strategy {
        AuthStrategy::None => vec![],
        AuthStrategy::Bearer { api_key } => bearer_auth(api_key),
        AuthStrategy::ApiKeyHeader {
            api_key,
            header_name,
        } => api_key_header(api_key, header_name),
    }
}

pub fn apply_header_policy(
    mut headers: Vec<(String, String)>,
    policy: &HeaderPolicy,
) -> Vec<(String, String)> {
    headers.extend(policy.headers.clone());
    headers
}

pub fn merge_metadata_policies(
    base: RequestMetadataPolicy,
    extra: RequestMetadataPolicy,
) -> RequestMetadataPolicy {
    let mut merged = base.header_overrides;
    merged.extend(extra.header_overrides);
    RequestMetadataPolicy {
        header_overrides: merged,
    }
}

/// Create standard Bearer token header
pub fn bearer_auth(api_key: &str) -> Vec<(String, String)> {
    vec![("Authorization".to_string(), format!("Bearer {}", api_key))]
}

/// Create standard API Key header (e.g. for Anthropic)
pub fn api_key_header(api_key: &str, header_name: &str) -> Vec<(String, String)> {
    vec![(header_name.to_string(), api_key.to_string())]
}

#[cfg(test)]
mod tests {
    use super::{api_key_header, bearer_auth};

    #[test]
    fn auth_headers_leave_content_type_to_the_json_body_encoder() {
        assert_eq!(
            vec![("Authorization".to_string(), "Bearer sk-test".to_string())],
            bearer_auth("sk-test")
        );
        assert_eq!(
            vec![("x-api-key".to_string(), "sk-test".to_string())],
            api_key_header("sk-test", "x-api-key")
        );
    }
}
