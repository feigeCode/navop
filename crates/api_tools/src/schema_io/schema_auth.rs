use serde_json::{Value, json};

use super::schema_shared::{operation_id, resolved};
use crate::http::{AuthConfig, AuthTarget, AuthType, RequestMethod};

pub fn import_openapi(root: &Value, operation: &Value) -> AuthConfig {
    let Some(name) = security_name(root, operation) else {
        return AuthConfig::default();
    };
    let scheme = resolved(root, &root["components"]["securitySchemes"][name]);
    match scheme["type"].as_str() {
        Some("http") if scheme["scheme"].as_str() == Some("bearer") => AuthConfig {
            auth_type: AuthType::Bearer,
            ..AuthConfig::default()
        },
        Some("http") if scheme["scheme"].as_str() == Some("basic") => AuthConfig {
            auth_type: AuthType::Basic,
            ..AuthConfig::default()
        },
        Some("apiKey") => api_key_config(scheme),
        _ => AuthConfig::default(),
    }
}

pub fn import_swagger(root: &Value, operation: &Value) -> AuthConfig {
    let Some(name) = security_name(root, operation) else {
        return AuthConfig::default();
    };
    let scheme = resolved(root, &root["securityDefinitions"][name]);
    match scheme["type"].as_str() {
        Some("basic") => AuthConfig {
            auth_type: AuthType::Basic,
            ..AuthConfig::default()
        },
        Some("apiKey")
            if scheme["name"]
                .as_str()
                .is_some_and(|key| key.eq_ignore_ascii_case("authorization")) =>
        {
            AuthConfig {
                auth_type: AuthType::Bearer,
                ..AuthConfig::default()
            }
        }
        Some("apiKey") => api_key_config(scheme),
        _ => AuthConfig::default(),
    }
}

fn security_name<'a>(root: &'a Value, operation: &'a Value) -> Option<&'a str> {
    operation
        .get("security")
        .or_else(|| root.get("security"))
        .and_then(Value::as_array)
        .and_then(|items| items.first())
        .and_then(Value::as_object)
        .and_then(|item| item.keys().next())
        .map(String::as_str)
}

fn api_key_config(scheme: &Value) -> AuthConfig {
    AuthConfig {
        auth_type: AuthType::ApiKey,
        key: scheme["name"].as_str().unwrap_or("X-API-Key").to_string(),
        add_to: if scheme["in"].as_str() == Some("query") {
            AuthTarget::Query
        } else {
            AuthTarget::Header
        },
        ..AuthConfig::default()
    }
}

pub fn export(auth: &AuthConfig, swagger: bool) -> Option<(String, Value)> {
    match auth.auth_type {
        AuthType::None => None,
        AuthType::Bearer if swagger => Some((
            "bearerAuth".into(),
            json!({"type": "apiKey", "name": "Authorization", "in": "header"}),
        )),
        AuthType::Bearer => Some((
            "bearerAuth".into(),
            json!({"type": "http", "scheme": "bearer"}),
        )),
        AuthType::Basic => Some((
            "basicAuth".into(),
            if swagger {
                json!({"type": "basic"})
            } else {
                json!({"type": "http", "scheme": "basic"})
            },
        )),
        AuthType::ApiKey => export_api_key(auth),
    }
}

fn export_api_key(auth: &AuthConfig) -> Option<(String, Value)> {
    let target = if auth.add_to == AuthTarget::Query {
        "query"
    } else {
        "header"
    };
    let key = if auth.key.is_empty() {
        "X-API-Key"
    } else {
        auth.key.as_str()
    };
    let name = format!(
        "apiKey_{}_{}",
        target,
        operation_id(key, RequestMethod::Get)
    );
    Some((name, json!({"type": "apiKey", "name": key, "in": target})))
}
