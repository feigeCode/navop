use super::*;

impl TerminalView {
    pub(super) fn select_all(
        &mut self,
        _: &SelectAll,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.block_selection = None;
        self.mouse_state.block_selecting = false;
        self.apply_or_queue_terminal_action(PendingTerminalAction::SelectAll, Some(_window), cx);
    }

    pub(super) fn clear_screen(
        &mut self,
        _: &ClearScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.accepts_live_terminal_input(cx) {
            return;
        }
        self.clear_history_prompt();
        self.apply_or_queue_terminal_action(PendingTerminalAction::ClearScreen, Some(window), cx);
        self.focus_terminal(window, cx);
    }

    pub(super) fn reset_render_cache(&mut self, cx: &mut Context<Self>) {
        self.pending_render_cache_reset = true;
        self.apply_pending_render_cache_reset(cx);
    }

    pub(super) fn apply_pending_render_cache_reset(&mut self, cx: &mut Context<Self>) {
        if !self.pending_render_cache_reset {
            return;
        }
        let term = self.terminal.read(cx).term().clone();
        let Some(term) = term.try_lock_unfair() else {
            self.schedule_terminal_render_retry(cx);
            return;
        };
        self.render_cache =
            RenderCache::new(term.screen_lines(), term.columns(), term.colors().clone());
        self.pending_render_cache_reset = false;
    }

    pub(super) fn clear_selection(
        &mut self,
        _: &ClearSelection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 如果侧边栏有激活的面板，按 Escape 关闭它
        if self.sidebar.read(cx).active_panel().is_some() {
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.set_active_panel(None, cx);
            });
            // 清除搜索
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.set_search_value("", window, cx);
            });
            if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
                search.clear();
            }
            self.invalidate_terminal_searches();
            cx.notify();
            return;
        }

        let accepts_live_input = self.accepts_live_terminal_input(cx);
        let had_block_selection = self.block_selection.take().is_some();
        self.mouse_state.block_selecting = false;
        if had_block_selection {
            cx.notify();
        }
        self.apply_or_queue_terminal_action(
            PendingTerminalAction::ResolveClearSelection {
                accepts_live_input,
                had_block_selection,
            },
            Some(window),
            cx,
        );
    }

    pub(super) fn search_forward(
        &mut self,
        _: &SearchForward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 如果侧边栏设置面板未激活，则激活它
        if self.sidebar.read(cx).active_panel() != Some(SidebarPanel::Settings) {
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.set_active_panel(Some(SidebarPanel::Settings), cx);
            });
            cx.notify();
            return;
        }
        // 执行向前搜索
        self.search_forward_internal(cx);
    }

    pub(super) fn search_backward(
        &mut self,
        _: &SearchBackward,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 如果侧边栏设置面板未激活，则激活它
        if self.sidebar.read(cx).active_panel() != Some(SidebarPanel::Settings) {
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.set_active_panel(Some(SidebarPanel::Settings), cx);
            });
            cx.notify();
            return;
        }
        // 执行向后搜索
        self.search_backward_internal(cx);
    }

    pub fn set_search_pattern(&mut self, pattern: &str) -> Result<()> {
        let changed = if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            search
                .set_pattern(pattern)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
            true
        } else {
            false
        };
        if changed {
            self.invalidate_terminal_searches();
        }
        Ok(())
    }
}
