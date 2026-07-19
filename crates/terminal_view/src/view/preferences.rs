use super::*;

impl TerminalView {
    pub fn sync_sidebar_theme(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.current_theme.clone();
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.update_current_theme(&theme, window, cx);
        });
    }

    pub fn set_auto_copy(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.auto_copy_on_select == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.auto_copy = enabled;
        });
    }

    pub fn apply_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.autocomplete_enabled == enabled {
            return;
        }
        self.autocomplete_enabled = enabled;
        if !enabled {
            self.suggestion_debounce.take();
            self.dismiss_history_prompt();
            self.hide_history_prompt_dropdown();
            self.dismiss_history_prompt_matches();
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_autocomplete_enabled(enabled, cx);
        });
        cx.notify();
    }

    pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.autocomplete_enabled == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.enable_autocomplete = enabled;
        });
    }

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.middle_click_paste == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.middle_click_paste = enabled;
        });
    }

    pub fn set_right_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.right_click_paste == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.right_click_paste = enabled;
        });
    }

    pub fn set_paste_image_upload(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.paste_image_upload == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.paste_image_upload = enabled;
        });
    }

    pub fn set_vim_scroll_to_arrow_keys(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.vim_scroll_to_arrow_keys == enabled {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.vim_scroll_to_arrow_keys = enabled;
        });
    }

    /// 增大字体
    pub fn increase_font_size(&mut self, cx: &mut Context<Self>) {
        let current = f32::from(self.font_size);
        self.set_font_size(current + 1.0, cx);
    }

    /// 减小字体
    pub fn decrease_font_size(&mut self, cx: &mut Context<Self>) {
        let current = f32::from(self.font_size);
        self.set_font_size(current - 1.0, cx);
    }

    /// 重置字体大小为默认值
    pub fn reset_font_size(&mut self, cx: &mut Context<Self>) {
        self.set_font_size(TERMINAL_RESET_FONT_SIZE, cx);
    }

    /// 获取当前字体大小
    pub fn font_size(&self) -> f32 {
        f32::from(self.font_size)
    }

    /// 设置主字体
    pub fn set_font_family(&mut self, family: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.font_family = normalize_terminal_primary_font(family.into().as_ref()).into();
        self.font_metrics = None;
        self.last_size = None;
        cx.notify();
    }

    /// 获取当前主字体
    pub fn font_family(&self) -> &SharedString {
        &self.font_family
    }

    /// 设置行高比例
    pub fn set_line_height_scale(&mut self, scale: f32, cx: &mut Context<Self>) {
        self.line_height_scale = scale.clamp(1.0, 2.5);
        self.line_height = self.font_size * self.line_height_scale;
        self.last_size = None;
        cx.notify();
    }

    /// 获取当前行高比例
    pub fn line_height_scale(&self) -> f32 {
        self.line_height_scale
    }

    pub fn reconnect(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let working_dir = self
            .terminal
            .read(cx)
            .current_working_dir()
            .map(str::to_string);
        self.focus_terminal_after_connect = true;
        self.terminal.update(cx, |terminal, cx| {
            terminal.reconnect(cx);
        });

        cx.spawn(async move |this, cx| {
            loop {
                let state = match this.update(cx, |this, cx| {
                    this.terminal.read(cx).connection_state().clone()
                }) {
                    Ok(state) => state,
                    Err(_) => break,
                };

                match state {
                    ConnectionState::Connected => {
                        let _ = this.update(cx, |this, cx| {
                            this.sidebar.update(cx, |sidebar, cx| {
                                sidebar.reconnect_file_manager(working_dir.clone(), cx);
                                sidebar.reconnect_server_monitor(cx);
                            });
                        });
                        break;
                    }
                    ConnectionState::Disconnected { .. } => break,
                    ConnectionState::Connecting => {
                        cx.background_executor()
                            .timer(Duration::from_millis(100))
                            .await;
                    }
                }
            }
        })
        .detach();
    }
}
