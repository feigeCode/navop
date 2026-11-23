use super::*;
use crate::sidebar::tool_dock::right_sidebar_width;
use one_core::dock::ToolDockLayout;

impl TerminalView {
    pub(super) fn send_tab(&mut self, _: &SendTab, _window: &mut Window, cx: &mut Context<Self>) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        if self.try_accept_explicit_history_prompt(cx) {
            return;
        }
        self.dismiss_history_prompt();
        self.write_to_pty(b"\x09".to_vec(), cx);
    }

    pub(super) fn send_shift_tab(
        &mut self,
        _: &SendShiftTab,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        self.dismiss_history_prompt();
        self.write_to_pty(b"\x1b[Z".to_vec(), cx);
    }

    pub(super) fn render_sidebar_resize_handle(
        &mut self,
        target: ResizingPanel,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let view = cx.entity().clone();
        let (id, axis, placement) = match target {
            ResizingPanel::LeftSidebar => (
                "terminal-left-sidebar-resize-handle",
                Axis::Horizontal,
                Some(HandlePlacement::Left),
            ),
            ResizingPanel::RightSidebar => (
                "terminal-right-sidebar-resize-handle",
                Axis::Horizontal,
                Some(HandlePlacement::Right),
            ),
            ResizingPanel::BottomSidebar => (
                "terminal-bottom-sidebar-resize-handle",
                Axis::Vertical,
                None,
            ),
        };

        let handle = resize_handle::<ResizePanel, ResizePanel>(id, axis);
        let handle = match placement {
            Some(placement) => handle.placement(placement),
            None => handle,
        };
        handle.on_drag(ResizePanel, move |info, _, _, cx| {
            cx.stop_propagation();
            view.update(cx, |view, cx| {
                view.resizing = Some(target);
                cx.notify();
            });
            cx.new(|_| info.deref().clone())
        })
    }

    pub(super) fn terminal_tool_layout(&self, cx: &App) -> TerminalToolDockLayout {
        TerminalToolDockLayout::from_open_panels(self.sidebar.read(cx).open_tool_panels())
    }

    /// 扩展面板的停靠布局（键是 `"{extension_id}:{panel_id}"`）。
    ///
    /// 内建 dock 每边只放一个面板，扩展面板遵循同样的语义：内建面板
    /// 优先占据所在边，扩展面板只在内建面板未占用的一边显示。
    pub(super) fn extension_tool_layout(&self, cx: &App) -> ToolDockLayout<SharedString> {
        let sidebar = self.sidebar.read(cx);
        let builtin = self.terminal_tool_layout(cx);
        let mut layout = ToolDockLayout::from_open_panels_cloned(sidebar.open_extension_panels());
        if builtin.left.is_some() {
            layout.clear_if_occupied(SidebarPlacement::Left);
        }
        if builtin.right.is_some() {
            layout.clear_if_occupied(SidebarPlacement::Right);
        }
        if builtin.bottom.is_some() {
            layout.clear_if_occupied(SidebarPlacement::Bottom);
        }
        layout
    }

    pub(super) fn render_internal_tool_panel(
        &self,
        panel: SidebarPanel,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.sidebar_tool_panels.get(&panel).cloned() else {
            return div().into_any_element();
        };
        let colors = self.sidebar.read(cx).colors();
        let panel_header = one_ui::theme_geometry().layout.panel_header;
        render_internal_tool_panel_frame(
            self.sidebar.clone(),
            panel,
            placement,
            view,
            colors,
            panel_header,
            cx,
        )
    }

    /// 渲染停靠在三边的扩展面板（标题 + 移动菜单 + 关闭）。
    pub(super) fn render_extension_tool_panel(
        &self,
        id: SharedString,
        placement: SidebarPlacement,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let Some(view) = self.sidebar.read(cx).extension_panel_view(&id) else {
            return div().into_any_element();
        };
        let colors = self.sidebar.read(cx).colors();
        render_extension_tool_panel_frame(self.sidebar.clone(), id, placement, view, colors, cx)
    }

    pub(super) fn resize_sidebar(
        &mut self,
        mouse_position: Point<Pixels>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(resizing) = self.resizing else {
            return;
        };

        match resizing {
            ResizingPanel::LeftSidebar => {
                let new_size = mouse_position.x - self.view_bounds.left();
                self.sidebar_panel_size = new_size.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            }
            ResizingPanel::RightSidebar => {
                let new_size = right_sidebar_width(self.view_bounds.right(), mouse_position.x);
                self.sidebar_panel_size = new_size.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            }
            ResizingPanel::BottomSidebar => {
                let new_size = self.view_bounds.bottom() - mouse_position.y;
                self.sidebar_panel_size = new_size.clamp(SIDEBAR_MIN_WIDTH, SIDEBAR_MAX_WIDTH);
            }
        }

        cx.notify();
    }

    pub(super) fn done_resizing(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.resizing = None;
        cx.notify();
    }
}
