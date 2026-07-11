use std::collections::BTreeSet;

use agent_client_protocol::schema::AuthMethodId;

use super::{AcpAuthConfig, AcpAuthMethodConfig, AcpError, AcpErrorKind, AcpRecoveryAction};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum AuthDecision {
    SkipNoMethods,
    UseLocalFallback,
    Authenticate(AuthMethodId),
    RequireInteraction { methods: Vec<AuthMethodId> },
}

pub(crate) fn select_auth(
    advertised: &[AuthMethodId],
    config: &AcpAuthConfig,
    available_env: &BTreeSet<String>,
    agent_id: &str,
    agent_name: &str,
) -> Result<AuthDecision, AcpError> {
    if advertised.is_empty() {
        return Ok(AuthDecision::SkipNoMethods);
    }
    if let Some(requested) = config.requested_method.as_deref() {
        return select_requested(
            requested,
            advertised,
            config,
            available_env,
            agent_id,
            agent_name,
        );
    }
    if let Some(decision) = select_preferred(advertised, config, available_env) {
        return Ok(decision);
    }
    if let Some(method) = configured_non_interactive(advertised, config, available_env) {
        return Ok(AuthDecision::Authenticate(method));
    }
    let interactive = configured_interactive(advertised, config);
    if !interactive.is_empty() {
        return Ok(AuthDecision::RequireInteraction {
            methods: interactive,
        });
    }
    if config.allow_unauthenticated_fallback {
        return Ok(AuthDecision::UseLocalFallback);
    }
    Err(missing_credentials(agent_id, agent_name, config))
}

fn select_requested(
    requested: &str,
    advertised: &[AuthMethodId],
    config: &AcpAuthConfig,
    available_env: &BTreeSet<String>,
    agent_id: &str,
    agent_name: &str,
) -> Result<AuthDecision, AcpError> {
    let configured = config.methods.iter().find(|method| method.id == requested);
    let advertised = advertised
        .iter()
        .find(|method| method.0.as_ref() == requested);
    let (Some(configured), Some(advertised)) = (configured, advertised) else {
        return Err(unsupported_method(agent_id, agent_name, requested));
    };
    if configured.interactive {
        return Ok(AuthDecision::RequireInteraction {
            methods: vec![advertised.clone()],
        });
    }
    if credentials_available(configured, available_env) {
        Ok(AuthDecision::Authenticate(advertised.clone()))
    } else {
        Err(missing_credentials(agent_id, agent_name, config))
    }
}

fn select_preferred(
    advertised: &[AuthMethodId],
    config: &AcpAuthConfig,
    available_env: &BTreeSet<String>,
) -> Option<AuthDecision> {
    let preferred = config.preferred_method.as_deref()?;
    let configured = config
        .methods
        .iter()
        .find(|method| method.id == preferred)?;
    let advertised = advertised
        .iter()
        .find(|method| method.0.as_ref() == preferred)?;
    if configured.interactive {
        Some(AuthDecision::RequireInteraction {
            methods: vec![advertised.clone()],
        })
    } else if credentials_available(configured, available_env) {
        Some(AuthDecision::Authenticate(advertised.clone()))
    } else {
        None
    }
}

fn configured_non_interactive(
    advertised: &[AuthMethodId],
    config: &AcpAuthConfig,
    available_env: &BTreeSet<String>,
) -> Option<AuthMethodId> {
    config.methods.iter().find_map(|configured| {
        (!configured.interactive && credentials_available(configured, available_env))
            .then(|| advertised_id(advertised, &configured.id))
            .flatten()
    })
}

fn configured_interactive(
    advertised: &[AuthMethodId],
    config: &AcpAuthConfig,
) -> Vec<AuthMethodId> {
    config
        .methods
        .iter()
        .filter(|method| method.interactive)
        .filter_map(|method| advertised_id(advertised, &method.id))
        .collect()
}

fn advertised_id(advertised: &[AuthMethodId], id: &str) -> Option<AuthMethodId> {
    advertised
        .iter()
        .find(|method| method.0.as_ref() == id)
        .cloned()
}

fn credentials_available(method: &AcpAuthMethodConfig, available_env: &BTreeSet<String>) -> bool {
    method
        .env_all
        .iter()
        .all(|name| available_env.contains(name))
        && (method.env_any.is_empty()
            || method
                .env_any
                .iter()
                .any(|name| available_env.contains(name)))
}

fn unsupported_method(agent_id: &str, agent_name: &str, method: &str) -> AcpError {
    AcpError::new(
        AcpErrorKind::UnsupportedAuthMethod,
        agent_id,
        agent_name,
        format!("ACP Agent 不支持鉴权方式 {method}"),
    )
    .with_recovery(AcpRecoveryAction::Configure {
        path: "acp-agents.json".to_string(),
    })
}

fn missing_credentials(agent_id: &str, agent_name: &str, config: &AcpAuthConfig) -> AcpError {
    let names = config
        .methods
        .iter()
        .flat_map(|method| method.env_all.iter().chain(&method.env_any))
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>()
        .join(", ");
    AcpError::new(
        AcpErrorKind::MissingCredentials,
        agent_id,
        agent_name,
        "ACP Agent 缺少可用凭证",
    )
    .with_detail(format!("需要环境变量: {names}"))
    .with_recovery(AcpRecoveryAction::Configure {
        path: "acp-agents.json".to_string(),
    })
}
