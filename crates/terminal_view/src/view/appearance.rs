use super::*;

impl TerminalView {
    /// 获取连接类型（本地 / SSH）
    pub fn connection_kind(&self, cx: &App) -> TerminalConnectionKind {
        self.terminal.read(cx).connection_kind()
    }

    /// 获取 SSH 连接 ID（本地终端返回 None）
    pub fn connection_id(&self, cx: &App) -> Option<i64> {
        self.terminal.read(cx).connection_id()
    }

    /// 获取本地终端的工作目录
    pub fn local_working_dir(&self) -> Option<&std::path::Path> {
        self.local_working_dir.as_deref()
    }

    /// 设置字体大小
    pub fn set_font_size(&mut self, size: f32, cx: &mut Context<Self>) {
        let clamped = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        let current = f32::from(self.font_size);
        if (current - clamped).abs() < f32::EPSILON {
            return;
        }
        let _ = update_settings(cx, move |settings| {
            settings.font_size = clamped;
        });
    }

    pub fn apply_terminal_settings(
        &mut self,
        font_size: f32,
        font_family: String,
        auto_copy: bool,
        autocomplete_enabled: bool,
        middle_click_paste: bool,
        right_click_paste: bool,
        paste_image_upload: bool,
        sync_path: bool,
        vim_scroll_to_arrow_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 字体大小
        let clamped = font_size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
        let current = f32::from(self.font_size);
        if (current - clamped).abs() >= f32::EPSILON {
            self.font_size = px(clamped);
            self.line_height = self.font_size * self.line_height_scale;
            self.font_metrics = None;
            self.last_size = None;
        }
        let font_family = SharedString::from(normalize_terminal_primary_font(&font_family));
        if self.font_family != font_family {
            self.font_family = font_family.clone();
            self.font_metrics = None;
            self.last_size = None;
        }

        self.auto_copy_on_select = auto_copy;
        self.apply_autocomplete_enabled(autocomplete_enabled, cx);
        if !self.history_prompt_enabled(cx) {
            self.suggestion_debounce.take();
            self.hide_history_prompt_dropdown();
            self.dismiss_history_prompt_matches();
        }
        self.middle_click_paste = middle_click_paste;
        self.right_click_paste = right_click_paste;
        self.paste_image_upload = paste_image_upload;
        self.vim_scroll_to_arrow_keys = vim_scroll_to_arrow_keys;

        self.terminal.update(cx, |terminal, _cx| {
            terminal.set_sync_path_with_terminal(sync_path);
        });

        let theme = self.current_theme.clone();
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.update_current_theme(&theme, window, cx);
            sidebar.set_font_size(clamped, window, cx);
            sidebar.set_font_family(font_family, window, cx);
            sidebar.set_auto_copy(auto_copy, cx);
            sidebar.set_middle_click_paste(middle_click_paste, cx);
            sidebar.set_right_click_paste(right_click_paste, cx);
            sidebar.set_paste_image_upload(paste_image_upload, cx);
            sidebar.set_vim_scroll_to_arrow_keys(vim_scroll_to_arrow_keys, cx);
            sidebar.set_sync_path_enabled(sync_path, cx);
        });

        cx.notify();
    }

    pub(super) fn apply_settings_snapshot(
        &mut self,
        settings: &TerminalSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.apply_terminal_settings(
            settings.font_size,
            settings.font_family.clone(),
            settings.auto_copy,
            settings.enable_autocomplete,
            settings.middle_click_paste,
            settings.right_click_paste,
            settings.paste_image_upload,
            settings.sync_path_with_terminal,
            settings.vim_scroll_to_arrow_keys,
            window,
            cx,
        );
        self.terminal.update(cx, |terminal, _cx| {
            terminal.set_scrollback_lines(settings.scrollback_lines);
        });
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_scrollback_lines(settings.scrollback_lines, window, cx);
        });
        self.apply_cursor_blink(settings.cursor_blink, window, cx);
        self.apply_confirm_multiline_paste(settings.confirm_multiline_paste, cx);
        self.apply_confirm_high_risk_command(settings.confirm_high_risk_command, cx);
        self.apply_custom_highlight_rules(&settings.custom_highlights, cx);
        let theme = TerminalTheme::from_application_theme(cx.theme());
        self.apply_theme(&theme, window, cx);
    }

    pub(super) fn apply_custom_highlight_rules(
        &mut self,
        rules: &[TerminalHighlightRule],
        cx: &mut Context<Self>,
    ) {
        if let Some(addon) = self
            .addon_manager
            .get_as_mut::<CustomHighlightAddon>("custom_highlights")
        {
            addon.set_rules(rules);
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_custom_highlights(rules.to_vec(), cx);
        });
        cx.notify();
    }

    /// 应用主题（不 emit 事件，用于跨 tab 同步）
    pub fn apply_theme(
        &mut self,
        theme: &TerminalTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.current_theme == *theme {
            return;
        }
        self.current_theme = theme.clone();
        self.sync_sidebar_theme(window, cx);
        cx.emit(TabContentEvent::StateChanged);
        cx.notify();
    }

    /// 应用光标闪烁（不 emit 事件，用于跨 tab 同步）
    pub fn apply_cursor_blink(
        &mut self,
        enabled: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cursor_blink_enabled = enabled;
        if enabled {
            if self.focus_handle.is_focused(window) {
                self.blink_manager.update(cx, BlinkCursor::start);
            }
        } else {
            self.blink_manager.update(cx, BlinkCursor::stop);
        }
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_cursor_blink(enabled, cx);
        });
        cx.notify();
    }

    /// 应用多行粘贴确认（不 emit 事件，用于跨 tab 同步）
    pub fn apply_confirm_multiline_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.confirm_multiline_paste = enabled;
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_confirm_multiline_paste(enabled, cx);
        });
        cx.notify();
    }

    /// 应用高危命令确认（不 emit 事件，用于跨 tab 同步）
    pub fn apply_confirm_high_risk_command(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.confirm_high_risk_command = enabled;
        self.sidebar.update(cx, |sidebar, cx| {
            sidebar.set_confirm_high_risk_command(enabled, cx);
        });
        cx.notify();
    }
}
