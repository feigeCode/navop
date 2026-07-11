use std::collections::BTreeSet;

use agent_client_protocol::schema::AuthMethodId;

use super::auth::{AuthDecision, select_auth};
use super::{AcpAuthConfig, AcpAuthMethodConfig, AcpErrorKind};

#[test]
fn requested_method_wins_when_advertised_and_configured() {
    let auth = auth_config(Some("api-key"), Some("chat-gpt"), true);
    let available = env(&["OPENAI_API_KEY"]);

    let decision = select_auth(
        &advertised(&["api-key", "chat-gpt"]),
        &auth,
        &available,
        "codex",
        "Codex",
    )
    .unwrap();

    assert!(matches!(
        decision,
        AuthDecision::Authenticate(method) if method.0.as_ref() == "api-key"
    ));
}

#[test]
fn interactive_method_requires_user_action() {
    let auth = auth_config(None, Some("opencode-login"), true);

    let decision = select_auth(
        &advertised(&["opencode-login"]),
        &auth,
        &BTreeSet::new(),
        "opencode",
        "OpenCode",
    )
    .unwrap();

    assert_eq!(
        AuthDecision::RequireInteraction {
            methods: advertised(&["opencode-login"]),
        },
        decision
    );
}

#[test]
fn missing_credentials_without_fallback_is_an_error() {
    let auth = auth_config(None, Some("api-key"), false);

    let error = select_auth(
        &advertised(&["api-key"]),
        &auth,
        &BTreeSet::new(),
        "codex",
        "Codex",
    )
    .unwrap_err();

    assert_eq!(AcpErrorKind::MissingCredentials, error.kind);
}

#[test]
fn requested_legacy_agent_method_requires_interaction() {
    let auth = AcpAuthConfig {
        requested_method: Some("opencode-login".to_string()),
        preferred_method: None,
        allow_unauthenticated_fallback: true,
        methods: Vec::new(),
    };

    let decision = select_auth(
        &advertised(&["opencode-login"]),
        &auth,
        &BTreeSet::new(),
        "opencode",
        "OpenCode",
    )
    .unwrap();

    assert_eq!(
        AuthDecision::RequireInteraction {
            methods: advertised(&["opencode-login"]),
        },
        decision
    );
}

fn advertised(ids: &[&str]) -> Vec<AuthMethodId> {
    ids.iter().map(|id| AuthMethodId::new(*id)).collect()
}

fn env(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

fn auth_config(
    requested: Option<&str>,
    preferred: Option<&str>,
    interactive_preferred: bool,
) -> AcpAuthConfig {
    let mut methods = vec![AcpAuthMethodConfig {
        id: "api-key".to_string(),
        env_any: vec!["OPENAI_API_KEY".to_string(), "CODEX_API_KEY".to_string()],
        env_all: Vec::new(),
        interactive: false,
    }];
    if let Some(preferred) = preferred.filter(|method| *method != "api-key") {
        methods.push(AcpAuthMethodConfig {
            id: preferred.to_string(),
            env_any: Vec::new(),
            env_all: Vec::new(),
            interactive: interactive_preferred,
        });
    }
    AcpAuthConfig {
        requested_method: requested.map(str::to_string),
        preferred_method: preferred.map(str::to_string),
        allow_unauthenticated_fallback: false,
        methods,
    }
}
