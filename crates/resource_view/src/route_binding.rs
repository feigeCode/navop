//! Route 构造:按声明绑定把 route/selection 投影为目标页路由。

use extension_runtime::extension::manifest::{
    ResourceWorkbenchBinding, ResourceWorkbenchBindingSource,
};
use std::collections::BTreeMap;

/// 按 open/link 声明构造目标页 route。
/// selection 来源从被点击行取值;route 来源从当前 route 透传(如 detail → mapping)。
pub fn build_route(
    bindings: &BTreeMap<String, ResourceWorkbenchBinding>,
    current_route: &serde_json::Value,
    selection: &serde_json::Value,
) -> serde_json::Value {
    let mut route = serde_json::Map::new();
    for (name, binding) in bindings {
        let value = match binding.source {
            ResourceWorkbenchBindingSource::Selection => pick(selection, &binding.path),
            ResourceWorkbenchBindingSource::Route => pick(current_route, &binding.path),
            ResourceWorkbenchBindingSource::Literal => {
                binding.value.clone().unwrap_or(serde_json::Value::Null)
            }
            ResourceWorkbenchBindingSource::Input | ResourceWorkbenchBindingSource::Paging => {
                serde_json::Value::Null
            }
        };
        if !value.is_null() {
            route.insert(name.clone(), value);
        }
    }
    serde_json::Value::Object(route)
}

fn pick(value: &serde_json::Value, path: &str) -> serde_json::Value {
    if path.is_empty() {
        return value.clone();
    }
    // manifest 声明 `/name` 风格;统一为带前导斜杠的 json pointer。
    let normalized = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    value
        .pointer(&normalized)
        .cloned()
        .unwrap_or(serde_json::Value::Null)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn binding(source: ResourceWorkbenchBindingSource, path: &str) -> ResourceWorkbenchBinding {
        ResourceWorkbenchBinding {
            source,
            path: path.into(),
            value_type: extension_runtime::extension::manifest::ResourceWorkbenchValueType::String,
            value: None,
        }
    }

    #[test]
    fn builds_route_from_selection_row() {
        let bindings = [(
            "name".to_string(),
            binding(ResourceWorkbenchBindingSource::Selection, "/name"),
        )]
        .into_iter()
        .collect();
        let row = serde_json::json!({"name": "orders-2026", "health": "green"});

        let route = build_route(&bindings, &serde_json::Value::Null, &row);

        assert_eq!(serde_json::json!({"name": "orders-2026"}), route);
    }

    #[test]
    fn forwards_route_param_for_links() {
        let bindings = [(
            "name".to_string(),
            binding(ResourceWorkbenchBindingSource::Route, "/name"),
        )]
        .into_iter()
        .collect();
        let current = serde_json::json!({"name": "orders-2026"});

        let route = build_route(&bindings, &current, &serde_json::Value::Null);

        assert_eq!(current, route);
    }
}
