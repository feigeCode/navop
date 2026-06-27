use agent_runtime::ToolRegistry;
use gpui::{App, Global};
use std::sync::Arc;

pub type PlanToolRegistryProvider =
    Arc<dyn Fn(&mut App) -> anyhow::Result<ToolRegistry> + Send + Sync + 'static>;

struct GlobalPlanToolRegistryProvider {
    provider: PlanToolRegistryProvider,
}

impl Global for GlobalPlanToolRegistryProvider {}

pub fn set_plan_tool_registry_provider(
    cx: &mut App,
    provider: impl Fn(&mut App) -> anyhow::Result<ToolRegistry> + Send + Sync + 'static,
) {
    cx.set_global(GlobalPlanToolRegistryProvider {
        provider: Arc::new(provider),
    });
}

pub fn build_plan_tool_registry(cx: &mut App) -> anyhow::Result<ToolRegistry> {
    let Some(provider) = cx
        .try_global::<GlobalPlanToolRegistryProvider>()
        .map(|global| global.provider.clone())
    else {
        return Ok(ToolRegistry::new());
    };
    provider(cx)
}
