use gpui::{
    App, AppContext as _, Context, DragMoveEvent, EntityId, EventEmitter, FocusHandle, Focusable,
    InteractiveElement, IntoElement, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Window, div, prelude::FluentBuilder, px,
};

use gpui_component::{Icon, Sizable as _, Size, StyledExt as _, h_flex, v_flex};
use one_assets::IconName;
use one_core::tab_container::{TabContent, TabContentEvent};
use one_ui::{IconButton, IconButtonRole};
use rust_i18n::t;

use super::WorkbenchShell;
use super::super::state::{WorkbenchPanelKind, WorkbenchPlacement, WorkbenchTab};
use super::widgets::{close_tab_tooltip, cycle_placement_tooltip, pin_tooltip};
use super::{DOCK_PANEL_HEIGHT, DOCK_PANEL_WIDTH, PANEL_HEADER_HEIGHT};
use crate::theme::{AgentChatTheme, with_agent_chat_theme};

/// 拖拽调宽把手的命中宽度。
const RESIZE_HANDLE_WIDTH: f32 = 6.0;

/// 左侧导航栏的拖拽标记（GPUI 拖拽载荷）。
#[derive(Clone)]
struct ResizeNav {
    entity_id: EntityId,
}

impl Render for ResizeNav {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

/// 右侧标签组的拖拽标记。
#[derive(Clone)]
struct ResizeRight {
    entity_id: EntityId,
}

impl Render for ResizeRight {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div().size(px(0.0))
    }
}

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
        let maximized = self.state.right_maximized();
        let show_right_group =
            !self.state.right_collapsed() && !self.state.right_tabs().is_empty();
        let nav_width = self.state.nav_width();
        let right_width = self.state.right_width();
        let hover = theme.hover_background();

        let body = with_agent_chat_theme(&theme, || {
            h_flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                // 放大占满时只有右侧标签组（见下），其余区域全部让位。
                .when_some((!maximized).then_some(nav).flatten(), |this, nav| {
                    this.child(
                        v_flex()
                            .w(px(nav_width))
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
                    .child(
                        div()
                            .id("workbench-nav-resize")
                            .w(px(RESIZE_HANDLE_WIDTH))
                            .h_full()
                            .flex_shrink_0()
                            .cursor_col_resize()
                            .hover(move |style| style.bg(hover))
                            .on_drag_move(cx.listener(
                                move |this, event: &DragMoveEvent<ResizeNav>, _, cx| {
                                    if event.drag(cx).entity_id != cx.entity_id() {
                                        return;
                                    }
                                    let delta = f32::from(
                                        event.event.position.x - event.bounds.center().x,
                                    );
                                    this.set_nav_width(this.state.nav_width() + delta, cx);
                                },
                            ))
                            .on_drag(
                                ResizeNav {
                                    entity_id: cx.entity_id(),
                                },
                                |drag, _, _, cx| {
                                    cx.stop_propagation();
                                    cx.new(|_| drag.clone())
                                },
                            ),
                    )
                })
                .when_some(
                    (!maximized).then_some(left_panel).flatten(),
                    |this, kind| {
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
                    },
                )
                .when(!maximized, |this| this.child(self.render_content(&theme)))
                .when(show_right_group, |this| {
                    this.when(!maximized, |this| {
                        this.child(
                            div()
                                .id("workbench-right-resize")
                                .w(px(RESIZE_HANDLE_WIDTH))
                                .h_full()
                                .flex_shrink_0()
                                .cursor_col_resize()
                                .hover(move |style| style.bg(hover))
                                .on_drag_move(cx.listener(
                                    move |this, event: &DragMoveEvent<ResizeRight>, _, cx| {
                                        if event.drag(cx).entity_id != cx.entity_id() {
                                            return;
                                        }
                                        // 把手在标签组左缘：光标左移 = 加宽。
                                        let delta = f32::from(
                                            event.event.position.x - event.bounds.center().x,
                                        );
                                        this.set_right_width(
                                            this.state.right_width() - delta,
                                            cx,
                                        );
                                    },
                                ))
                                .on_drag(
                                    ResizeRight {
                                        entity_id: cx.entity_id(),
                                    },
                                    |drag, _, _, cx| {
                                        cx.stop_propagation();
                                        cx.new(|_| drag.clone())
                                    },
                                ),
                        )
                    })
                    .child(
                        if maximized {
                            v_flex()
                                .flex_1()
                                .h_full()
                                .min_h_0()
                                .min_w_0()
                                .border_r_1()
                                .border_color(theme.border)
                                .child(self.render_right_group(&theme, cx))
                                .into_any_element()
                        } else {
                            div()
                                .w(px(right_width))
                                .flex_shrink_0()
                                .h_full()
                                .min_h_0()
                                .border_l_1()
                                .border_color(theme.border)
                                .child(self.render_right_group(&theme, cx))
                                .into_any_element()
                        },
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

        with_agent_chat_theme(&theme, || {
            v_flex()
                .size_full()
                .min_w_0()
                .min_h_0()
                .overflow_hidden()
                .bg(theme.background)
                .text_color(theme.foreground)
                .child(body)
                .when_some(bottom, |this, bottom| this.child(bottom))
        })
    }
}

impl WorkbenchShell {
    /// 左侧 / 底部的单槽面板：标题栏（图标 + 名称 + 移到下一处 + 关闭）+ 内容。
    fn render_dock(
        &self,
        slot: WorkbenchTab,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let Some(view) = self.panels.get(&slot).cloned() else {
            return div().into_any_element();
        };
        let header = self.render_panel_header(slot.kind, workbench_panel_icon(slot.kind), theme, cx);

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
            .filter(|kind| !tabs.iter().any(|tab| tab.kind == *kind) && self.has_panel(*kind))
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
        for (index, tab) in tabs.iter().enumerate() {
            // 该实例在同 kind 标签中的下标，决定显示名要不要带序号。
            let ordinal = tabs[..index]
                .iter()
                .filter(|prev| prev.kind == tab.kind)
                .count();
            bar = bar.child(self.render_tab(
                *tab,
                ordinal,
                Some(*tab) == active,
                theme,
                cx,
            ));
        }
        // 「新建页签」：先弹一个空白选择面板（虚拟页签），点选后当前页签
        // 即变为对应功能。
        if self.picker_open {
            bar = bar.child(self.render_picker_tab(theme, cx));
        }
        bar = bar.child(
            IconButton::new("workbench-tab-add", IconName::Plus)
                .role(IconButtonRole::Compact)
                .tooltip(t!("Workbench.add_tab").to_string())
                .text_color(theme.muted_foreground)
                .on_click(cx.listener(|this, _, _, cx| this.toggle_tab_picker(cx))),
        );
        // 还没并排打开的面板：点一下就进标签组，避免再叠一层下拉菜单。
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
        // 放大占满 / 还原。
        let maximized = self.state.right_maximized();
        bar = bar.child(
            IconButton::new("workbench-right-maximize", {
                if maximized {
                    IconName::WindowRestore
                } else {
                    IconName::Maximize
                }
            })
            .role(IconButtonRole::Compact)
            .tooltip(if maximized {
                t!("Workbench.restore_right_sidebar").to_string()
            } else {
                t!("Workbench.maximize_right_sidebar").to_string()
            })
            .text_color(theme.muted_foreground)
            .on_click(cx.listener(|this, _, _, cx| this.toggle_right_maximized(cx))),
        );

        let content = if self.picker_open {
            self.render_picker_panel(theme, cx)
        } else {
            active
                .and_then(|tab| self.panels.get(&tab).cloned())
                .map(|view| {
                    div()
                        .flex_1()
                        .min_h_0()
                        .min_w_0()
                        .overflow_hidden()
                        .child(view)
                        .into_any_element()
                })
                .unwrap_or_else(|| div().flex_1().into_any_element())
        };

        v_flex()
            .size_full()
            .min_w_0()
            .min_h_0()
            .bg(theme.background)
            .child(bar)
            .child(content)
            .into_any_element()
    }

    /// 单个标签：点击激活，右侧 X 关闭。`ordinal` 是该实例在同 kind
    /// 标签里的下标，决定显示名要不要带序号。
    fn render_tab(
        &self,
        tab: WorkbenchTab,
        ordinal: usize,
        active: bool,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let kind = tab.kind;
        let on_select = cx
            .listener(move |this, _, _, cx| this.select_right_tab_instance(tab, cx));
        let on_close = cx.listener(move |this, _, _, cx| this.close_right_tab_instance(tab, cx));

        h_flex()
            .id(SharedString::from(format!(
                "workbench-tab-{}#{}",
                kind.id(),
                tab.seq()
            )))
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
                    .child(tab.display_title(ordinal)),
            )
            .child(
                IconButton::new(
                    SharedString::from(format!(
                        "workbench-tab-close-{}#{}",
                        kind.id(),
                        tab.seq()
                    )),
                    IconName::Close,
                )
                .role(IconButtonRole::Compact)
                .tooltip(close_tab_tooltip(kind))
                .on_click(on_close),
            )
            .on_click(on_select)
            .into_any_element()
    }

    /// 「新建页签」的虚拟标签：只在选择面板打开时出现在标签条上，
    /// 点 X 或点选面板后消失。
    fn render_picker_tab(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        h_flex()
            .id("workbench-tab-picker")
            .min_w_0()
            .items_center()
            .gap_1()
            .px_1p5()
            .py_1()
            .rounded(theme.surface_radius)
            .bg(theme.panel_hover)
            .cursor_pointer()
            .child(Icon::new(IconName::Plus).with_size(Size::XSmall))
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(theme.foreground)
                    .child(t!("Workbench.add_tab").to_string()),
            )
            .child(
                IconButton::new("workbench-tab-picker-close", IconName::Close)
                    .role(IconButtonRole::Compact)
                    .tooltip(t!("Workbench.close_tab", panel = t!("Workbench.add_tab").to_string()).to_string())
                    .on_click(cx.listener(|this, _, _, cx| this.close_tab_picker(cx))),
            )
            .into_any_element()
    }

    /// 「新建页签」选择面板：列出所有可用面板（终端、文件、审查），
    /// 点击后当前页签即变为该功能。
    fn render_picker_panel(
        &self,
        theme: &AgentChatTheme,
        cx: &mut Context<Self>,
    ) -> gpui::AnyElement {
        let hover = theme.hover_background();
        let mut list = v_flex().w(px(320.0)).gap_0p5();
        for kind in WorkbenchPanelKind::DOCKABLE
            .into_iter()
            .filter(|kind| self.has_panel(*kind))
        {
            list = list.child(
                h_flex()
                    .id(SharedString::from(format!("workbench-picker-{}", kind.id())))
                    .w_full()
                    .items_center()
                    .gap_2()
                    .px_2()
                    .py_1p5()
                    .rounded(theme.surface_radius)
                    .cursor_pointer()
                    .hover(move |style| style.bg(hover))
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.pick_panel(kind, window, cx)
                    }))
                    .child(
                        Icon::new(kind.icon())
                            .small()
                            .text_color(theme.muted_foreground),
                    )
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .truncate()
                            .text_sm()
                            .text_color(theme.foreground)
                            .child(kind.title()),
                    )
                    // 多例面板点选是「每次新建」而非定位，给一行提示。
                    .when(kind.multi_instance(), |this| {
                        this.child(
                            div()
                                .flex_shrink_0()
                                .text_xs()
                                .text_color(theme.muted_foreground)
                                .child(t!("Workbench.picker_multi_hint").to_string()),
                        )
                    }),
            );
        }

        div()
            .debug_selector(|| "workbench-picker".to_string())
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .flex()
            .items_center()
            .justify_center()
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .text_color(theme.foreground)
                            .child(t!("Workbench.picker_title").to_string()),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(theme.muted_foreground)
                            .child(t!("Workbench.picker_hint").to_string()),
                    )
                    .child(list),
            )
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
