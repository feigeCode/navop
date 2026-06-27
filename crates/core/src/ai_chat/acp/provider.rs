use crate::ai_chat::acp::config::AcpAgentConfig;
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

#[cfg(test)]
mod tests {
    use super::{build_acp_agent_configs, set_acp_agent_config_provider};
    use crate::ai_chat::acp::config::AcpAgentConfig;
    use gpui::TestAppContext;

    #[gpui::test]
    fn build_acp_agent_configs_returns_empty_without_provider(cx: &mut TestAppContext) {
        let configs = cx.update(|cx| build_acp_agent_configs(cx).unwrap());

        assert!(configs.is_empty());
    }

    #[gpui::test]
    fn build_acp_agent_configs_uses_registered_provider(cx: &mut TestAppContext) {
        let configs = cx.update(|cx| {
            set_acp_agent_config_provider(cx, |_cx| {
                Ok(vec![AcpAgentConfig::new_http(
                    "local",
                    "Local ACP",
                    "http://127.0.0.1:9876",
                )])
            });
            build_acp_agent_configs(cx).unwrap()
        });

        assert_eq!(1, configs.len());
        assert_eq!("local", configs[0].id.as_ref());
    }
}
