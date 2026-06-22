use gpui::{App, Global};
use public_mcp::tools::InternalFunctionDefinition;
use serde_json::json;

#[derive(Default)]
struct GlobalPublicMcpInternalFunctions {
    functions: Vec<InternalFunctionDefinition>,
}

impl Global for GlobalPublicMcpInternalFunctions {}

pub fn register_internal_function(cx: &mut App, definition: InternalFunctionDefinition) {
    ensure_registry(cx);
    let functions = &mut cx
        .global_mut::<GlobalPublicMcpInternalFunctions>()
        .functions;
    functions.retain(|function| function.name() != definition.name());
    functions.push(definition);
}

pub(super) fn builtin_definitions() -> Vec<InternalFunctionDefinition> {
    vec![app_info_function()]
}

pub(super) fn ensure_registry(cx: &mut App) {
    if !cx.has_global::<GlobalPublicMcpInternalFunctions>() {
        cx.set_global(GlobalPublicMcpInternalFunctions::default());
    }
}

pub(super) fn definitions(cx: &App) -> Vec<InternalFunctionDefinition> {
    cx.try_global::<GlobalPublicMcpInternalFunctions>()
        .map(|state| state.functions.clone())
        .unwrap_or_default()
}

fn app_info_function() -> InternalFunctionDefinition {
    InternalFunctionDefinition::read_only(
        "onetcli.app_info",
        "Read OnetCli app metadata.",
        |_| async {
            Ok(json!({
                "name": "onetcli",
                "version": env!("CARGO_PKG_VERSION")
            }))
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::TestAppContext;

    #[gpui::test]
    fn builtin_internal_functions_include_app_info(cx: &mut TestAppContext) {
        cx.update(|cx| {
            for definition in builtin_definitions() {
                register_internal_function(cx, definition);
            }
        });

        let names = cx.update(|cx| {
            definitions(cx)
                .into_iter()
                .map(|function| function.name().to_string())
                .collect::<Vec<_>>()
        });

        assert_eq!(vec!["onetcli.app_info"], names);
    }
}
