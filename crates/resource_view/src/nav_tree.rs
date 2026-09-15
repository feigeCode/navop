//! 左侧导航树:静态根 + 任意层级 lazy children。
//!
//! 每个节点用 `节点键` 定位(根 = root.id;子节点 = 父键 + `\u{1}` + 行键),
//! 展开状态与拉取结果按节点键缓存。子节点拉取时把父行数据放进
//! `BindingContext::parent`,供 manifest 的 `parent` 绑定源取值。

use extension_plugin_adapter::{BindingContext, WorkbenchDispatchError};
use extension_runtime::extension::manifest::ResourceWorkbenchTreeChildren;
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt as _, h_flex, spinner::Spinner, v_flex,
};

use crate::layout::{NavContent, ResolvedTreeRoot, TreeChildRow};
use crate::{NEXT_MOUNT_ID, NativeResourceWorkbench, collection_table, route_binding};

const KEY_SEP: char = '\u{1}';

/// 节点子节点加载状态。
#[derive(Debug, Clone)]
pub(crate) enum TreeChildrenState {
    Loading,
    Loaded(Vec<TreeChildRow>),
    Failed(String),
}

/// 一次子节点拉取/点击所需的节点定位信息。
#[derive(Debug, Clone)]
struct TreeNodeRef {
    key: String,
    root_page_id: String,
    /// 该节点自身的行数据;根节点为 Null。
    row: serde_json::Value,
    /// 拉取该节点子节点用的声明;叶子为 None。
    children: Option<ResourceWorkbenchTreeChildren>,
    /// 点击该节点时使用的 open 声明(来自父级 children.open)。
    open: Option<extension_runtime::extension::manifest::ResourceWorkbenchOpen>,
    /// 父节点行数据;根/一级子节点为 Null。
    parent_row: serde_json::Value,
}

/// 树子节点的命中判定:该行点击后将到达的 `(页面, 路由)` 是否等于当前选中。
///
/// 必须与导航目标完全一致——只比较目标页面 id 会让所有指向同一页面的
/// 兄弟节点一起高亮。
pub(crate) fn tree_child_is_active(
    root_page_id: &str,
    open: Option<&extension_runtime::extension::manifest::ResourceWorkbenchOpen>,
    row: &serde_json::Value,
    parent_row: &serde_json::Value,
    selected_page: &str,
    current_route: &serde_json::Value,
) -> bool {
    let (target_page, target_route) =
        navigation_target(root_page_id, open, row, parent_row, current_route);
    target_page == selected_page && target_route == *current_route
}

fn navigation_target(
    root_page_id: &str,
    open: Option<&extension_runtime::extension::manifest::ResourceWorkbenchOpen>,
    row: &serde_json::Value,
    parent_row: &serde_json::Value,
    current_route: &serde_json::Value,
) -> (String, serde_json::Value) {
    match open {
        Some(open) => (
            open.page_id.clone(),
            route_binding::build_route_with_parent(&open.route, current_route, row, parent_row),
        ),
        None => (root_page_id.to_string(), row.clone()),
    }
}

impl NativeResourceWorkbench {
    fn tree_roots(&self) -> &[ResolvedTreeRoot] {
        match self.layout.left.as_ref().map(|left| &left.content) {
            Some(NavContent::Tree { roots }) => roots,
            _ => &[],
        }
    }

    /// 按节点键回溯到节点引用:根键直接命中;子节点键沿已加载缓存逐层下探。
    fn resolve_tree_node(&self, key: &str) -> Option<TreeNodeRef> {
        let mut parts = key.split(KEY_SEP);
        let root_id = parts.next()?;
        let root = self.tree_roots().iter().find(|root| root.id == root_id)?;
        let mut node = TreeNodeRef {
            key: root.id.clone(),
            root_page_id: root.page_id.clone(),
            row: serde_json::Value::Null,
            children: root.children.clone(),
            open: None,
            parent_row: serde_json::Value::Null,
        };
        for row_key in parts {
            let children = node.children.take()?;
            let Some(TreeChildrenState::Loaded(rows)) = self.tree_children.get(&node.key) else {
                return None;
            };
            let row = rows.iter().find(|row| row.key == row_key)?;
            node = TreeNodeRef {
                key: format!("{}{KEY_SEP}{}", node.key, row.key),
                root_page_id: node.root_page_id,
                parent_row: node.row,
                row: row.value.clone(),
                open: children.open.clone(),
                children: children.children.map(|next| *next),
            };
        }
        Some(node)
    }

