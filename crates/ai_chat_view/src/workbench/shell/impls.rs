use gpui::{
    App, Context, EventEmitter, FocusHandle, Focusable, InteractiveElement, IntoElement,
    ParentElement, Render, SharedString, StatefulInteractiveElement, Styled, Window, div,
    prelude::FluentBuilder, px,
};

use gpui_component::{Icon, Sizable as _, Size, h_flex, v_flex};
use one_assets::IconName;
use one_core::layout::SIDEBAR_DEFAULT_WIDTH;
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use super::WorkbenchShell;
use super::super::state::{WorkbenchPanelKind, WorkbenchPlacement};
use super::widgets::{close_tab_tooltip, cycle_placement_tooltip, pin_tooltip};
use super::{DOCK_PANEL_HEIGHT, DOCK_PANEL_WIDTH, PANEL_HEADER_HEIGHT};
use crate::theme::{AgentChatTheme, with_agent_chat_theme};

impl EventEmitter<TabContentEvent> for WorkbenchShell {}

impl TabContent for WorkbenchShell {
    fn content_key(&self) -> &'static str {
        "AiWorkbench"
    }

    fn title(&self, _cx: &App) -> SharedString {
        t!("AgentUi.workbench").into()
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::AI.color().with_size(Size::Medium))
    }

    fn closeable(&self, _cx: &App) -> bool {
        self.tab_closeable
    }

    fn can_rename(&self, _cx: &App) -> bool {
        false
    }

    fn dump(&self, _cx: &App) -> serde_json::Value {
        // 布局不走标签页持久化（`tab_persistence` 在本仓未启用），
        // 真实落盘在 `WorkbenchShell::persist_layout` → `AppSettings`。
        serde_json::json!({ "version": 1 })
    }
}

impl Focusable for WorkbenchShell {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for WorkbenchShell {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = self.snapshot(cx);
        let show_nav = !self.state.nav_collapsed() && self.has_nav();
        let nav = show_nav
            .then(|| self.render_session_nav(&theme, cx))
            .flatten();
        let left_panel = self.state.left();
        let bottom_panel = self.state.bottom();
        let has_right_group = !self.state.right_tabs().is_empty();

        let body = with_agent_chat_theme(&theme, || {
            h_flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .when_some(nav, |this, nav| {
                    this.child(
                        v_flex()
                            .w(SIDEBAR_DEFAULT_WIDTH)
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .min_w_0()
                                    .overflow_hidden()
                                    .child(nav),
                            ),
                    )
                })
                .when_some(left_panel, |this, kind| {
                    this.child(
                        div()
                            .w(px(DOCK_PANEL_WIDTH))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_r_1()
                            .border_color(theme.border)
                            .child(self.render_dock(kind, &theme, cx)),
                    )
                })
                .child(self.render_content(&theme))
                .when(has_right_group, |this| {
                    this.child(
                        div()
                            .w(px(DOCK_PANEL_WIDTH))
                            .flex_shrink_0()
                            .h_full()
                            .min_h_0()
                            .border_l_1()
                            .border_color(theme.border)
                            .child(self.render_right_group(&theme, cx)),
                    )
                })
                .child(self.render_rail(&theme, cx))
                .into_any_element()
        });

        let bottom = bottom_panel.map(|kind| {
            with_agent_chat_theme(&theme, || {
                div()
                    .w_full()
                    .h(px(DOCK_PANEL_HEIGHT))
                    .flex_shrink_0()
                    .border_t_1()
                    .border_color(theme.border)
                    .child(self.render_dock(kind, &theme, cx))
                    .into_any_element()
            })
        });

        let header =
            with_agent_chat_theme(&theme, || self.render_header(&theme, cx).into_any_element());

        with_agent_chat_theme(&theme, || {
            v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(header)
                .child(body)
                .when_some(bottom, |this, bottom| this.child(bottom))
        })
    }
}

