use super::*;

struct ToolDockRenderState {
    sidebar_size: Pixels,
    right_width: Pixels,
    render_internal_dock: bool,
    left_panel: Option<AnyElement>,
    right_panel: Option<AnyElement>,
    bottom_panel: Option<AnyElement>,
}

struct CenterRegionState {
    font_family: SharedString,
    bottom_panel: Option<AnyElement>,
    sidebar_size: Pixels,
}

struct RightRegionState {
    panel: Option<AnyElement>,
    sidebar_size: Pixels,
    width: Pixels,
}

impl TerminalView {
    pub(super) fn prepare_render(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> SharedString {
        let font_metrics = self.terminal_font_metrics(window, cx);
        if self.cell_width != font_metrics.cell_width {
            self.cell_width = font_metrics.cell_width;
            self.last_size = None;
        }
        self.line_height = self.font_size * self.line_height_scale;
        self.apply_pending_scrollbar_offset(cx);
        self.sync_recording_ticker(cx);
        self.sync_recording_playback_ticker(cx);
        self.sync_recording_playback_slider(window, cx);

        let terminal_mode = self.terminal.read(cx).mode();
        self.handle_alt_screen_transition(terminal_mode, cx);
        font_metrics.effective_family
    }

    fn apply_pending_scrollbar_offset(&mut self, cx: &mut Context<Self>) {
        let Some(new_display_offset) = self.scrollbar_handle.take_future_display_offset() else {
            return;
        };
        self.terminal.update(cx, |terminal, _| {
            let current = terminal.term().lock().grid().display_offset() as i32;
            let delta = new_display_offset as i32 - current;
            if delta != 0 {
                terminal.scroll(delta);
            }
        });
    }

    fn handle_alt_screen_transition(&mut self, terminal_mode: TermMode, cx: &mut Context<Self>) {
        let alt_screen = terminal_mode.contains(TermMode::ALT_SCREEN);
        if alt_screen == self.last_alt_screen {
            return;
        }
        tracing::info!(
            target: "terminal_residue",
            from = self.last_alt_screen,
            to = alt_screen,
            last_size = ?self.last_size,
            "alt_screen mode transition"
        );
        self.last_alt_screen = alt_screen;
        if alt_screen && self.last_size.is_some() {
            tracing::info!(target: "terminal_residue", "nudge_resize fired on enter alt_screen");
            self.terminal
                .update(cx, |terminal, _| terminal.nudge_resize());
        }
    }

    pub(super) fn render_tool_dock(
        &mut self,
        font_family: SharedString,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let state = self.tool_dock_render_state(cx);
        let view = cx.entity().clone();
        let bg_color = self.current_theme.background;
        let center = CenterRegionState {
            font_family,
            bottom_panel: state.bottom_panel,
            sidebar_size: state.sidebar_size,
        };
        let right = RightRegionState {
            panel: state.right_panel,
            sidebar_size: state.sidebar_size,
            width: state.right_width,
        };

        h_flex()
            .debug_selector(|| "terminal-tool-dock-root".to_string())
            .size_full()
            .min_w_0()
            .min_h_0()
            .overflow_hidden()
            .bg(bg_color)
            .key_context(TERMINAL_CONTEXT)
            .on_action(cx.listener(Self::start_recording_action))
            .on_action(cx.listener(Self::pause_recording_action))
            .on_action(cx.listener(Self::resume_recording_action))
            .on_action(cx.listener(Self::stop_recording_action))
            .when_some(state.left_panel, |this, panel| {
                this.child(self.render_left_region(panel, state.sidebar_size, cx))
            })
            .child(self.render_center_region(center, cx))
            .when(state.render_internal_dock, |this| {
                this.child(self.render_right_region(right, cx))
            })
            .when(state.render_internal_dock, |this| {
                this.child(ResizeEventHandler { view })
            })
            .into_any_element()
    }

    fn tool_dock_render_state(&self, cx: &mut Context<Self>) -> ToolDockRenderState {
        let render_internal_dock = self.render_mode == TerminalRenderMode::Embedded;
        let layout = if render_internal_dock {
            self.terminal_tool_layout(cx)
        } else {
            TerminalToolDockLayout::default()
        };
        let sidebar_size = self.sidebar_panel_size;
        let right_width = if render_internal_dock {
            right_tool_region_width(&layout, sidebar_size)
        } else {
            px(0.0)
        };
        ToolDockRenderState {
            sidebar_size,
            right_width,
            render_internal_dock,
            left_panel: layout
                .left
                .map(|panel| self.render_internal_tool_panel(panel, SidebarPlacement::Left, cx)),
            right_panel: layout
                .right
                .map(|panel| self.render_internal_tool_panel(panel, SidebarPlacement::Right, cx)),
            bottom_panel: layout
                .bottom
                .map(|panel| self.render_internal_tool_panel(panel, SidebarPlacement::Bottom, cx)),
        }
    }

    fn render_left_region(
        &mut self,
        panel: AnyElement,
        sidebar_size: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(|| "terminal-tool-dock-left".to_string())
            .relative()
            .h_full()
            .w(sidebar_size)
            .min_w(sidebar_size)
            .max_w(sidebar_size)
            .flex_shrink_0()
            .overflow_hidden()
            .child(self.render_sidebar_resize_handle(ResizingPanel::LeftSidebar, cx))
            .child(panel)
            .into_any_element()
    }

    fn render_center_region(
        &mut self,
        state: CenterRegionState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let workspace_editor = self
            .workspace_editor
            .as_ref()
            .filter(|editor| editor.read(cx).has_open_tabs())
            .cloned();
        let primary_content = if let Some(editor) = workspace_editor {
            v_flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(editor)
                .into_any_element()
        } else {
            v_flex()
                .flex_1()
                .min_h_0()
                .min_w_0()
                .overflow_hidden()
                .child(self.render_terminal_viewport(state.font_family, cx))
                .child(self.command_bar.clone())
                .into_any_element()
        };
        v_flex()
            .debug_selector(|| "terminal-tool-dock-center".to_string())
            .flex_1()
            .h_full()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .child(primary_content)
            .child(self.render_terminal_session_footer(cx))
            .when_some(state.bottom_panel, |this, panel| {
                this.child(self.render_bottom_region(panel, state.sidebar_size, cx))
            })
            .into_any_element()
    }

    fn render_bottom_region(
        &mut self,
        panel: AnyElement,
        sidebar_size: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .debug_selector(|| "terminal-tool-dock-bottom".to_string())
            .relative()
            .w_full()
            .h(sidebar_size)
            .min_h(sidebar_size)
            .max_h(sidebar_size)
            .flex_shrink_0()
            .overflow_hidden()
            .child(self.render_sidebar_resize_handle(ResizingPanel::BottomSidebar, cx))
            .child(panel)
            .into_any_element()
    }

    fn render_right_region(
        &mut self,
        state: RightRegionState,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        h_flex()
            .debug_selector(|| "terminal-tool-dock-right".to_string())
            .h_full()
            .w(state.width)
            .min_w(state.width)
            .max_w(state.width)
            .flex_shrink_0()
            .overflow_hidden()
            .when_some(state.panel, |this, panel| {
                this.child(self.render_right_panel(panel, state.sidebar_size, cx))
            })
            .child(
                div()
                    .debug_selector(|| "terminal-tool-dock-toolbar".to_string())
                    .h_full()
                    .w(TOOLBAR_WIDTH)
                    .min_w(TOOLBAR_WIDTH)
                    .max_w(TOOLBAR_WIDTH)
                    .flex_shrink_0()
                    .child(self.sidebar_toolbar.clone()),
            )
            .into_any_element()
    }

    fn render_right_panel(
        &mut self,
        panel: AnyElement,
        sidebar_size: Pixels,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .relative()
            .h_full()
            .w(sidebar_size)
            .min_w(sidebar_size)
            .max_w(sidebar_size)
            .flex_shrink_0()
            .overflow_hidden()
            .child(self.render_sidebar_resize_handle(ResizingPanel::RightSidebar, cx))
            .child(panel)
            .into_any_element()
    }
}
