use super::*;
use std::collections::HashMap;

pub(super) fn composer_agent_options(
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
) -> Vec<ComposerAgentOption> {
    composer_agent_options_with_status(
        backend,
        acp_agents,
        current_acp_id,
        acp_connecting,
        &HashMap::new(),
    )
}

/// 同 [`composer_agent_options`]，但允许用探测状态替换每个 ACP agent 的副标题。
///
/// 状态由调用方提供，`None` 表示尚未探测、保持默认副标题。这样探测能力不渗进
/// 输入框的数据模型。
pub(super) fn composer_agent_options_with_status(
    backend: Backend,
    acp_agents: &[AcpAgentEntry],
    current_acp_id: Option<&SharedString>,
    acp_connecting: bool,
    statuses: &HashMap<SharedString, SharedString>,
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
        let mut option = ComposerAgentOption::acp(
            entry.id.clone(),
            entry.name.clone(),
            backend == Backend::Acp && current_acp_id == Some(&entry.id),
            acp_connecting,
        );
        if let Some(status) = statuses.get(&entry.id) {
            option.subtitle = status.clone();
        }
        option
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
    has_live_connection_or_auth: bool,
    requested_id: &SharedString,
) -> bool {
    backend == Backend::Acp && current_id == Some(requested_id) && has_live_connection_or_auth
}
