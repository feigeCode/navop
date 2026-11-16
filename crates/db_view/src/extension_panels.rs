//! 数据库侧边栏的扩展面板接入点（`anchor: "db-sidebar"`）。
//!
//! db_view 不认识 nsh：宿主（navop_app）实现 [`DatabaseExtensionPanelHost`]
//! 并注册进来。侧栏只按规格（id/title/icon/order）决定工具栏上多出哪些按钮，
//! 真正打开某个面板时才向宿主要素材（`open_panel` 返回一个 `AnyView`）。
//! 依赖方向与 `terminal_view::sidebar::extension_panels` 完全一致。

use std::path::{Path, PathBuf};
use std::sync::Arc;

use gpui::{AnyView, App, Global, SharedString};
use gpui_component::Icon;
use one_assets::IconName;

/// 宿主侧给数据库侧栏列出的一个扩展面板。
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DatabaseExtensionPanelSpec {
    /// 全局唯一键（`"{extension_id}:{panel_id}"`），也是宿主打开面板的入参。
    pub id: String,
    /// 工具栏 tooltip 与面板标题。
    pub title: SharedString,
    /// 图标路径（扩展根下的文件）；缺省用扩展占位图标。
    pub icon_path: Option<PathBuf>,
    /// 排序权重，小的在前。
    pub order: i64,
}

/// 面板 `when` 子句的求值上下文（由 db_view 按当前连接填）。
#[derive(Clone, Debug, Default)]
pub struct DatabasePanelContext {
    /// 当前活跃连接的 id（`StoredConnection.id`）；没有连接时为 None。
    pub connection_id: Option<i64>,
    /// `ConnectionType` 的展示名小写，如 `"database"`。
    pub connection_type: Option<String>,
    /// 连接 params 解析后的 JSON（`when` 可写 `connection.params.db_type`）。
    pub params: Option<serde_json::Value>,
}

/// 宿主实现：列出某上下文下可见的扩展面板 + 打开其中一个。
///
/// 两个方法都会被侧栏在主线程调用；`panels` 不带 `&mut App`（只读 catalog），
/// `open_panel` 需要建 gpui 视图所以拿 `&mut App`。
pub trait DatabaseExtensionPanelHost: Send + Sync {
    fn panels(
        &self,
        cx: &App,
        context: &DatabasePanelContext,
    ) -> Vec<DatabaseExtensionPanelSpec>;

    /// 创建面板视图。`context` 与 [`DatabaseExtensionPanelHost::panels`] 收到
    /// 的是同一份启动上下文，宿主可据此决定面板绑定的连接，并把它作为
    /// `host.params.context` 交给页面。
    fn open_panel(
        &self,
        id: &str,
        context: &DatabasePanelContext,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
        cx: &mut App,
    ) -> Option<AnyView>;
}

struct GlobalDatabaseExtensionPanels(Arc<dyn DatabaseExtensionPanelHost>);

impl Global for GlobalDatabaseExtensionPanels {}

/// 宿主 init 时注册实现（未注册时侧栏就没有扩展面板，布局与以前一致）。
pub fn init_database_extension_panels(
    host: Arc<dyn DatabaseExtensionPanelHost>,
    cx: &mut App,
) {
    cx.set_global(GlobalDatabaseExtensionPanels(host));
}

/// 当前上下文下可见的扩展面板规格；宿主未注册时返回空。
pub fn database_extension_panel_specs(
    cx: &App,
    context: &DatabasePanelContext,
) -> Vec<DatabaseExtensionPanelSpec> {
    cx.try_global::<GlobalDatabaseExtensionPanels>()
        .map(|global| global.0.panels(cx, context))
        .unwrap_or_default()
}

/// 打开一个扩展面板；宿主未注册或面板不存在时返回 `None`。
pub fn open_database_extension_panel(
    id: &str,
    context: &DatabasePanelContext,
    tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
    cx: &mut App,
) -> Option<AnyView> {
    // 先把 host 克隆出来：`open_panel` 要 `&mut App`，不能同时借用 global。
    let host = cx.try_global::<GlobalDatabaseExtensionPanels>()?.0.clone();
    host.open_panel(id, context, tabs, cx)
}

/// 扩展图标：资产文件存在才生效，否则退占位图标。
fn icon_from_path(path: Option<&Path>) -> Icon {
    match path {
        Some(path) if path.is_file() => {
            let asset = one_core::storage::connection_icon::local_icon_asset_path(path);
            Icon::default().path(asset).color()
        }
        _ => Icon::new(IconName::ExtensionsLine),
    }
}

