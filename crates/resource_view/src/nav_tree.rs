//! 左侧导航树:静态根 + 任意层级 children(remote lazy 拉取 / static 声明期给定)。
//!
//! 每个节点用 `节点键` 定位(根 = root.id;子节点 = 父键 + `\u{1}` + 行键)。
//! **展开意图与子节点缓存是两份状态**:`tree_expanded` 记录哪些节点是打开的,
//! `tree_children` 缓存数据。折叠只把键从展开集合里去掉、不删缓存,所以
//! "展开 → 折叠 → 展开"不会重复请求;刷新则显式清缓存再重拉。
//!
//! 子节点来源由 `children.kind` 决定:
//! - `remote`:执行声明的 operation,父行数据放进 `BindingContext::parent`;
//! - `static`:声明期给定的功能子项,展开即得,零请求。
//!
//! 静态项的 `parent` 是**直接父节点**的领域行(如「索引 → Mapping」的索引行),
//! 它自己的行数据只是稳定元数据 `{id}`,不冒充领域行 —— 否则 `source: selection`
//! 会悄悄取到一个除了 id 什么都没有的对象。

use extension_plugin_adapter::{BindingContext, WorkbenchDispatchError};
use extension_runtime::extension::manifest::{
    ResourceWorkbenchOpen, ResourceWorkbenchTreeChildren,
};
use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, div, prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme, Icon, IconName, Sizable, Size, StyledExt as _, h_flex, spinner::Spinner, v_flex,
};

use crate::layout::{NavContent, ResolvedTreeRoot, TreeChildRow};
use crate::route_binding::RouteSources;
use crate::{NEXT_MOUNT_ID, NativeResourceWorkbench, collection_table, route_binding};

const KEY_SEP: char = '\u{1}';

/// 树行在 tab 序列里的序号基准:按深度错开,不让同一层所有行共用一个序号。
const TREE_TAB_INDEX_BASE: isize = 1000;

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
    /// 该节点自身的行数据;静态项是 `{id}` 元数据,根节点为 Null。
    row: serde_json::Value,
    /// 该节点子节点的声明;叶子为 None。
    children: Option<ResourceWorkbenchTreeChildren>,
    /// 该节点自身的跳转声明;缺省时点击回退到根页面 + 行数据作 route。
    open: Option<ResourceWorkbenchOpen>,
    /// 父节点行数据;根/一级子节点为 Null。
    parent_row: serde_json::Value,
}

/// 节点键 = 父键 + 分隔符 + 行键。行键来自 keyPaths(远程)或 item id(静态)。
fn child_node_key(parent_key: &str, row_key: &str) -> String {
    format!("{parent_key}{KEY_SEP}{row_key}")
}

/// 静态集合 → 行。零请求:展开那一刻同步构造。
///
/// 每项自带 `open`,因此同一层里的功能子项可以各自指向不同页面 —— 这是静态形态
/// 与远程形态最本质的差别(远程的 `open` 是集合级、所有行共用一份)。
fn static_child_rows(children: &ResourceWorkbenchTreeChildren) -> Vec<TreeChildRow> {
    children
        .static_items()
        .unwrap_or_default()
        .iter()
        .map(|item| TreeChildRow {
            key: item.id.clone(),
            label: item.title.clone(),
            value: serde_json::json!({ "id": item.id }),
            open: Some(item.open.clone()),
            children: item.children.as_deref().cloned(),
        })
        .collect()
}

/// provider 结果 → 行:按 itemsPath 取集合,labelPath 取标签,keyPaths 取行键。
fn remote_child_rows(
    value: &serde_json::Value,
    children: &ResourceWorkbenchTreeChildren,
) -> Vec<TreeChildRow> {
    let items_path = children.items_path.as_deref().unwrap_or_default();
    let label_path = children.label_path.as_deref().unwrap_or_default();
    collection_table::items_at(value, items_path)
        .into_iter()
        .map(|row| TreeChildRow {
            key: collection_table::row_key(&row, &children.key_paths),
            label: row
                .pointer(label_path)
                .map(crate::plain_text)
                .unwrap_or_default(),
            value: row,
            open: children.open.clone(),
            children: children.children.as_deref().cloned(),
        })
        .collect()
}

