//! Route 构造:按声明绑定把 route/selection/parent/connection 投影为目标页路由。

use extension_runtime::extension::manifest::{
    ResourceWorkbenchBinding, ResourceWorkbenchBindingSource,
};
use std::collections::BTreeMap;

/// 尚未取值的来源统一指向这个常量,免去每个调用点自造临时 `Null`。
static NO_VALUE: serde_json::Value = serde_json::Value::Null;

/// 构造目标页路由时的取值来源。
///
/// 与 `BindingContext` 同一组来源:provider 参数侧取得到的,路由侧也必须取得到。
/// 做成结构体而不是逐个加位置参数,是因为**漏一个来源不会报错,只会静默丢弃**:
/// `connection` 当初就是在参数侧加了字段、路由侧继续按 `Null` 处理,
/// 扩展声明了却永远拿不到值。新增来源时两边一起加,结构体让这件事显式。
pub struct RouteSources<'a> {
    /// 当前页路由:供 `source: route` 透传(如 detail → mapping)。
    pub route: &'a serde_json::Value,
    /// 被点击行:供 `source: selection`。
    pub selection: &'a serde_json::Value,
    /// 树 lazy 展开时的父节点行:供 `source: parent`。
    pub parent: &'a serde_json::Value,
    /// 连接的保存配置:供 `source: connection`。
    pub connection: &'a serde_json::Value,
}

impl<'a> RouteSources<'a> {
    /// 只带当前路由与连接的来源:用于 links / tabs 这类没有行上下文的导航。
    ///
    /// `connection` 是显式参数而不是内部置空——它不依赖行上下文,漏传就等于
    /// "声明了 `source: connection` 的 tab 永远拿不到值",正是这条 bug 的成因。
    /// 需要空值必须由调用方明确写 `&serde_json::Value::Null`。
    pub fn without_row_context(
        current_route: &'a serde_json::Value,
        connection: &'a serde_json::Value,
    ) -> Self {
        Self {
            route: current_route,
            selection: &NO_VALUE,
            parent: &NO_VALUE,
            connection,
        }
    }
}

/// 按 open/link 声明构造目标页 route。
pub fn build_route(
    bindings: &BTreeMap<String, ResourceWorkbenchBinding>,
    sources: &RouteSources<'_>,
) -> serde_json::Value {
    let mut route = serde_json::Map::new();
    for (name, binding) in bindings {
        let value = match binding.source {
            ResourceWorkbenchBindingSource::Selection => pick(sources.selection, &binding.path),
            ResourceWorkbenchBindingSource::Route => pick(sources.route, &binding.path),
            ResourceWorkbenchBindingSource::Parent => pick(sources.parent, &binding.path),
            ResourceWorkbenchBindingSource::Connection => pick(sources.connection, &binding.path),
            ResourceWorkbenchBindingSource::Literal => {
                binding.value.clone().unwrap_or(serde_json::Value::Null)
            }
            // input/paging 只在 provider 参数侧有意义:导航的触发点是行点击或
            // 链接,没有表单输入,也没有列表分页上下文。注册期已拒绝这两个来源
            // 出现在 route 绑定里,所以这里只是不 panic 的兜底。
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

        let route = build_route(
            &bindings,
            &RouteSources {
                selection: &row,
                ..RouteSources::without_row_context(
                    &serde_json::Value::Null,
                    &serde_json::Value::Null,
                )
            },
        );

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

        let route = build_route(
            &bindings,
            &RouteSources::without_row_context(&current, &serde_json::Value::Null),
        );

        assert_eq!(current, route);
    }

    #[test]
    fn merges_parent_and_selection_for_nested_tree_nodes() {
        let bindings = [
            (
                "namespace".to_string(),
                binding(ResourceWorkbenchBindingSource::Parent, "/name"),
            ),
            (
                "pod".to_string(),
                binding(ResourceWorkbenchBindingSource::Selection, "/name"),
            ),
        ]
        .into_iter()
        .collect();
        let parent = serde_json::json!({"name": "default"});
        let row = serde_json::json!({"name": "api-0"});

        let route = build_route(
            &bindings,
            &RouteSources {
                selection: &row,
                parent: &parent,
                ..RouteSources::without_row_context(
                    &serde_json::Value::Null,
                    &serde_json::Value::Null,
                )
            },
        );

        assert_eq!(
            serde_json::json!({"namespace": "default", "pod": "api-0"}),
            route
        );
    }

    #[test]
    fn tab_route_keeps_connection_binding() {
        // 回归:links/tabs 这类导航没有行上下文,曾经顺带把 connection 也置空 ——
        // 于是 tab 上的 `source: connection` 绑定"声明合法、永远拿不到值"。
        // 无行上下文 != 无连接上下文。
        let bindings = [(
            "database".to_string(),
            binding(ResourceWorkbenchBindingSource::Connection, "/database"),
        )]
        .into_iter()
        .collect();
        let connection = serde_json::json!({"namespace": "prod", "database": "orders"});

        let route = build_route(
            &bindings,
            &RouteSources::without_row_context(&serde_json::Value::Null, &connection),
        );

        assert_eq!(serde_json::json!({"database": "orders"}), route);
    }

    #[test]
    fn builds_route_from_connection_config() {
        // 回归:`source: connection` 曾经被无条件当成 Null 丢掉,扩展声明了
        // 也拿不到值(参数侧支持、路由侧静默忽略)。
        let bindings = [(
            "namespace".to_string(),
            binding(ResourceWorkbenchBindingSource::Connection, "/namespace"),
        )]
        .into_iter()
        .collect();
        let connection = serde_json::json!({"namespace": "prod", "database": "orders"});

        let route = build_route(
            &bindings,
            &RouteSources {
                connection: &connection,
                ..RouteSources::without_row_context(
                    &serde_json::Value::Null,
                    &serde_json::Value::Null,
                )
            },
        );

        assert_eq!(serde_json::json!({"namespace": "prod"}), route);
    }
}
