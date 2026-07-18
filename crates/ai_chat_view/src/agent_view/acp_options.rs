use super::*;

pub(super) fn composer_agent_options(
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
) -> Vec<ComposerAgentOption> {
    let mut options = vec![ComposerAgentOption::local(
        "One Agent",
        backend == Backend::Local,
        acp_connecting,
    )];
    options.extend(acp_agents.iter().map(|entry| {
        if let Some(diagnostic) = &entry.diagnostic {
            return ComposerAgentOption::invalid_acp(
                entry.id.clone(),
                entry.name.clone(),
                diagnostic.message.clone(),
            );
        }
        ComposerAgentOption::acp(
            entry.id.clone(),
            entry.name.clone(),
            backend == Backend::Acp && current_acp_id == Some(&entry.id),
            acp_connecting,
        )
    }));
    options
}

pub(super) fn current_agent_label(
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
) -> SharedString {
    if acp_connecting {
        return SharedString::from(t!("AgentUi.connecting").to_string());
    }
    if backend == Backend::Local {
        return SharedString::from("One Agent");
    }
    current_acp_id
        .and_then(|id| acp_agents.iter().find(|entry| &entry.id == id))
        .map(|entry| entry.name.clone())
        .unwrap_or_else(|| SharedString::from("ACP Agent"))
}

pub(super) fn agent_option_disabled(agent: &ComposerAgentOption) -> bool {
    !agent.enabled || (agent.connecting && agent.id.is_some())
}

pub(super) fn agent_selection_is_active(
    backend: Backend,
    current_id: Option<&SharedString>,
    has_pending: bool,
    requested_id: &SharedString,
) -> bool {
    current_id == Some(requested_id) && (backend == Backend::Acp || has_pending)
}