/// 键盘动作:只映射到鼠标已有的语义,不引入第三套行为。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TreeKeyAction {
    /// 与点击整行一致:导航。
    Activate,
    /// 与点击箭头一致:展开。
    Expand,
    /// 与点击箭头一致:折叠。
    Collapse,
}

impl TreeKeyAction {
    fn from_key(key: &str) -> Option<Self> {
        match key {
            "enter" | "space" => Some(Self::Activate),
            "right" => Some(Self::Expand),
            "left" => Some(Self::Collapse),
            _ => None,
        }
    }
}

/// 树子节点的命中判定:该行点击后将到达的 `(页面, 路由)` 是否等于当前选中。
///
/// 必须与导航目标完全一致——只比较目标页面 id 会让所有指向同一页面的
/// 兄弟节点一起高亮。
pub(crate) fn tree_child_is_active(
    root_page_id: &str,
    open: Option<&extension_runtime::extension::manifest::ResourceWorkbenchOpen>,
    row: &serde_json::Value,
    selected_page: &str,
    sources: &RouteSources<'_>,
) -> bool {
    let (target_page, target_route) = navigation_target(root_page_id, open, row, sources);
    target_page == selected_page && target_route == *sources.route
}

fn navigation_target(
    root_page_id: &str,
    open: Option<&extension_runtime::extension::manifest::ResourceWorkbenchOpen>,
    row: &serde_json::Value,
    sources: &RouteSources<'_>,
) -> (String, serde_json::Value) {
    match open {
        Some(open) => (
            open.page_id.clone(),
            route_binding::build_route(&open.route, sources),
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

    fn root_node(&self, root: &ResolvedTreeRoot) -> TreeNodeRef {
        TreeNodeRef {
            key: root.id.clone(),
            root_page_id: root.page_id.clone(),
            row: serde_json::Value::Null,
            children: root.children.clone(),
            open: None,
            parent_row: serde_json::Value::Null,
        }
    }

    /// 按节点键回溯到节点引用:根键直接命中;子节点键沿已加载缓存逐层下探。
    ///
    /// 回溯只读缓存、不发请求 —— 解析不到就说明这一层还没加载完,调用方按
    /// "暂时解析不到"处理(见 `resume_tree_expansions`)。
    fn resolve_tree_node(&self, key: &str) -> Option<TreeNodeRef> {
        let mut parts = key.split(KEY_SEP);
        let root_id = parts.next()?;
        let root = self.tree_roots().iter().find(|root| root.id == root_id)?;
        let mut node = self.root_node(root);
        for row_key in parts {
            let Some(TreeChildrenState::Loaded(rows)) = self.tree_children.get(&node.key) else {
                return None;
            };
            let row = rows.iter().find(|row| row.key == row_key)?;
            node = TreeNodeRef {
                key: child_node_key(&node.key, &row.key),
                root_page_id: node.root_page_id,
                parent_row: node.row,
                row: row.value.clone(),
                open: row.open.clone(),
                children: row.children.clone(),
            };
        }
        Some(node)
    }

    /// 记一次加载代次。同键的新一轮加载会让旧轮次的迟到响应失效。
    ///
    /// 折叠不再丢缓存,所以"代次"就是唯一的作废手段:刷新时把整张 seq 表清空,
    /// 查不到代次的响应一律丢弃。
    fn begin_tree_load(&mut self, key: &str) -> u64 {
        self.tree_load_generation = self.tree_load_generation.wrapping_add(1);
        let generation = self.tree_load_generation;
        self.tree_load_seq.insert(key.to_string(), generation);
        generation
    }

    /// 拉取节点的子节点。
    ///
    /// 静态集合在这里直接出结果并返回,不会走到 provider 分支:静态项没有
    /// operation 可发,一旦掉进远程分支就是"点开是空的"。
    fn load_tree_children(&mut self, node: &TreeNodeRef, cx: &mut Context<Self>) {
        let Some(children) = node.children.clone() else {
            return;
        };
        if !children.is_remote() {
            self.tree_children.insert(
                node.key.clone(),
                TreeChildrenState::Loaded(static_child_rows(&children)),
            );
            cx.notify();
            return;
        }
        let Some(operation) = children
            .operation
            .clone()
            .filter(|operation| !operation.trim().is_empty())
        else {
            // 注册期已保证远程形态必有 operation。真走到这里说明声明绕过了校验,
            // 报成失败节点(可重试)而不是静默当叶子。
            self.tree_children.insert(
                node.key.clone(),
                TreeChildrenState::Failed("tree children declare no operation".to_string()),
            );
            cx.notify();
            return;
        };
        let generation = self.begin_tree_load(&node.key);
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
            // `..Default::default()` 会把 connection 留成 Null,于是树 lazy 展开的
            // operation 一旦声明 `source: connection` 就报 BindingMissing ——
            // 和其它构造点一样,这里必须填宿主注入的真实值。
            connection: self.connection.clone(),
            ..Default::default()
        };
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
                // 迟到响应隔离:折叠、刷新或同键重拉都会让代次前进,旧轮次的结果
                // 直接丢弃,不写回已经不是它的子树。
                if this.tree_load_seq.get(&node_key) != Some(&generation) {
                    return;
                }
                let state = match result {
                    Ok(value) => TreeChildrenState::Loaded(remote_child_rows(&value, &children)),
                    Err(error) => TreeChildrenState::Failed(error.to_string()),
                };
                this.tree_children.insert(node_key, state);
                this.resume_tree_expansions(cx);
                cx.notify();
            });
        })
        .detach();
    }

    /// 确保节点子数据就绪:已有缓存(含在途加载)不重复拉取。
    ///
    /// `Failed` 不算就绪 —— 展开一个失败节点等价于"再试一次"。
    fn ensure_tree_children(&mut self, node: &TreeNodeRef, cx: &mut Context<Self>) {
        if matches!(
            self.tree_children.get(&node.key),
            Some(TreeChildrenState::Loaded(_) | TreeChildrenState::Loading)
        ) {
            return;
        }
        self.load_tree_children(node, cx);
    }

    /// 展开节点。折叠只隐藏、缓存保留,所以重复展开是零成本的。
    fn expand_tree_node(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(node) = self.resolve_tree_node(key) else {
            return;
        };
        if node.children.is_none() {
            return;
        }
        self.tree_expanded.insert(node.key.clone());
        self.ensure_tree_children(&node, cx);
        cx.notify();
    }

    /// 折叠节点:只把键移出展开集合,不动 `tree_children`。
    fn collapse_tree_node(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.tree_expanded.remove(key) {
            cx.notify();
        }
    }

    fn toggle_tree_node(&mut self, key: &str, cx: &mut Context<Self>) {
        if self.tree_expanded.contains(key) {
            self.collapse_tree_node(key, cx);
        } else {
            self.expand_tree_node(key, cx);
        }
    }

    /// 失败节点重试:强制重新加载(绕过 `ensure_tree_children` 的就绪判断)。
    fn retry_tree_children(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(node) = self.resolve_tree_node(key) else {
            return;
        };
        self.load_tree_children(&node, cx);
    }

    /// 刷新整棵树:缓存失效后从根重拉,并恢复仍然存在的展开态。
    ///
    /// 不能只 `tree_children.clear()`:节点键是结构化的(父键 + 行键),清空缓存
    /// 后子节点的行数据无从回溯,展开态会整体丢失。这里先快照展开键,清空后
    /// 由根往下重新加载,再逐批恢复能解析到的键。
    fn refresh_tree(&mut self, cx: &mut Context<Self>) {
        let mut pending: Vec<String> = self.tree_expanded.iter().cloned().collect();
        // 浅的先展开,深的才解析得到。
        pending.sort_by_key(|key| key.matches(KEY_SEP).count());
        self.tree_children.clear();
        self.tree_load_seq.clear();
        self.tree_pending_expand = pending.into_iter().collect();
        for root in self.tree_roots().to_vec() {
            let node = self.root_node(&root);
            if node.children.is_some() {
                self.load_tree_children(&node, cx);
            }
        }
        self.resume_tree_expansions(cx);
        cx.notify();
    }

    /// 把刷新前快照下来的展开键恢复到"当前缓存能解析到"为止。
    ///
    /// 远程分支是异步回来的,一次调用只覆盖已加载到的层;每批加载完成后都会
    /// 再调用一次,直到没有新进展。集合为空时是零开销的早退。
    fn resume_tree_expansions(&mut self, cx: &mut Context<Self>) {
        loop {
            let mut progressed = false;
            for key in self.tree_pending_expand.iter().cloned().collect::<Vec<_>>() {
                let Some(node) = self.resolve_tree_node(&key) else {
                    continue;
                };
                self.tree_pending_expand.remove(&key);
                if node.children.is_none() {
                    continue;
                }
                self.tree_expanded.insert(node.key.clone());
                self.load_tree_children(&node, cx);
                progressed = true;
            }
            if !progressed {
                return;
            }
        }
    }

    /// 整行点击:只导航。
    ///
    /// 展开/折叠归箭头。两者合一时,点"有子节点的行"会同时展开并跳页 ——
    /// 一个动作两个副作用,用户想翻子项却被迫换了页面。
    fn on_tree_node_click(&mut self, key: &str, cx: &mut Context<Self>) {
        let Some(node) = self.resolve_tree_node(key) else {
            return;
        };
        if node.row.is_null() {
            self.select_page(node.root_page_id.clone(), cx);
            return;
        }
        let (page, route) = navigation_target(
            &node.root_page_id,
            node.open.as_ref(),
            &node.row,
            &self.route_sources(&node.row, &node.parent_row),
        );
        self.navigate(page, route, cx);
    }

    /// 左侧树入口:刷新按钮 + 遍历根节点,递归渲染已展开层级。
    pub(crate) fn render_nav_tree(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let mut list = v_flex().w_full().gap_0p5();
        for root in self.tree_roots().to_vec() {
            let node = self.root_node(&root);
            list = list.child(self.render_tree_node(&node, &root.title, 0, cx));
            list = self.render_tree_children(list, &node, 1, cx);
        }
        v_flex()
            .w_full()
            .gap_0p5()
            .child(self.render_tree_header(cx))
            .child(list)
    }

    fn render_tree_header(&mut self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = cx.theme().clone();
        let hover = theme.list_hover;
        h_flex()
            .w_full()
            .min_w_0()
            .justify_end()
            .px_2()
            .pb_1()
            .child(
                h_flex()
                    .id("tree-refresh")
                    .px_1()
                    .py_0p5()
                    .rounded(theme.radius)
                    .cursor_pointer()
                    .text_color(theme.muted_foreground)
                    .hover(move |this| this.bg(hover))
                    .on_click(cx.listener(|this: &mut Self, _event, _window, cx| {
                        this.refresh_tree(cx);
                    }))
                    // `gpui_component::IconName` 是精选子集(完整 Lucide 目录在
                    // `gpui_kit_assets::IconName`),没有 `RefreshCw`;语义等价的
                    // 「刷新」图标是 `RotateCw`。
                    .child(Icon::new(IconName::RotateCw).size_3()),
            )
    }

    fn render_tree_children(
        &mut self,
        mut list: gpui::Div,
        node: &TreeNodeRef,
        depth: usize,
        cx: &mut Context<Self>,
    ) -> gpui::Div {
        // 折叠态在这里短路:不再往下走,也不渲染任何一层的行。
        if !self.tree_expanded.contains(&node.key) {
            return list;
        }
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
            Some(TreeChildrenState::Failed(error)) => {
                let retry_key = node.key.clone();
                list.child(
                    h_flex()
                        .id(SharedString::from(format!("tree-error-{}", node.key)))
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .pl(indent)
                        .py_1()
                        .text_xs()
                        .text_color(theme.danger)
                        .child(div().min_w_0().truncate().child(error))
                        .child(
                            div()
                                .id(SharedString::from(format!("tree-retry-{}", node.key)))
                                .flex_shrink_0()
                                .cursor_pointer()
                                .underline()
                                .on_click(cx.listener(
                                    move |this: &mut Self, _event, _window, cx| {
                                        cx.stop_propagation();
                                        this.retry_tree_children(&retry_key, cx);
                                    },
                                ))
                                .child("retry"),
                        ),
                )
            }
            Some(TreeChildrenState::Loaded(rows)) => {
                for row in rows {
                    let child = TreeNodeRef {
                        key: child_node_key(&node.key, &row.key),
                        root_page_id: node.root_page_id.clone(),
                        parent_row: node.row.clone(),
                        row: row.value.clone(),
                        open: row.open.clone(),
                        children: row.children.clone(),
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
                &self.selected_page,
                &self.route_sources(&node.row, &node.parent_row),
            )
        };
        let has_children = node.children.is_some();
        let expanded = self.tree_expanded.contains(&node.key);
        let key = node.key.clone();
        let toggle_key = key.clone();
        let focus_key = key.clone();
        let entity = cx.entity().downgrade();
        h_flex()
            .id(SharedString::from(format!("tree-node-{}", node.key)))
            .w_full()
            .min_w_0()
            .pl(gpui::px(8.0 + depth as f32 * 16.0))
            .pr_2()
            .py_1()
            .rounded(theme.radius)
            .cursor_pointer()
            .focusable()
            .tab_index(TREE_TAB_INDEX_BASE + depth as isize)
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
                // 箭头是**独立**的点击目标:命中它时先于整行收到事件,并显式阻止
                // 冒泡,否则一次点击会既展开又导航。
                this.child(
                    div()
                        .id(SharedString::from(format!("tree-toggle-{}", node.key)))
                        .flex_shrink_0()
                        .cursor_pointer()
                        .on_click(cx.listener(move |this: &mut Self, _event, _window, cx| {
                            cx.stop_propagation();
                            this.toggle_tree_node(&toggle_key, cx);
                        }))
                        .child(
                            Icon::new(if expanded {
                                IconName::ChevronDown
                            } else {
                                IconName::ChevronRight
                            })
                            .size_3()
                            .text_color(theme.muted_foreground),
                        ),
                )
            })
            .child(div().min_w_0().truncate().child(title.to_string()))
            .on_click(cx.listener(
                move |this: &mut Self, _event: &gpui::ClickEvent, _window, cx| {
                    this.on_tree_node_click(&key, cx);
                },
            ))
            .on_key_down(move |event, _window, cx| {
                let Some(action) = TreeKeyAction::from_key(event.keystroke.key.as_str()) else {
                    return;
                };
                let key = focus_key.clone();
                entity
                    .update(cx, |this, cx| match action {
                        TreeKeyAction::Activate => this.on_tree_node_click(&key, cx),
                        TreeKeyAction::Expand => this.expand_tree_node(&key, cx),
                        TreeKeyAction::Collapse => this.collapse_tree_node(&key, cx),
                    })
                    .ok();
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use extension_runtime::extension::manifest::{
        ResourceWorkbenchBinding, ResourceWorkbenchBindingSource, ResourceWorkbenchOpen,
        ResourceWorkbenchStaticTreeItem, ResourceWorkbenchTreeChildrenKind,
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

    /// 测试里没有连接上下文;`RouteSources` 借的是引用,所以要一个真正的
    /// `'static` 值而不是临时 `Value::Null`。
    static NO_CONNECTION: serde_json::Value = serde_json::Value::Null;

    fn sources<'a>(
        current_route: &'a serde_json::Value,
        selection: &'a serde_json::Value,
        parent: &'a serde_json::Value,
    ) -> RouteSources<'a> {
        RouteSources {
            route: current_route,
            selection,
            parent,
            connection: &NO_CONNECTION,
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

    fn static_item(id: &str, title: &str, page_id: &str) -> ResourceWorkbenchStaticTreeItem {
        ResourceWorkbenchStaticTreeItem {
            id: id.into(),
            title: title.into(),
            open: ResourceWorkbenchOpen {
                page_id: page_id.into(),
                route: BTreeMap::from([(
                    "name".to_string(),
                    binding(ResourceWorkbenchBindingSource::Parent, "/name"),
                )]),
            },
            children: None,
        }
    }

    fn static_children(
        items: Vec<ResourceWorkbenchStaticTreeItem>,
    ) -> ResourceWorkbenchTreeChildren {
        ResourceWorkbenchTreeChildren {
            kind: ResourceWorkbenchTreeChildrenKind::Static,
            operation: None,
            items_path: None,
            key_paths: Vec::new(),
            label_path: None,
            open: None,
            children: None,
            items,
        }
    }

    fn remote_children() -> ResourceWorkbenchTreeChildren {
        ResourceWorkbenchTreeChildren {
            kind: ResourceWorkbenchTreeChildrenKind::Remote,
            operation: Some("listContainers".into()),
            items_path: Some("/containers".into()),
            key_paths: vec!["/name".into()],
            label_path: Some("/name".into()),
            open: Some(open_by_id()),
            children: None,
            items: Vec::new(),
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
            "container-detail",
            &sources(&current, &rows[0], &null),
        ));
        assert!(!tree_child_is_active(
            "containers",
            Some(&open),
            &rows[1],
            "container-detail",
            &sources(&current, &rows[1], &null),
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
            "containers",
            &sources(&json!({"id": "abc"}), &row, &null),
        ));
        assert!(!tree_child_is_active(
            "containers",
            Some(&open),
            &row,
            "container-detail",
            &sources(&json!({"id": "other"}), &row, &null),
        ));
    }

    #[test]
    fn child_without_open_matches_root_page_and_row_route() {
        let row = json!({"id": "abc"});
        let null = serde_json::Value::Null;
        // 没有 open 声明时 route 直接取行数据,与 sources 无关。
        assert!(tree_child_is_active(
            "containers",
            None,
            &row,
            "containers",
            &sources(&row, &row, &null),
        ));
        assert!(!tree_child_is_active(
            "containers",
            None,
            &row,
            "containers",
            &sources(&json!({"id": "zzz"}), &row, &null),
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
            "pod-detail",
            &sources(&current, &row, &parent),
        ));
        assert!(!tree_child_is_active(
            "namespaces",
            Some(&open),
            &row,
            "pod-detail",
            &sources(&current, &row, &json!({"name": "kube-system"})),
        ));
    }

    /// 树节点导航的 `source: connection` 绑定必须取到注入的连接上下文。
    ///
    /// 回归:路由侧的来源曾经把 connection 无条件当空处理,扩展声明了
    /// "当前 namespace" 这类绑定也永远拿不到值。
    #[test]
    fn tree_child_route_keeps_connection_binding() {
        let open = ResourceWorkbenchOpen {
            page_id: "pod-detail".into(),
            route: BTreeMap::from([(
                "namespace".to_string(),
                binding(ResourceWorkbenchBindingSource::Connection, "/namespace"),
            )]),
        };
        let row = json!({"name": "api-0"});
        let connection = json!({"namespace": "prod", "database": "orders"});
        let current = json!({"namespace": "prod"});
        let sources = RouteSources {
            route: &current,
            selection: &row,
            parent: &NO_CONNECTION,
            connection: &connection,
        };

        assert!(tree_child_is_active(
            "namespaces",
            Some(&open),
            &row,
            "pod-detail",
            &sources,
        ));
    }

    /// 静态集合展开即得:行由 item 直接构造,键就是 item.id,不经过任何 provider。
    #[test]
    fn static_items_become_rows_without_a_provider_row() {
        let children = static_children(vec![
            static_item("mapping", "Mapping", "index-mapping"),
            static_item("settings", "Settings", "index-settings"),
        ]);
        let rows = static_child_rows(&children);
        assert_eq!(2, rows.len());
        assert_eq!("mapping", rows[0].key);
        assert_eq!("Mapping", rows[0].label);
        // 行数据只是稳定元数据,不含领域字段。
        assert_eq!(json!({"id": "mapping"}), rows[0].value);
        assert!(rows[0].children.is_none());
    }

    /// 同一层的静态项各自持有 open,可以指向不同页面。
    ///
    /// 回归:远程形态的 `open` 是集合级的,若静态项照抄集合级语义,这一层的
    /// 所有功能子项会全部跳到同一页。
    #[test]
    fn static_siblings_keep_their_own_open() {
        let children = static_children(vec![
            static_item("mapping", "Mapping", "index-mapping"),
            static_item("settings", "Settings", "index-settings"),
        ]);
        let rows = static_child_rows(&children);
        assert_eq!(
            "index-mapping",
            rows[0].open.as_ref().unwrap().page_id.as_str()
        );
        assert_eq!(
            "index-settings",
            rows[1].open.as_ref().unwrap().page_id.as_str()
        );
    }

    /// 静态项的 `parent` 绑定取的是**直接父节点**的领域行(索引行),不是它自己。
    #[test]
    fn static_item_parent_binding_resolves_to_the_index_row() {
        let children = static_children(vec![static_item("mapping", "Mapping", "index-mapping")]);
        let rows = static_child_rows(&children);
        let index_row = json!({"name": "logs"});
        assert!(tree_child_is_active(
            "indices",
            rows[0].open.as_ref(),
            &rows[0].value,
            "index-mapping",
            &sources(&json!({"name": "logs"}), &rows[0].value, &index_row),
        ));
        // 换一个索引就不该命中:A/B 索引切换时高亮不能串。
        assert!(!tree_child_is_active(
            "indices",
            rows[0].open.as_ref(),
            &rows[0].value,
            "index-mapping",
            &sources(&json!({"name": "metrics"}), &rows[0].value, &index_row),
        ));
    }

    /// 远程形态回归:行的 open/children 仍来自集合级声明。
    #[test]
    fn remote_rows_still_carry_the_collection_open() {
        let children = remote_children();
        let rows = remote_child_rows(
            &json!({"containers": [{"name": "web"}, {"name": "db"}]}),
            &children,
        );
        assert_eq!(2, rows.len());
        assert_eq!("web", rows[0].key);
        assert_eq!("web", rows[0].label);
        assert_eq!(
            "container-detail",
            rows[0].open.as_ref().unwrap().page_id.as_str()
        );
    }

    /// 静态分支必须在 provider 分支之前返回。
    ///
    /// 这是结构断言而不是行为断言:静态项没有 operation 可发,一旦顺序颠倒,
    /// 编译和注册都不会报错,只会表现成"点开是空的"。
    #[test]
    fn static_children_are_materialized_before_the_dispatcher() {
        let source = include_str!("nav_tree.rs");
        let body = source
            .split("fn load_tree_children")
            .nth(1)
            .expect("load_tree_children");
        let static_at = body.find("static_child_rows").expect("static branch");
        let dispatch_at = body.find("dispatch_invoke_scoped").expect("remote branch");
        assert!(
            static_at < dispatch_at,
            "static children must return before the provider dispatch"
        );
    }

    #[test]
    fn tree_keys_separate_parent_and_row() {
        assert_eq!("indices\u{1}logs", child_node_key("indices", "logs"));
    }

    #[test]
    fn key_actions_cover_mouse_semantics_only() {
        assert_eq!(
            Some(TreeKeyAction::Activate),
            TreeKeyAction::from_key("enter")
        );
        assert_eq!(
            Some(TreeKeyAction::Activate),
            TreeKeyAction::from_key("space")
        );
        assert_eq!(
            Some(TreeKeyAction::Expand),
            TreeKeyAction::from_key("right")
        );
        assert_eq!(
            Some(TreeKeyAction::Collapse),
            TreeKeyAction::from_key("left")
        );
        assert_eq!(None, TreeKeyAction::from_key("tab"));
    }
}
