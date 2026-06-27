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

#[cfg(test)]
mod tests {
    use super::{build_plan_tool_registry, set_plan_tool_registry_provider};
    use agent_runtime::{ToolName, ToolRegistry, tools::builtin::EchoTool};
    use gpui::TestAppContext;
    use std::sync::Arc;

    #[gpui::test]
    fn build_plan_tool_registry_returns_empty_without_provider(cx: &mut TestAppContext) {
        let registry = cx.update(|cx| build_plan_tool_registry(cx).unwrap());

        assert!(registry.is_empty());
    }

    #[gpui::test]
    fn build_plan_tool_registry_uses_registered_provider(cx: &mut TestAppContext) {
        let registry = cx.update(|cx| {
            set_plan_tool_registry_provider(cx, |_cx| {
                Ok(ToolRegistry::new().with_tool(Arc::new(EchoTool)))
            });
            build_plan_tool_registry(cx).unwrap()
        });

        assert!(registry.contains(&ToolName::new("echo")));
    }
}
