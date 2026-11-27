//! nsh 扩展面板：宿主 UI 锚点（终端侧边栏 / 数据库侧栏）里嵌的第二个 surface。
//!
//! 一个面板 = 同一 app 的一个独立 `AppView` 实例：入口复用
//! `shellApp.entry`，宿主以 `host.params.surface = "panel"` + `panel = <id>`
//! 启动它，页面按 surface 分支渲染。与 tab 的差别只在挂载位置与生命周期：
//! - tab 由 tab 容器持有；面板由 [`NshPanel`] 自己持有并画在宿主锚点里；
//! - 面板不建插件 AI 侧栏（那是 tab 场景的伴随面板）；
//! - 卸载/停用扩展时，宿主（终端侧栏）要主动 drop 掉面板实体。

use gpui::{
    App, AppContext as _, Context, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    ParentElement, Render, Styled, Task, Window, div,
};

use super::{Trust, tab};

/// 一个扩展面板实例（`contributes.panels` 的一条已注册项）。
///
/// 装载流程与 [`tab::NshTab`] 同构：先激活后端（租 provider lease），
/// 再建 `AppView`；差别是这里不接 tab 容器，而是被宿主锚点（终端侧栏）
/// 作为子视图嵌入。
pub struct NshPanel {
    inner: Entity<tab::NshTab>,
    panel_id: String,
    extension_id: String,
    title: String,
}

impl NshPanel {
    /// 打开一个扩展面板。
    ///
    /// `contribution` 是所属 app 的贡献项（入口/后端/能力都取自它）；
    /// `panel` 提供面板 id 与标题。`when` 过滤与排序由宿主锚点在列出
    /// 面板时处理，这里只负责装载。
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn open(
        contribution: extension_runtime::RegisteredShellAppContribution,
        panel: extension_runtime::RegisteredPanelContribution,
        launch_context: Option<serde_json::Value>,
        tabs: Option<gpui::WeakEntity<one_core::tab_container::TabContainer>>,
        surface_theme: Option<nsh_platform_gpui::Theme>,
        service: crate::UniversalPluginService,
        trust: Trust,
        dispatcher: super::dispatch::MainThread,
        cx: &mut App,
    ) -> Entity<Self> {
        let inner = tab::NshTab::open_panel(
            contribution,
            &panel.id,
            &panel.anchor,
            launch_context,
            tabs,
            surface_theme,
            service,
            trust,
            dispatcher,
            cx,
        );
        let extension_id = inner.read(cx).extension_id().to_string();
        cx.new(|_| Self {
            extension_id,
            panel_id: panel.id,
            title: panel.title,
            inner,
        })
    }

    /// 面板所属扩展。
    pub fn extension_id(&self) -> &str {
        &self.extension_id
    }

    /// 面板 id（清单 `contributes.panels[].id`）。
    pub fn panel_id(&self) -> &str {
        &self.panel_id
    }

    /// 面板标题。
    pub fn title(&self) -> &str {
        &self.title
    }

    /// 扩展停用/卸载：停掉内部页面并请求宿主移除本面板。
    ///
    /// 返回的任务在 provider 租约归还后完成；宿主锚点应当随后 drop 本
    /// 实体（面板实体没有 tab 容器帮忙收尾）。
    pub(crate) fn close_for_extension(&mut self, cx: &mut Context<Self>) -> Task<bool> {
        self.inner
            .update(cx, |inner, cx| inner.close_for_extension(cx))
    }

    /// provider 事件转发（与 tab 同语义）：换进程重装，被移除停在错误页。
    pub(crate) fn runtime_changed(
        &mut self,
        event: &extension_plugin_adapter::RuntimeMonitorEvent,
        cx: &mut Context<Self>,
    ) {
        self.inner
            .update(cx, |inner, cx| inner.runtime_changed(event, cx));
    }
}

impl EventEmitter<one_core::tab_container::TabContentEvent> for NshPanel {}

impl Focusable for NshPanel {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.inner.read(cx).focus_handle(cx)
    }
}

impl Render for NshPanel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // 面板出错时内部 NshTab 自己会把错误画出来，这里只管铺满。
        let _ = cx;
        div()
            .size_full()
            .min_w_0()
            .min_h_0()
            .child(self.inner.clone())
    }
}