    /// 拉取节点的子节点:走 dispatcher 执行声明的 operation,父行数据作
    /// `parent` 绑定源,结果按 itemsPath/labelPath/keyPaths 投影成行。
    fn load_tree_children(&mut self, node: &TreeNodeRef, cx: &mut Context<Self>) {
        let Some(children) = node.children.clone() else {
            return;
        };
        if matches!(
            self.tree_children.get(&node.key),
            Some(TreeChildrenState::Loading | TreeChildrenState::Loaded(_))
        ) {
            return;
        }
        self.tree_children
            .insert(node.key.clone(), TreeChildrenState::Loading);
        cx.notify();

        let mount_id = NEXT_MOUNT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let scope = self
            .session
            .scope(format!("__tree__{}", node.key), mount_id);
        let workbench = self.descriptor.clone();
        let context = BindingContext {
            paging: serde_json::json!({"page": 1, "limit": 200, "cursor": null}),
            parent: node.row.clone(),
            ..Default::default()
        };
        let operation = children.operation.clone();
        let node_key = node.key.clone();
        let tokio = self.tokio.clone();
        cx.spawn(async move |this, cx| {
            let result = tokio
                .spawn(async move {
                    extension_plugin_adapter::dispatch_invoke_scoped(
                        &scope, &workbench, &operation, &context, false,
                    )
                    .await
                })
                .await
                .unwrap_or_else(|join_error| {
                    Err(WorkbenchDispatchError::Provider(join_error.to_string()))
                });
            let _ = this.update(cx, |this, cx| {
                let state = match result {
                    Ok(value) => TreeChildrenState::Loaded(
                        collection_table::items_at(&value, &children.items_path)
                            .into_iter()
                            .map(|row| TreeChildRow {
                                key: collection_table::row_key(&row, &children.key_paths),
                                label: row
                                    .pointer(&children.label_path)
                                    .map(crate::plain_text)
                                    .unwrap_or_default(),
                                value: row,
                            })
                            .collect(),
                    ),
                    Err(error) => TreeChildrenState::Failed(error.to_string()),
                };
                this.tree_children.insert(node_key, state);
                cx.notify();
            });
        })
        .detach();
    }

    /// 折叠节点:移除自身与全部后代缓存。
    fn collapse_tree_node(&mut self, key: &str) {
        let prefix = format!("{key}{KEY_SEP}");
        self.tree_children
            .retain(|k, _| k != key && !k.starts_with(&prefix));
    }

    /// 节点点击:有 children 声明则切换展开;随后按 open 声明导航
    /// (根节点缺省选中根页面,子节点缺省回退根页面 + 行数据作 route)。
    fn on_tree_node_click(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(node) = self.resolve_tree_node(key) else {
            return;
        };
        if node.children.is_some() {
            if self.tree_children.contains_key(&node.key) {
                self.collapse_tree_node(&node.key);
            } else {
                self.load_tree_children(&node, cx);
            }
        }
        if node.row.is_null() {
            self.select_page(node.root_page_id.clone(), cx);
            return;
        }
        let (page, route) = navigation_target(
            &node.root_page_id,
            node.open.as_ref(),
            &node.row,
            &node.parent_row,
            &self.route,
        );
        self.navigate(page, route, cx);
    }

