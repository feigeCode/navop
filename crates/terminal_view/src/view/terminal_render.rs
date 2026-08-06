use super::clipboard::block_selection_text_from_term;
use super::*;
use alacritty_terminal::sync::FairMutex;

fn with_terminal_if_ready<T, R>(
    term: &FairMutex<T>,
    update: impl FnOnce(&mut T) -> R,
) -> Option<R> {
    let mut term = term.try_lock_unfair()?;
    Some(update(&mut term))
}

impl TerminalView {
    pub(super) fn schedule_terminal_render_retry(&mut self, cx: &mut Context<Self>) {
        if self.terminal_render_retry.is_some() {
            return;
        }

        self.terminal_render_retry = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(8))
                .await;
            let _ = this.update(cx, |this, cx| {
                this.terminal_render_retry = None;
                cx.notify();
            });
        }));
    }

    pub(super) fn render_terminal(
        &mut self,
        font_family: SharedString,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let (is_local, term) = {
            let terminal = self.terminal.read(cx);
            (
                terminal.live_connection_kind() == Some(TerminalConnectionKind::Local),
                terminal.term().clone(),
            )
        };

        let updated = with_terminal_if_ready(&term, |term| {
            let cursor = term.grid().cursor.point;
            let display_offset = term.grid().display_offset();
            self.terminal_frame_snapshot = TerminalFrameSnapshot {
                mode: *term.mode(),
                display_offset,
                history_size: term.history_size(),
                screen_lines: term.screen_lines(),
                columns: term.columns(),
                selection_present: term.selection.is_some(),
                selection_text: term.selection_to_string(),
                block_selection_text: block_selection_text_from_term(term, self.block_selection),
                cursor_screen_line: cursor.line.0 + display_offset as i32,
                cursor_column: cursor.column.0,
            };

            // Keep terminal parsing off the GPUI critical path. A fair blocking
            // lock here can freeze the entire window while a large PTY chunk is
            // being parsed. If the parser owns the lock, preserve the previous
            // render cache and retry on a later frame instead.
            {
                let display_offset = term.grid().display_offset();
                let visible_lines = 0..term.screen_lines();
                let context = TerminalAddonFrameContext {
                    term,
                    visible_lines,
                    display_offset,
                    is_local,
                    base_dir: self.local_working_dir.as_deref(),
                };
                self.addon_manager.dispatch_frame(&context);
            }

            self.render_cache.update(
                term,
                &self.addon_manager,
                &self.current_theme,
                self.block_selection
                    .filter(|selection| !selection.is_empty())
                    .map(|selection| selection.bounds()),
            );
        });
        if updated.is_none() {
            // A skipped frame must arrange another attempt: the Wakeup that
            // triggered this render has already been consumed.
            self.schedule_terminal_render_retry(cx);
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
            self.performance_metrics.clone(),
            self.focus_handle.clone(),
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
        let view_paste_selection = view.clone();
        let view_paste = view.clone();
        let view_select_all = view.clone();
        let view_clear_screen = view.clone();
        let view_clear = view.clone();
        let can_paste_selection =
            accepts_live_input && selection_text.as_ref().is_some_and(|text| !text.is_empty());
        let selection_text_for_paste = selection_text.clone();
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
            // 粘贴选中内容
            .item(
                PopupMenuItem::new(t!("ContextMenu.paste_selection"))
                    .disabled(!can_paste_selection)
                    .on_click(move |_, window, cx| {
                        let Some(selection_text) = selection_text_for_paste.clone() else {
                            return;
                        };
                        let _ = view_paste_selection.update(cx, |this, cx| {
                            this.paste_text(&selection_text, window, cx);
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

#[cfg(test)]
mod tests {
    use super::with_terminal_if_ready;
    use alacritty_terminal::sync::FairMutex;

    #[test]
    fn terminal_frame_lock_attempt_never_waits_for_parser() {
        let term = FairMutex::new(1usize);
        let parser_guard = term.lock();

        assert_eq!(None, with_terminal_if_ready(&term, |value| *value += 1));

        drop(parser_guard);
        assert_eq!(Some(()), with_terminal_if_ready(&term, |value| *value += 1));
        assert_eq!(2, *term.lock());
    }
}
