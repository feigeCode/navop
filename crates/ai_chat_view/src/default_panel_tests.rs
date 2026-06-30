use crate::default_panel::{build_sidebar_config, enabled_provider_configs};
use crate::{AcpAgentConfig, AgentChatViewConfig};
use agent_runtime::model::{MockModelClient, ModelClient};
use agent_runtime::{ResourceContext, Runtime, RuntimeServices, ToolRegistry, ToolRouter};
use one_core::llm::{ProviderConfig, ProviderType};
use std::sync::Arc;

#[test]
fn enabled_provider_configs_filters_disabled_entries() {
    let enabled = ProviderConfig {
        id: 1,
        name: "enabled".to_string(),
        provider_type: ProviderType::OpenAI,
        enabled: true,
        ..ProviderConfig::default()
    };
    let disabled = ProviderConfig {
        id: 2,
        name: "disabled".to_string(),
        provider_type: ProviderType::OpenAI,
        enabled: false,
        ..ProviderConfig::default()
    };

    let configs = enabled_provider_configs(vec![enabled, disabled]);

    assert_eq!(1, configs.len());
    assert_eq!("enabled", configs[0].name);
}

#[test]
fn sidebar_config_keeps_acp_agents_available() {
    let config = AgentChatViewConfig::new(test_runtime(), ResourceContext::new(), Vec::new());
    let agents = vec![AcpAgentConfig::new("codex", "Codex ACP", "codex")];

    let config = build_sidebar_config(config, agents);

    assert!(config.sidebar_mode);
    assert_eq!(1, config.acp_agents.len());
    assert_eq!(config.acp_agents[0].id.as_ref(), "codex");
}

fn test_runtime() -> Arc<Runtime> {
    let model: Arc<dyn ModelClient> = Arc::new(MockModelClient::new([]));
    Arc::new(Runtime::new(RuntimeServices::new(
        model,
        Arc::new(ToolRouter::new(ToolRegistry::new())),
    )))
}
