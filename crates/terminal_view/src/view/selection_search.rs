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
        self.terminal.update(cx, |terminal, _| {
            terminal.select_all();
        });
        cx.notify();
    }

    pub(super) fn clear_screen(
        &mut self,
        _: &ClearScreen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.clear_history_prompt();
        self.terminal.update(cx, |terminal, cx| {
            terminal.clear_screen(cx);
        });
        self.reset_render_cache(cx);
        self.focus_terminal(window, cx);
        cx.notify();
    }

    pub(super) fn reset_render_cache(&mut self, cx: &mut Context<Self>) {
        let (screen_lines, columns, colors) = {
            let terminal = self.terminal.read(cx);
            let term = terminal.term().lock();
            (term.screen_lines(), term.columns(), term.colors().clone())
        };
        self.render_cache = RenderCache::new(screen_lines, columns, colors);
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
            cx.notify();
            return;
        }

        let had_block_selection = self.block_selection.take().is_some();
        self.mouse_state.block_selecting = false;

        let term = self.terminal.read(cx).term().clone();
        let mut term_lock = term.lock();
        let in_vi_mode = term_lock.mode().contains(TermMode::VI);
        let has_selection = term_lock.selection.is_some();

        if in_vi_mode {
            if has_selection {
                term_lock.selection = None;
            } else {
                term_lock.toggle_vi_mode();
            }
            drop(term_lock);
            cx.notify();
        } else if has_selection || had_block_selection {
            if has_selection {
                term_lock.selection = None;
            }
            drop(term_lock);
            cx.notify();
        } else {
            drop(term_lock);
            self.write_to_pty(b"\x1b".to_vec(), cx);
        }
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
        if let Some(search) = self.addon_manager.get_as_mut::<SearchAddon>("search") {
            search
                .set_pattern(pattern)
                .map_err(|e| anyhow::anyhow!("{}", e))?;
        }
        Ok(())
    }
}
