//! 数据库侧边栏的扩展面板宿主实现（`anchor: "db-sidebar"`）。
//!
//! 与终端侧栏的 [`crate::terminal_extension_panels`] 对称：从 extension-runtime
//! catalog 取面板声明，按 when 过滤后交给 db_view 渲染；面板视图由 nsh 宿主
//! （`NshHost::open_panel`）创建——同一个 nsh app 的第二种 surface。

use std::sync::Arc;

use db_view::extension_panels::{
    DatabaseExtensionPanelHost, DatabaseExtensionPanelSpec, DatabasePanelContext,
};
use gpui::{AnyView, App, SharedString};
use one_core::when_clause::{WhenContext, evaluate};

/// 把 db_view 给的连接上下文翻成 when 求值输入。
///
/// 键与内建面板一致用点号路径：`connection.type`（连接形态，小写）与
/// `connection.params`（连接参数原文），扩展可以在 when 里按库类型取面板，
/// 例如 `when: "connection.params.db_type == 'mysql'"`。
fn when_context(context: &DatabasePanelContext) -> WhenContext {
    let mut map = serde_json::Map::new();
    if let Some(connection_type) = &context.connection_type {
        map.insert(
            "connection.type".into(),
            serde_json::Value::String(connection_type.clone()),
        );
    }
    if let Some(params) = &context.params {
        map.insert("connection.params".into(), params.clone());
    }
    WhenContext::from_json(serde_json::Value::Object(map))
}

/// 面板启动上下文：页面从 `host.params.context` 读到它，用来定位自己挂在
/// 哪条连接上（与 DB 菜单那条路径同形，只是这里代表“当前连接”）。
fn launch_context(context: &DatabasePanelContext) -> serde_json::Value {
    let mut map = serde_json::Map::new();
    if let Some(connection_id) = context.connection_id {
        map.insert("connectionId".into(), connection_id.into());
    }
    if let Some(connection_type) = &context.connection_type {
        map.insert("connectionType".into(), connection_type.clone().into());
    }
    if let Some(params) = &context.params {
        map.insert("params".into(), params.clone());
    }
    serde_json::Value::Object(map)
}

/// `navop.db` 的数据库侧栏扩展面板宿主。
pub(crate) struct NavopDatabaseExtensionPanels;

impl DatabaseExtensionPanelHost for NavopDatabaseExtensionPanels {
    fn panels(&self, cx: &App, context: &DatabasePanelContext) -> Vec<DatabaseExtensionPanelSpec> {
        let Some(catalog) = extension_runtime::global_catalog(cx) else {
            return Vec::new();
        };
        let context = when_context(context);
        catalog
            .panels_for_anchor("db-sidebar")
            .into_iter()
            .filter(|panel| match &panel.when_clause {
                Some(clause) => evaluate(clause, &context).unwrap_or(false),
                None => true,
            })
            .map(|panel| DatabaseExtensionPanelSpec {
                id: format!("{}:{}", panel.extension_id, panel.id),
                title: SharedString::new(panel.title.clone()),
                icon_path: panel.icon_path.clone(),
                order: panel.order,
            })
            .collect()
    }

    fn open_panel(
        &self,
        id: &str,
        context: &DatabasePanelContext,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
        cx: &mut App,
    ) -> Option<AnyView> {
        // id 形如 "{extension_id}:{panel_id}"；extension_id 自带点号，
        // 用 rsplit 取最后一个冒号前的整段。
        let (extension_id, panel_id) = id.rsplit_once(':')?;
        let host = cx.try_global::<universal_plugins::NshHost>()?.clone();
        let panel = host.open_panel_with_context(
            extension_id,
            panel_id,
            Some(launch_context(context)),
            tabs,
            cx,
        )?;
        Some(panel.into())
    }
}

/// 注册数据库扩展面板宿主（app init 时调用一次）。
pub(crate) fn init_database_extension_panels(cx: &mut App) {
    db_view::extension_panels::init_database_extension_panels(
        Arc::new(NavopDatabaseExtensionPanels),
        cx,
    );
}
