use crate::AcpAgentConfig;
use gpui::{App, Global};
use std::sync::Arc;

pub type AcpAgentConfigProvider =
    Arc<dyn Fn(&mut App) -> anyhow::Result<Vec<AcpAgentConfig>> + Send + Sync + 'static>;

struct GlobalAcpAgentConfigProvider {
    provider: AcpAgentConfigProvider,
}

impl Global for GlobalAcpAgentConfigProvider {}

pub fn set_acp_agent_config_provider(
    cx: &mut App,
    provider: impl Fn(&mut App) -> anyhow::Result<Vec<AcpAgentConfig>> + Send + Sync + 'static,
) {
    cx.set_global(GlobalAcpAgentConfigProvider {
        provider: Arc::new(provider),
    });
}

pub fn build_acp_agent_configs(cx: &mut App) -> anyhow::Result<Vec<AcpAgentConfig>> {
    let Some(provider) = cx
        .try_global::<GlobalAcpAgentConfigProvider>()
        .map(|global| global.provider.clone())
    else {
        return Ok(Vec::new());
    };
    provider(cx)
}