impl WorkbenchShell {
    /// 左侧 / 底部的单槽面板：标题栏（图标 + 名称 + 移到下一处 + 关闭）+ 内容。
    fn render_dock(
        &self,
        kind: WorkbenchPanelKind,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(view) = self.panels.get(&kind).cloned() else {
            return div().into_any_element();
        };
        let header = self.render_panel_header(kind, workbench_panel_icon(kind), theme, cx);

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(header)
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(view),
            )
            .into_any_element()
    }

    /// 右侧标签组：标签条（含「并排打开」入口）+ 激活面板内容。
    fn render_right_group(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let tabs = self.state.right_tabs().to_vec();
        let active = self.state.right_active();
        let unpinned: Vec<WorkbenchPanelKind> = WorkbenchPanelKind::DOCKABLE
            .into_iter()
            .filter(|kind| !tabs.contains(kind) && self.has_panel(*kind))
            .collect();

        let mut bar = h_flex()
            .h(px(PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_1()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.panel);
        for kind in &tabs {
            bar = bar.child(self.render_tab(*kind, Some(*kind) == active, theme, cx));
        }
        // 还没并排打开的面板：点一下就进标签组，避免再叠一层下拉菜单。
        if !unpinned.is_empty() {
            bar = bar.child(div().flex_1());
            for kind in unpinned {
                let on_pin = cx.listener(move |this, _, _, cx| {
                    this.open_panel(kind, WorkbenchPlacement::Right, cx);
                });
                bar = bar.child(
                    IconButton::new(
                        SharedString::from(format!("workbench-pin-{}", kind.id())),
                        kind.icon(),
                    )
                    .role(IconButtonRole::Compact)
                    .tooltip(pin_tooltip(kind))
                    .text_color(theme.muted_foreground)
                    .on_click(on_pin),
                );
            }
        }

        let content = active
            .and_then(|kind| self.panels.get(&kind).cloned())
            .map(|view| {
                div()
                    .flex_1()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(view)
                    .into_any_element()
            })
            .unwrap_or_else(|| div().flex_1().into_any_element());

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(bar)
            .child(content)
            .into_any_element()
    }

    /// 单个标签：点击激活，右侧 X 关闭。
    fn render_tab(
        &self,
        kind: WorkbenchPanelKind,
        active: bool,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let on_select =
            cx.listener(move |this, _, _, cx| this.select_right_tab(kind, cx));
        let on_close = cx.listener(move |this, _, _, cx| this.close_panel(kind, cx));

        h_flex()
            .id(SharedString::from(format!("workbench-tab-{}", kind.id())))
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_1()
            .rounded(theme.surface_radius)
            .cursor_pointer()
            .when(active, |this| this.bg(theme.panel_hover))
            .hover(|style| style.bg(theme.panel_hover))
            .child(Icon::new(kind.icon()).with_size(Size::XSmall))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(if active {
                        theme.foreground
                    } else {
                        theme.muted_foreground
                    })
                    .child(kind.title()),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("workbench-tab-close-{}", kind.id())),
                    IconName::Close,
                )
                .role(IconButtonRole::Compact)
                .tooltip(close_tab_tooltip(kind))
                .on_click(on_close),
            )
            .on_click(on_select)
            .into_any_element()
    }

    /// 单槽面板的标题栏。
    fn render_panel_header(
        &self,
        kind: WorkbenchPanelKind,
        icon: IconName,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let next = self
            .state
            .placement_of(kind)
            .map(WorkbenchPlacement::next)
            .unwrap_or(WorkbenchPlacement::Right);
        let on_cycle = cx.listener(move |this, _, _, cx| this.cycle_panel_placement(kind, cx));
        let on_close = cx.listener(move |this, _, _, cx| this.close_panel(kind, cx));

        h_flex()
            .h(px(PANEL_HEADER_HEIGHT))
            .flex_shrink_0()
            .items_center()
            .gap_2()
            .px_2()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.panel)
            .child(Icon::new(icon).with_size(Size::Small))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_sm()
                    .text_color(theme.foreground)
                    .child(kind.title()),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("workbench-dock-move-{}", kind.id())),
                    next.icon(),
                )
                .role(IconButtonRole::Compact)
                .tooltip(cycle_placement_tooltip(next))
                .on_click(on_cycle),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!("workbench-dock-close-{}", kind.id())),
                    IconName::Close,
                )
                .role(IconButtonRole::Compact)
                .tooltip(t!("Workbench.close_panel").to_string())
                .on_click(on_close),
            )
            .into_any_element()
    }
}

/// 面板在标题栏里的图标。与 `WorkbenchPanelKind::icon` 分开是为了让渲染层
/// 可以给停靠头换一套更「面板化」的图标，而不影响工具条与标签条。
fn workbench_panel_icon(kind: WorkbenchPanelKind) -> IconName {
    kind.icon()
}