    /// 左侧树入口:遍历根节点,递归渲染已展开层级。
    pub(crate) fn render_nav_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut list = v_flex().w_full().gap_0p5();
        for root in self.tree_roots().to_vec() {
            let node = TreeNodeRef {
                key: root.id.clone(),
                root_page_id: root.page_id.clone(),
                row: serde_json::Value::Null,
                children: root.children.clone(),
                open: None,
                parent_row: serde_json::Value::Null,
            };
            list = list.child(self.render_tree_node(&node, &root.title, 0, cx));
            list = self.render_tree_children(list, &node, 1, cx);
        }
        list
    }

    fn render_tree_children(
        &mut self,
        mut list: gpui::Div,
        node: &TreeNodeRef,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        let theme = cx.theme().clone();
        let indent = gpui::px(8.0 + depth as f32 * 16.0);
        match self.tree_children.get(&node.key).cloned() {
            Some(TreeChildrenState::Loading) => list.child(
                h_flex()
                    .id(SharedString::from(format!("tree-loading-{}", node.key)))
                    .w_full()
                    .pl(indent)
                    .py_1()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(Spinner::new().with_size(Size::XSmall)),
            ),
            Some(TreeChildrenState::Failed(error)) => list.child(
                h_flex()
                    .id(SharedString::from(format!("tree-error-{}", node.key)))
                    .w_full()
                    .pl(indent)
                    .py_1()
                    .text_xs()
                    .text_color(theme.danger)
                    .child(error),
            ),
            Some(TreeChildrenState::Loaded(rows)) => {
                let Some(children) = node.children.clone() else {
                    return list;
                };
                for row in rows {
                    let child = TreeNodeRef {
                        key: format!("{}{KEY_SEP}{}", node.key, row.key),
                        root_page_id: node.root_page_id.clone(),
                        parent_row: node.row.clone(),
                        row: row.value.clone(),
                        open: children.open.clone(),
                        children: children.children.clone().map(|next| *next),
                    };
                    list = list.child(self.render_tree_node(&child, &row.label, depth, cx));
                    list = self.render_tree_children(list, &child, depth + 1, cx);
                }
                list
            }
            None => list,
        }
    }

    fn render_tree_node(
        &mut self,
        node: &TreeNodeRef,
        title: &str,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let is_root = node.row.is_null();
        let active = if is_root {
            node.root_page_id == self.selected_page
        } else {
            tree_child_is_active(
                &node.root_page_id,
                node.open.as_ref(),
                &node.row,
                &node.parent_row,
                &self.selected_page,
                &self.route,
            )
        };
        let has_children = node.children.is_some();
        let expanded = self
            .tree_children
            .get(&node.key)
            .is_some_and(|state| !matches!(state, TreeChildrenState::Failed(_)));
        let key = node.key.clone();
        h_flex()
            .id(SharedString::from(format!("tree-node-{}", node.key)))
            .w_full()
            .min_w_0()
            .pl(gpui::px(8.0 + depth as f32 * 16.0))
            .pr_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .when(is_root, |this| this.text_sm())
            .when(!is_root, |this| this.text_xs())
            .text_color(if active {
                theme.sidebar_accent_foreground
            } else {
                theme.sidebar_foreground
            })
            .when(active, |this| this.bg(theme.sidebar_accent))
            .when(active && is_root, |this| this.font_medium())
            .when(!active, |this| {
                let hover = theme.list_hover;
                this.hover(move |this| this.bg(hover))
            })
            .when(has_children, |this| {
                this.child(
                    Icon::new(if expanded {
                        IconName::ChevronDown
                    } else {
                        IconName::ChevronRight
                    })
                    .size_3()
                    .text_color(theme.muted_foreground),
                )
            })
            .child(div().min_w_0().truncate().child(title.to_string()))
            .on_click(cx.listener(
                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                    this.on_tree_node_click(&key, cx);
                },
            ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::extension::manifest::{
        ResourceWorkbenchBinding, ResourceWorkbenchBindingSource, ResourceWorkbenchOpen,
        ResourceWorkbenchValueType,
    };
    use serde_json::json;
    use std::collections::BTreeMap;

    fn binding(source: ResourceWorkbenchBindingSource, path: &str) -> ResourceWorkbenchBinding {
        ResourceWorkbenchBinding {
            source,
            path: path.into(),
            value_type: ResourceWorkbenchValueType::String,
            value: None,
        }
    }

    fn open_by_id() -> ResourceWorkbenchOpen {
        ResourceWorkbenchOpen {
            page_id: "container-detail".into(),
            route: BTreeMap::from([(
                "id".to_string(),
                binding(ResourceWorkbenchBindingSource::Selection, "/id"),
            )]),
        }
    }

    #[test]
    fn only_the_selected_sibling_is_active() {
        let open = open_by_id();
        let rows = [json!({"id": "abc"}), json!({"id": "xyz"})];
        let current = json!({"id": "abc"});
        let null = serde_json::Value::Null;
        assert!(tree_child_is_active(
            "containers",
            Some(&open),
            &rows[0],
            &null,
            "container-detail",
            &current
        ));
        assert!(!tree_child_is_active(
            "containers",
            Some(&open),
            &rows[1],
            &null,
            "container-detail",
            &current
        ));
    }

    #[test]
    fn no_row_is_active_on_an_unrelated_page() {
        let open = open_by_id();
        let row = json!({"id": "abc"});
        let null = serde_json::Value::Null;
        assert!(!tree_child_is_active(
            "containers",
            Some(&open),
            &row,
            &null,
            "containers",
            &json!({"id": "abc"})
        ));
        assert!(!tree_child_is_active(
            "containers",
            Some(&open),
            &row,
            &null,
            "container-detail",
            &json!({"id": "other"})
        ));
    }

    #[test]
    fn child_without_open_matches_root_page_and_row_route() {
        let row = json!({"id": "abc"});
        let null = serde_json::Value::Null;
        assert!(tree_child_is_active(
            "containers",
            None,
            &row,
            &null,
            "containers",
            &row
        ));
        assert!(!tree_child_is_active(
            "containers",
            None,
            &row,
            &null,
            "containers",
            &json!({"id": "zzz"})
        ));
    }

    #[test]
    fn nested_child_route_includes_parent_binding() {
        let open = ResourceWorkbenchOpen {
            page_id: "pod-detail".into(),
            route: BTreeMap::from([
                (
                    "namespace".to_string(),
                    binding(ResourceWorkbenchBindingSource::Parent, "/name"),
                ),
                (
                    "pod".to_string(),
                    binding(ResourceWorkbenchBindingSource::Selection, "/name"),
                ),
            ]),
        };
        let parent = json!({"name": "default"});
        let row = json!({"name": "api-0"});
        let current = json!({"namespace": "default", "pod": "api-0"});
        assert!(tree_child_is_active(
            "namespaces",
            Some(&open),
            &row,
            &parent,
            "pod-detail",
            &current
        ));
        assert!(!tree_child_is_active(
            "namespaces",
            Some(&open),
            &row,
            &json!({"name": "kube-system"}),
            "pod-detail",
            &current
        ));
    }
}
