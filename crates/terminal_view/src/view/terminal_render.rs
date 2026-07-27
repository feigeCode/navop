use super::*;

impl TerminalView {
    pub(super) fn render_terminal(
        &mut self,
        font_family: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        // Prepare addons before rendering
        {
            let is_local = self.terminal.read(cx).live_connection_kind()
                == Some(TerminalConnectionKind::Local);
            let term = self.terminal.read(cx).term().lock();
            let display_offset = term.grid().display_offset();
            let visible_lines = 0..term.screen_lines();
            let context = TerminalAddonFrameContext {
                term: &term,
                visible_lines,
                display_offset,
                is_local,
                base_dir: self.local_working_dir.as_deref(),
            };
            self.addon_manager.dispatch_frame(&context);
        }

        // Update render cache with decorations from all addons
        {
            let term = self.terminal.read(cx).term().clone();
            let mut term = term.lock();

            self.render_cache.update(
                &mut term,
                &self.addon_manager,
                &self.current_theme,
                self.block_selection
                    .filter(|selection| !selection.is_empty())
                    .map(|selection| selection.bounds()),
            );
        }

        // 获取光标可见性
        let cursor_visible = if self.cursor_blink_enabled {
            self.blink_manager.read(cx).visible()
        } else {
            true
        };

        TerminalElement::new(
            &self.render_cache,
            font_family,
            self.font_size,
            self.font_fallbacks.iter().map(|s| s.to_string()).collect(),
            self.line_height_scale,
            cursor_visible,
            self.cell_width, // 传入预计算的 cell_width，确保与 resize 一致
        )
        .into_element()
    }

    /// 构建终端右键菜单
    pub(super) fn build_context_menu(
        menu: PopupMenu,
        has_selection: bool,
        selection_text: Option<String>,
        accepts_live_input: bool,
        view: &Entity<Self>,
        sidebar: &Entity<TerminalSidebar>,
        _window: &mut Window,
        _cx: &mut Context<PopupMenu>,
    ) -> PopupMenu {
        let view_copy = view.clone();
        let view_paste = view.clone();
        let view_select_all = view.clone();
        let view_clear_screen = view.clone();
        let view_clear = view.clone();
        let copy_shortcut = terminal_shortcut_label(TERMINAL_COPY_SHORTCUT);
        let paste_shortcut = terminal_shortcut_label(TERMINAL_PASTE_SHORTCUT);
        let select_all_shortcut = terminal_shortcut_label(TERMINAL_SELECT_ALL_SHORTCUT);
        let clear_screen_shortcut = terminal_shortcut_label(TERMINAL_CLEAR_SCREEN_SHORTCUT);

        let mut menu = menu
            // 复制
            .item(
                PopupMenuItem::new(t!(
                    "ContextMenu.copy_with_shortcut",
                    shortcut = copy_shortcut
                ))
                .icon(IconName::Copy)
                .action(Box::new(Copy))
                .disabled(!has_selection)
                .on_click(move |_, window, cx| {
                    let _ = view_copy.update(cx, |this, cx| {
                        this.copy(&Copy, window, cx);
                    });
                }),
            )
            // 粘贴
            .item(
                PopupMenuItem::new(t!(
                    "ContextMenu.paste_with_shortcut",
                    shortcut = paste_shortcut
                ))
                .action(Box::new(Paste))
                .disabled(!accepts_live_input)
                .on_click(move |_, window, cx| {
                    let _ = view_paste.update(cx, |this, cx| {
                        this.paste(&Paste, window, cx);
                    });
                }),
            )
            .separator()
            .item(
                PopupMenuItem::new(t!(
                    "ContextMenu.clear_screen_with_shortcut",
                    shortcut = clear_screen_shortcut
                ))
                .icon(IconName::Delete)
                .action(Box::new(ClearScreen))
                .disabled(!accepts_live_input)
                .on_click(move |_, window, cx| {
                    let _ = view_clear_screen.update(cx, |this, cx| {
                        this.clear_screen(&ClearScreen, window, cx);
                    });
                }),
            )
            .separator()
            // 全选
            .item(
                PopupMenuItem::new(t!(
                    "ContextMenu.select_all_with_shortcut",
                    shortcut = select_all_shortcut
                ))
                .action(Box::new(SelectAll))
                .on_click(move |_, window, cx| {
                    let _ = view_select_all.update(cx, |this, cx| {
                        this.select_all(&SelectAll, window, cx);
                    });
                }),
            )
            // 清除选择
            .item(
                PopupMenuItem::new(t!("ContextMenu.clear_selection"))
                    .action(Box::new(ClearSelection))
                    .disabled(!has_selection)
                    .on_click(move |_, window, cx| {
                        let _ = view_clear.update(cx, |this, cx| {
                            this.clear_selection(&ClearSelection, window, cx);
                        });
                    }),
            );

        // 询问AI（仅在有选中文本时可用）
        if let Some(text) = selection_text {
            let message = format!(
                "{}",
                t!(
                    "TerminalView.ask_ai_selection_template",
                    content = text.trim()
                )
            );
            let sidebar_clone = sidebar.clone();
            menu = menu.separator().item(
                PopupMenuItem::new(t!("ContextMenu.ask_ai"))
                    .icon(IconName::AI.color())
                    .on_click(move |_, _window, cx| {
                        sidebar_clone.update(cx, |sidebar, cx| {
                            sidebar.ask_ai(message.clone(), cx);
                        });
                    }),
            );

            let save_text = text.trim().to_string();
            let sidebar_quick = sidebar.clone();
            if !save_text.is_empty() {
                menu = menu.item(
                    PopupMenuItem::new(t!("ContextMenu.save_quick_command"))
                        .icon(IconName::SquareTerminal)
                        .on_click(move |_, window, cx| {
                            sidebar_quick.update(cx, |sidebar, cx| {
                                sidebar.add_quick_command(save_text.clone(), window, cx);
                            });
                        }),
                );
            }
        }

        menu
    }
}
