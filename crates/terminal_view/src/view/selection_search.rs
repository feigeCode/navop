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
        // 如果侧边栏有激活的面板且焦点在侧边栏上（例如面板输入框），按 Escape 关闭它。
        // 终端聚焦时 Escape 属于终端本身，必须继续往下走并发送给 PTY，
        // 否则面板打开期间 vim 等程序永远收不到 Escape。
        if self.sidebar.read(cx).active_panel().is_some()
            && self.sidebar.read(cx).focus_handle(cx).is_focused(window)
        {
            self.sidebar.update(cx, |sidebar, cx| {
                sidebar.set_active_panel(None, cx);
            });
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

    pub(super) fn open_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.search_open = true;
        let focus_handle = self.search_input.read(cx).focus_handle(cx);
        focus_handle.focus(window, cx);
        cx.notify();
    }

    pub(super) fn close_terminal_search(
        &mut self,
        _: &CloseTerminalSearch,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.close_search(window, cx);
    }

    pub(super) fn close_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.search_open {
            return;
        }
        self.search_open = false;
        let search_input = self.search_input.clone();
        search_input.update(cx, |input, cx| input.set_value("", window, cx));
        if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            search.clear();
        }
        self.invalidate_terminal_searches();
        // 浮层卸载前把焦点还给终端，避免后续快捷键和终端输入丢失。
        self.focus_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn search_forward(
        &mut self,
        _: &SearchForward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_open {
            self.open_search(window, cx);
            return;
        }
        self.search_forward_internal(cx);
    }

    pub(super) fn search_backward(
        &mut self,
        _: &SearchBackward,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.search_open {
            self.open_search(window, cx);
            return;
        }
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

    pub(super) fn set_search_options(&mut self, options: TerminalSearchOptions) -> Result<()> {
        let changed = if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            search
                .set_options(options)
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