/// 一个扩展面板的运行时条目（工具栏按钮 + 面板视图缓存）。
#[derive(Clone)]
pub(crate) struct ExtensionPanelEntry {
    /// 全局唯一键（`"{extension_id}:{panel_id}"`）。
    pub(crate) id: SharedString,
    /// 面板标题（工具栏 tooltip 与面板标题栏）。
    pub(crate) title: SharedString,
    /// 面板图标。
    pub(crate) icon: Icon,
    /// 排序权重。
    pub(crate) order: i64,
    /// 面板视图（首次打开时创建并缓存）。
    pub(crate) view: Option<AnyView>,
    /// 视图是按哪个连接建的（`DatabasePanelContext.connection_id`）。
    /// 换了连接就要重建，否则页面还挂着上一条连接。
    pub(crate) view_connection: Option<i64>,
}

impl ExtensionPanelEntry {
    fn from_spec(spec: DatabaseExtensionPanelSpec) -> Self {
        Self {
            id: SharedString::new(spec.id),
            title: spec.title,
            icon: icon_from_path(spec.icon_path.as_deref()),
            order: spec.order,
            view: None,
            view_connection: None,
        }
    }
}

/// 把全局 specs 同步进现有条目列表：保留仍存在面板的视图缓存，新增的按
/// spec 建条目，消失的丢弃。返回按 order、id 排序的新列表。
pub(crate) fn sync_extension_panels(
    current: Vec<ExtensionPanelEntry>,
    specs: Vec<DatabaseExtensionPanelSpec>,
) -> Vec<ExtensionPanelEntry> {
    let mut specs = specs;
    specs.sort_by(|a, b| a.order.cmp(&b.order).then_with(|| a.id.cmp(&b.id)));
    specs
        .into_iter()
        .map(|spec| {
            if let Some(existing) = current.iter().find(|entry| entry.id.as_ref() == spec.id) {
                let mut kept = existing.clone();
                kept.title = spec.title;
                kept.icon = icon_from_path(spec.icon_path.as_deref());
                kept.order = spec.order;
                kept
            } else {
                ExtensionPanelEntry::from_spec(spec)
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::AppContext;

    fn spec(id: &str, order: i64, title: &str) -> DatabaseExtensionPanelSpec {
        DatabaseExtensionPanelSpec {
            id: id.to_string(),
            title: SharedString::new(title),
            icon_path: None,
            order,
        }
    }

    #[test]
    fn specs_are_sorted_by_order_then_id() {
        let next = sync_extension_panels(
            Vec::new(),
            vec![spec("b", 20, "B"), spec("a", 20, "A"), spec("c", 10, "C")],
        );
        assert_eq!(
            next.iter().map(|entry| entry.id.to_string()).collect::<Vec<_>>(),
            vec!["c".to_string(), "a".to_string(), "b".to_string()]
        );
    }

    /// 视图缓存的最小替身：`AnyView` 只要求一个 `Render` 实体。
    struct DummyView;

    impl gpui::Render for DummyView {
        fn render(
            &mut self,
            _window: &mut gpui::Window,
            _cx: &mut gpui::Context<Self>,
        ) -> impl gpui::IntoElement {
            gpui::div()
        }
    }

    #[gpui::test]
    fn existing_entries_keep_their_view_and_pick_up_new_titles(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            let view = AnyView::from(cx.new(|_cx| DummyView));
            let mut existing = sync_extension_panels(Vec::new(), vec![spec("a", 1, "旧标题")]);
            assert_eq!(existing.len(), 1);
            existing[0].view = Some(view);

            let next = sync_extension_panels(existing, vec![spec("a", 1, "新标题")]);
            assert_eq!(next.len(), 1);
            assert_eq!(next[0].title.as_ref(), "新标题");
            assert!(next[0].view.is_some(), "重新同步不能丢掉已建视图");
        });
    }

    #[test]
    fn removed_specs_are_dropped() {
        let existing = sync_extension_panels(Vec::new(), vec![spec("a", 1, "A"), spec("b", 2, "B")]);
        let next = sync_extension_panels(existing, vec![spec("b", 2, "B")]);
        assert_eq!(next.len(), 1);
        assert_eq!(next[0].id.as_ref(), "b");
    }
}
