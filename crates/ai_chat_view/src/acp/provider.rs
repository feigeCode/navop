use crate::{AcpAgentConfig, AcpAgentEntry};
use agent_runtime::ToolExecutionMode;
use gpui::{App, Global};
use std::sync::Arc;

pub type AcpAgentConfigProvider =
    Arc<dyn Fn(&mut App) -> anyhow::Result<Vec<AcpAgentEntry>> + Send + Sync + 'static>;

struct GlobalAcpAgentConfigProvider {
    provider: AcpAgentConfigProvider,
}

impl Global for GlobalAcpAgentConfigProvider {}

pub type AcpToolModeGetter =
    Arc<dyn Fn(&mut App) -> Option<ToolExecutionMode> + Send + Sync + 'static>;
pub type AcpToolModeSetter =
    Arc<dyn Fn(&mut App, ToolExecutionMode) -> anyhow::Result<()> + Send + Sync + 'static>;

struct GlobalAcpToolModeProvider {
    getter: AcpToolModeGetter,
    setter: AcpToolModeSetter,
}

impl Global for GlobalAcpToolModeProvider {}

pub fn set_acp_agent_config_provider(
    cx: &mut App,
    provider: impl Fn(&mut App) -> anyhow::Result<Vec<AcpAgentEntry>> + Send + Sync + 'static,
) {
    cx.set_global(GlobalAcpAgentConfigProvider {
        provider: Arc::new(provider),
    });
}

/// Registers the host-side permission bridge used by ACP sessions.
///
/// ACP agents call the Public MCP server in a separate process/protocol hop,
/// so the chat's tool-mode selector needs an explicit host integration rather
/// than relying on the local `agent_runtime` branch.
pub fn set_acp_tool_mode_provider(
    cx: &mut App,
    getter: impl Fn(&mut App) -> Option<ToolExecutionMode> + Send + Sync + 'static,
    setter: impl Fn(&mut App, ToolExecutionMode) -> anyhow::Result<()> + Send + Sync + 'static,
) {
    cx.set_global(GlobalAcpToolModeProvider {
        getter: Arc::new(getter),
        setter: Arc::new(setter),
    });
}

pub fn current_acp_tool_mode(cx: &mut App) -> Option<ToolExecutionMode> {
    let getter = cx
        .try_global::<GlobalAcpToolModeProvider>()
        .map(|provider| provider.getter.clone())?;
    getter(cx)
}

pub fn set_current_acp_tool_mode(cx: &mut App, mode: ToolExecutionMode) -> anyhow::Result<()> {
    let setter = cx
        .try_global::<GlobalAcpToolModeProvider>()
        .map(|provider| provider.setter.clone())
        .ok_or_else(|| anyhow::anyhow!("ACP tool mode provider is not configured"))?;
    setter(cx, mode)
}

pub fn build_acp_agent_configs(cx: &mut App) -> anyhow::Result<Vec<AcpAgentConfig>> {
    Ok(build_acp_agent_entries(cx)?
        .into_iter()
        .filter_map(|entry| entry.config)
        .collect())
}

pub fn build_acp_agent_entries(cx: &mut App) -> anyhow::Result<Vec<AcpAgentEntry>> {
    let Some(provider) = cx
        .try_global::<GlobalAcpAgentConfigProvider>()
        .map(|global| global.provider.clone())
    else {
        return Ok(Vec::new());
    };
    provider(cx)
}
