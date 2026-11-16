//! 终端侧边栏的扩展面板宿主实现。
//!
//! 把 extension-runtime catalog 里的 `panels`（anchor = `terminal-sidebar`）
//! 经 when 过滤后交给 terminal_view 渲染；面板视图由 nsh 宿主
//! （`NshHost::open_panel`）创建——面板是同一 nsh app 的第二 surface。

use std::sync::Arc;

use gpui::{AnyView, App, SharedString};
use one_core::when_clause::{WhenContext, evaluate};
use terminal_view::TerminalExtensionPanelHost;

/// 终端扩展面板的 when 上下文键（求值输入）。
///
/// 目前侧栏构造时可用的只有连接形态；后续接入 cwd、会话状态等再扩。
fn when_context(connection_kind: Option<&str>) -> WhenContext {
    let mut context = serde_json::Map::new();
    if let Some(kind) = connection_kind {
        context.insert(
            "connection.kind".into(),
            serde_json::Value::String(kind.to_string()),
        );
    }
    WhenContext::from_json(serde_json::Value::Object(context))
}

/// `navop.app` 的终端扩展面板宿主。
pub(crate) struct NavopTerminalExtensionPanels;

impl TerminalExtensionPanelHost for NavopTerminalExtensionPanels {
    fn panels(&self, cx: &App) -> Vec<terminal_view::SidebarExtensionPanelSpec> {
        let Some(catalog) = extension_runtime::global_catalog(cx) else {
            return Vec::new();
        };
        // 终端侧栏目前对所有连接形态提供同一组面板；when 子句留空即全过。
        // 连接上下文（kind 等）在 P2 navop.terminal 模块接入后补全。
        let context = when_context(None);
        catalog
            .panels_for_anchor("terminal-sidebar")
            .into_iter()
            .filter(|panel| match &panel.when_clause {
                Some(clause) => evaluate(clause, &context).unwrap_or(false),
                None => true,
            })
            .map(|panel| terminal_view::SidebarExtensionPanelSpec {
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
        colors: &terminal_view::TerminalColors,
        cx: &mut App,
    ) -> Option<AnyView> {
        // id 形如 "{extension_id}:{panel_id}"；extension_id 自带点号，
        // 用 rsplit 取最后一个冒号前的整段。
        let (extension_id, panel_id) = id.rsplit_once(':')?;
        let host = cx.try_global::<universal_plugins::NshHost>()?.clone();
        // 面板跟随终端配色，而不是应用主题：侧栏和面板看起来是一体的。
        let theme = universal_plugins::nsh_terminal_theme(colors, cx);
        let panel = host.open_panel_with_theme(
            extension_id,
            panel_id,
            None,
            None,
            Some(theme),
            cx,
        )?;
        Some(panel.into())
    }
}

/// 注册终端扩展面板宿主（app init 时调用一次）。
pub(crate) fn init_terminal_extension_panels(cx: &mut App) {
    terminal_view::init_sidebar_extension_panels(Arc::new(NavopTerminalExtensionPanels), cx);
}
