//! 终端设置面板
//!
//! 提供搜索、字体设置和主题切换功能

use gpui::FontWeight;
use gpui::prelude::FluentBuilder;
use gpui::{
    AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    InteractiveElement, IntoElement, MouseButton, ParentElement, Pixels, Render, SharedString,
    Styled, Subscription, Window, div, px,
};
use gpui_component::{
    ActiveTheme, Colorize, Icon, IconName, Sizable, Size, WindowExt,
    button::{Button, ButtonVariants},
    color_picker::{ColorPicker, ColorPickerState},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    notification::Notification,
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectItem, SelectState},
    switch::Switch,
    try_parse_color, v_flex,
};
use rust_i18n::t;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    TerminalHighlightRule,
    theme::{
        MAX_FONT_SIZE, MIN_FONT_SIZE, TerminalColors, TerminalTheme, available_monospace_fonts,
        is_supported_terminal_primary_font,
    },
};
use one_core::settings::{AppSettings, CustomFont, is_installed_font_family};

#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalFontOption {
    value: SharedString,
    label: SharedString,
}

impl TerminalFontOption {
    fn new(font_family: &str, installed_font_names: &[String]) -> Self {
        let label = if is_installed_font_family(font_family, installed_font_names) {
            SharedString::from(font_family)
        } else {
            format!("{} (未安装)", font_family).into()
        };

        Self {
            value: font_family.into(),
            label,
        }
    }
}

impl SelectItem for TerminalFontOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }

    fn matches(&self, query: &str) -> bool {
        self.value.to_lowercase().contains(&query.to_lowercase())
    }
}

fn terminal_font_options(
    custom_fonts: &[CustomFont],
    installed_font_names: &[String],
) -> Vec<TerminalFontOption> {
    let mut fonts = available_monospace_fonts()
        .into_iter()
        .map(|font| TerminalFontOption::new(font, installed_font_names))
        .collect::<Vec<_>>();

    for family in custom_fonts
        .iter()
        .flat_map(|font| font.monospace_families.iter())
    {
        let family = family.trim();
        if !is_supported_terminal_primary_font(family)
            || fonts
                .iter()
                .any(|existing| existing.value.as_ref() == family)
        {
            continue;
        }
        fonts.push(TerminalFontOption::new(family, installed_font_names));
    }

    fonts
}

/// 设置面板事件
#[derive(Clone, Debug)]
pub enum SettingsPanelEvent {
    /// 关闭面板
    Close,
    /// 搜索模式变化
    SearchPatternChanged(String),
    /// 搜索前一个
    SearchPrevious,
    /// 搜索下一个
    SearchNext,
    /// 字体大小变更
    FontSizeChanged(f32),
    /// 字体变更
    FontFamilyChanged(String),
    /// 主题变更
    ThemeChanged(TerminalTheme),
    /// 滚屏历史保留行数变更
    ScrollbackLinesChanged(usize),
    /// 光标闪烁变更
    CursorBlinkChanged(bool),
    /// 非 bracketed 模式下，多行粘贴确认开关
    ConfirmMultilinePasteChanged(bool),
    /// 高危命令确认开关
    ConfirmHighRiskCommandChanged(bool),
    /// 自动会话日志开关
    AutoSessionLoggingChanged(bool),
    /// 选中自动复制开关
    AutoCopyChanged(bool),
    /// 自动补全开关
    AutocompleteChanged(bool),
    /// 弹框候选词开关
    SuggestionPopupChanged(bool),
    /// 中键粘贴开关
    MiddleClickPasteChanged(bool),
    /// 右键快速粘贴开关
    RightClickPasteChanged(bool),
    /// SSH 粘贴图片上传开关
    PasteImageUploadChanged(bool),
    /// vim/TUI 滚轮转方向键开关
    VimScrollToArrowKeysChanged(bool),
    /// 选中文本高亮相同内容开关
    SelectionHighlightChanged(bool),
    /// 路径同步开关变更
    SyncPathChanged(bool),
    /// 自定义高亮规则变更
    CustomHighlightsChanged(Vec<TerminalHighlightRule>),
}

/// 设置面板组件
pub struct SettingsPanel {
    /// 搜索输入框状态
    search_input_state: Entity<InputState>,
    /// 字体大小输入框状态
    font_size_input_state: Entity<InputState>,
    /// 滚屏历史行数输入框状态
    scrollback_lines_input_state: Entity<InputState>,
    /// 字体选择状态
    font_select_state: Entity<SelectState<Vec<TerminalFontOption>>>,
    /// 当前主题
    current_theme: TerminalTheme,
    /// 当前终端字体大小
    font_size: f32,
    /// 当前终端字体
    font_family: SharedString,
    /// 字体大小输入变更抑制
    suppress_font_size_change: bool,
    /// 当前滚屏历史保留行数
    scrollback_lines: usize,
    /// 滚屏历史行数输入变更抑制
    suppress_scrollback_lines_change: bool,
    /// 光标闪烁开关
    cursor_blink: bool,
    /// 非 bracketed 模式下，多行粘贴确认
    confirm_multiline_paste: bool,
    /// 高危命令确认
    confirm_high_risk_command: bool,
    /// 自动保存终端、SSH、串口会话日志
    auto_session_logging: bool,
    /// 选中自动复制
    auto_copy: bool,
    /// 自动补全
    autocomplete_enabled: bool,
    /// 弹框候选词
    suggestion_popup_enabled: bool,
    /// 中键粘贴
    middle_click_paste: bool,
    /// 右键快速粘贴
    right_click_paste: bool,
    /// SSH 粘贴图片上传
    paste_image_upload: bool,
    /// vim/TUI 滚轮转方向键
    vim_scroll_to_arrow_keys: bool,
    /// 选中文本高亮相同内容
    selection_highlight: bool,
    /// 路径与终端同步开关
    sync_path: bool,
    /// 全局自定义高亮规则
    custom_highlights: Vec<TerminalHighlightRule>,
    /// 是否有文件管理器面板（仅 SSH 终端有）
    has_file_manager: bool,
    /// 焦点句柄
    focus_handle: FocusHandle,
    /// 订阅
    _subscriptions: Vec<Subscription>,
}

impl SettingsPanel {
    pub fn new(
        initial_theme: &TerminalTheme,
        initial_font_size: Pixels,
        initial_font_family: SharedString,
        has_file_manager: bool,
        auto_copy: bool,
        autocomplete_enabled: bool,
        suggestion_popup_enabled: bool,
        middle_click_paste: bool,
        right_click_paste: bool,
        paste_image_upload: bool,
        sync_path: bool,
        vim_scroll_to_arrow_keys: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input_state =
            cx.new(|cx| InputState::new(window, cx).placeholder(t!("Settings.search_placeholder")));

        // 字体大小输入框
        let font_size = f32::from(initial_font_size);
        let font_size_input_state = cx.new(|cx| InputState::new(window, cx).placeholder("13"));
        font_size_input_state.update(cx, |state: &mut InputState, cx| {
            state.set_value(&format!("{:.0}", font_size), window, cx);
        });

        let scrollback_lines = AppSettings::global(cx).terminal_scrollback_lines;
        let auto_session_logging = AppSettings::global(cx).terminal_auto_session_logging;
        let selection_highlight = AppSettings::global(cx).terminal_selection_highlight;
        let scrollback_lines_input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(AppSettings::DEFAULT_TERMINAL_SCROLLBACK_LINES.to_string())
                .pattern(
                    regex::Regex::new(r"^\d{0,7}$")
                        .expect("terminal scrollback lines regex 应可编译"),
                )
        });
        scrollback_lines_input_state.update(cx, |state: &mut InputState, cx| {
            state.set_value(&scrollback_lines.to_string(), window, cx);
        });

        // 字体选择列表
        let installed_font_names = cx.text_system().all_font_names();
        let fonts = terminal_font_options(
            AppSettings::global(cx).custom_fonts.as_slice(),
            &installed_font_names,
        );

        // 找到当前字体的索引
        let current_font = initial_font_family.to_string();
        let selected_index = fonts
            .iter()
            .position(|f| f.value.as_ref() == current_font)
            .map(|i| gpui_component::IndexPath::default().row(i));

        let font_select_state =
            cx.new(|cx| SelectState::new(fonts, selected_index, window, cx).searchable(true));

        let mut subscriptions = Vec::new();

        // 订阅搜索输入事件
        let input_entity = search_input_state.clone();
        subscriptions.push(cx.subscribe_in(
            &search_input_state,
            window,
            move |_this, _state, event, _window, cx| match event {
                InputEvent::Change => {
                    let value = input_entity.read(cx).value().to_string();
                    cx.emit(SettingsPanelEvent::SearchPatternChanged(value));
                }
                InputEvent::PressEnter { secondary, .. } => {
                    if *secondary {
                        cx.emit(SettingsPanelEvent::SearchPrevious);
                    } else {
                        cx.emit(SettingsPanelEvent::SearchNext);
                    }
                }
                _ => {}
            },
        ));

        // 订阅字体大小输入事件
        let font_size_entity = font_size_input_state.clone();
        subscriptions.push(cx.subscribe_in(
            &font_size_input_state,
            window,
            move |this, _state, event: &InputEvent, _window, cx| match event {
                InputEvent::Change => {
                    if this.suppress_font_size_change {
                        return;
                    }
                    let value = font_size_entity.read(cx).value().to_string();
                    if let Ok(size) = value.parse::<f32>() {
                        let clamped: f32 = size.clamp(MIN_FONT_SIZE, MAX_FONT_SIZE);
                        this.font_size = clamped;
                        cx.emit(SettingsPanelEvent::FontSizeChanged(clamped));
                    }
                }
                _ => {}
            },
        ));

        // 订阅字体大小步进事件
        let font_size_entity2 = font_size_input_state.clone();
        subscriptions.push(cx.subscribe_in(
            &font_size_input_state,
            window,
            move |this, _state, event: &NumberInputEvent, window, cx| match event {
                NumberInputEvent::Step(action) => {
                    let current = this.font_size;
                    let new_size = match action {
                        StepAction::Increment => (current + 1.0).min(MAX_FONT_SIZE),
                        StepAction::Decrement => (current - 1.0).max(MIN_FONT_SIZE),
                    };
                    this.font_size = new_size;
                    font_size_entity2.update(cx, |state: &mut InputState, cx| {
                        state.set_value(&format!("{:.0}", new_size), window, cx);
                    });
                    cx.emit(SettingsPanelEvent::FontSizeChanged(new_size));
                }
            },
        ));

        let scrollback_lines_entity = scrollback_lines_input_state.clone();
        subscriptions.push(cx.subscribe_in(
            &scrollback_lines_input_state,
            window,
            move |this, _state, event: &InputEvent, _window, cx| {
                if !matches!(event, InputEvent::Change) || this.suppress_scrollback_lines_change {
                    return;
                }

                let value = scrollback_lines_entity.read(cx).value().to_string();
                if let Ok(lines) = value.parse::<usize>() {
                    let lines = AppSettings::normalize_terminal_scrollback_lines(lines);
                    this.scrollback_lines = lines;
                    cx.emit(SettingsPanelEvent::ScrollbackLinesChanged(lines));
                }
            },
        ));

        let scrollback_lines_step_entity = scrollback_lines_input_state.clone();
        subscriptions.push(cx.subscribe_in(
            &scrollback_lines_input_state,
            window,
            move |this, _state, event: &NumberInputEvent, window, cx| match event {
                NumberInputEvent::Step(action) => {
                    let lines = match action {
                        StepAction::Increment => this.scrollback_lines.saturating_add(1_000),
                        StepAction::Decrement => this.scrollback_lines.saturating_sub(1_000),
                    };
                    let lines = AppSettings::normalize_terminal_scrollback_lines(lines);
                    this.scrollback_lines = lines;
                    scrollback_lines_step_entity.update(cx, |state: &mut InputState, cx| {
                        state.set_value(&lines.to_string(), window, cx);
                    });
                    cx.emit(SettingsPanelEvent::ScrollbackLinesChanged(lines));
                }
            },
        ));

        // 订阅字体选择事件
        subscriptions.push(cx.subscribe_in(
            &font_select_state,
            window,
            move |this, _state, event: &SelectEvent<Vec<TerminalFontOption>>, _window, cx| {
                if let SelectEvent::Confirm(Some(font_family)) = event {
                    this.font_family = font_family.clone();
                    cx.emit(SettingsPanelEvent::FontFamilyChanged(
                        font_family.to_string(),
                    ));
                }
            },
        ));

        Self {
            search_input_state,
            font_size_input_state,
            scrollback_lines_input_state,
            font_select_state,
            current_theme: initial_theme.clone(),
            font_size,
            font_family: initial_font_family,
            suppress_font_size_change: false,
            scrollback_lines,
            suppress_scrollback_lines_change: false,
            cursor_blink: false,
            confirm_multiline_paste: true,
            confirm_high_risk_command: true,
            auto_session_logging,
            auto_copy,
            autocomplete_enabled,
            suggestion_popup_enabled,
            middle_click_paste,
            right_click_paste,
            paste_image_upload,
            sync_path,
            vim_scroll_to_arrow_keys,
            selection_highlight,
            custom_highlights: Vec::new(),
            has_file_manager,
            focus_handle: cx.focus_handle(),
            _subscriptions: subscriptions,
        }
    }

    /// 设置当前主题
    pub fn set_current_theme(
        &mut self,
        theme: TerminalTheme,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.current_theme = theme;
        cx.notify();
    }

    pub fn set_font_size(&mut self, font_size: f32, window: &mut Window, cx: &mut Context<Self>) {
        self.font_size = font_size;
        self.suppress_font_size_change = true;
        self.font_size_input_state.update(cx, |state, cx| {
            state.set_value(&format!("{:.0}", font_size), window, cx);
        });
        self.suppress_font_size_change = false;
        cx.notify();
    }

    pub fn set_font_family(
        &mut self,
        font_family: SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.font_family = font_family.clone();
        let installed_font_names = cx.text_system().all_font_names();
        let fonts = terminal_font_options(
            AppSettings::global(cx).custom_fonts.as_slice(),
            &installed_font_names,
        );
        self.font_select_state.update(cx, |state, cx| {
            state.set_items(fonts, window, cx);
            state.set_selected_value(&font_family, window, cx);
        });
        cx.notify();
    }

    pub fn set_scrollback_lines(
        &mut self,
        lines: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let lines = AppSettings::normalize_terminal_scrollback_lines(lines);
        self.scrollback_lines = lines;
        self.suppress_scrollback_lines_change = true;
        self.scrollback_lines_input_state.update(cx, |state, cx| {
            state.set_value(&lines.to_string(), window, cx);
        });
        self.suppress_scrollback_lines_change = false;
        cx.notify();
    }

    pub fn set_auto_copy(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.auto_copy = enabled;
        cx.notify();
    }

    pub fn set_autocomplete_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.autocomplete_enabled = enabled;
        cx.notify();
    }

    pub fn set_suggestion_popup_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.suggestion_popup_enabled = enabled;
        cx.notify();
    }

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.middle_click_paste = enabled;
        cx.notify();
    }

    pub fn set_right_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.right_click_paste = enabled;
        cx.notify();
    }

    pub fn set_paste_image_upload(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.paste_image_upload = enabled;
        cx.notify();
    }

    pub fn set_vim_scroll_to_arrow_keys(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.vim_scroll_to_arrow_keys = enabled;
        cx.notify();
    }

    pub fn set_sync_path(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.sync_path = enabled;
        cx.notify();
    }

    pub fn set_custom_highlights(
        &mut self,
        rules: Vec<TerminalHighlightRule>,
        cx: &mut Context<Self>,
    ) {
        self.custom_highlights = rules;
        cx.notify();
    }

    pub fn set_cursor_blink(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.cursor_blink = enabled;
        cx.notify();
    }

    pub fn set_selection_highlight(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.selection_highlight = enabled;
        cx.notify();
    }

    pub fn set_confirm_multiline_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.confirm_multiline_paste = enabled;
        cx.notify();
    }

    pub fn set_confirm_high_risk_command(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.confirm_high_risk_command = enabled;
        cx.notify();
    }

    fn emit_custom_highlights_changed(&self, cx: &mut Context<Self>) {
        cx.emit(SettingsPanelEvent::CustomHighlightsChanged(
            self.custom_highlights.clone(),
        ));
    }

    fn next_rule_id() -> String {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX 纪元")
            .as_nanos();
        format!("custom-highlight-{nanos}")
    }

    fn open_rule_editor(
        &mut self,
        index: Option<usize>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let existing = index.and_then(|idx| self.custom_highlights.get(idx).cloned());
        let initial_pattern = existing
            .as_ref()
            .map(|rule| rule.pattern.clone())
            .unwrap_or_default();
        let initial_foreground = existing
            .as_ref()
            .and_then(|rule| rule.foreground.clone())
            .unwrap_or_default();
        let initial_background = existing
            .as_ref()
            .and_then(|rule| rule.background.clone())
            .unwrap_or_default();
        let initial_priority = existing
            .as_ref()
            .map(|rule| rule.priority.to_string())
            .unwrap_or_else(|| "50".to_string());
        let initial_note = existing
            .as_ref()
            .map(|rule| rule.note.clone())
            .unwrap_or_default();
        let pattern_state = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("CustomHighlight.placeholders.regex"));
            if !initial_pattern.is_empty() {
                state = state.default_value(&initial_pattern);
            }
            state
        });
        let foreground_color = parse_optional_hex_color(Some(&initial_foreground));
        let background_color = parse_optional_hex_color(Some(&initial_background));
        let foreground_state = cx.new(|cx| {
            let state = ColorPickerState::new(window, cx);
            match foreground_color {
                Some(color) => state.default_value(color),
                None => state,
            }
        });
        let background_state = cx.new(|cx| {
            let state = ColorPickerState::new(window, cx);
            match background_color {
                Some(color) => state.default_value(color),
                None => state,
            }
        });
        let priority_state = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder("50")
                .pattern(regex::Regex::new(r"^\d{0,3}$").expect("priority regex 应可编译"));
            state = state.default_value(&initial_priority);
            state
        });
        let note_state = cx.new(|cx| {
            let mut state =
                InputState::new(window, cx).placeholder(t!("CustomHighlight.placeholders.note"));
            if !initial_note.is_empty() {
                state = state.default_value(&initial_note);
            }
            state
        });
        let enabled = existing.as_ref().map(|rule| rule.enabled).unwrap_or(true);
        let view = cx.entity().clone();
        let title = if existing.is_some() {
            t!("CustomHighlight.edit_rule_title").to_string()
        } else {
            t!("CustomHighlight.add_rule_title").to_string()
        };

        window.open_dialog(cx, move |dialog, window, _cx| {
            let view_ok = view.clone();
            let pattern_ok = pattern_state.clone();
            let foreground_ok = foreground_state.clone();
            let background_ok = background_state.clone();
            let foreground_clear = foreground_state.clone();
            let background_clear = background_state.clone();
            let priority_ok = priority_state.clone();
            let note_ok = note_state.clone();
            let existing_rule = existing.clone();

            dialog
                .title(title.clone())
                .child(
                    v_flex()
                        .gap_3()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(div().text_xs().child(t!("CustomHighlight.fields.regex")))
                                .child(Input::new(&pattern_state).small().w_full()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .child(t!("CustomHighlight.fields.foreground")),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(ColorPicker::new(&foreground_state).small())
                                                .child(
                                                    Button::new(
                                                        "custom-highlight-clear-foreground",
                                                    )
                                                    .label(t!("Common.none").to_string())
                                                    .ghost()
                                                    .xsmall()
                                                    .on_click(window.listener_for(
                                                        &foreground_clear,
                                                        move |state, _, window, cx| {
                                                            state.clear_value(window, cx);
                                                        },
                                                    )),
                                                ),
                                        ),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .child(t!("CustomHighlight.fields.background")),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(ColorPicker::new(&background_state).small())
                                                .child(
                                                    Button::new(
                                                        "custom-highlight-clear-background",
                                                    )
                                                    .label(t!("Common.none").to_string())
                                                    .ghost()
                                                    .xsmall()
                                                    .on_click(window.listener_for(
                                                        &background_clear,
                                                        move |state, _, window, cx| {
                                                            state.clear_value(window, cx);
                                                        },
                                                    )),
                                                ),
                                        ),
                                ),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    v_flex()
                                        .w(px(96.0))
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .child(t!("CustomHighlight.fields.priority")),
                                        )
                                        .child(Input::new(&priority_state).small()),
                                )
                                .child(
                                    v_flex()
                                        .flex_1()
                                        .gap_1()
                                        .child(
                                            div()
                                                .text_xs()
                                                .child(t!("CustomHighlight.fields.note")),
                                        )
                                        .child(Input::new(&note_state).small().w_full()),
                                ),
                        )
                        .into_any_element(),
                )
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Common.save").to_string())
                        .cancel_text(t!("Common.cancel").to_string()),
                )
                .on_ok(move |_event, window, cx| {
                    let pattern = pattern_ok.read(cx).value().trim().to_string();
                    let foreground = serialize_optional_color(foreground_ok.read(cx).value());
                    let background = serialize_optional_color(background_ok.read(cx).value());
                    let note = note_ok.read(cx).value().trim().to_string();
                    let priority_text = priority_ok.read(cx).value().trim().to_string();
                    let priority = if priority_text.is_empty() {
                        50
                    } else if let Ok(priority) = priority_text.parse::<u8>() {
                        priority
                    } else {
                        window.push_notification(
                            Notification::error(t!("CustomHighlight.errors.invalid_priority")),
                            cx,
                        );
                        return false;
                    };

                    let rule = TerminalHighlightRule {
                        id: existing_rule
                            .as_ref()
                            .map(|rule| rule.id.clone())
                            .unwrap_or_else(SettingsPanel::next_rule_id),
                        enabled,
                        pattern,
                        foreground,
                        background,
                        priority,
                        note,
                    };

                    if let Err(error) = validate_rule_for_save(&rule) {
                        window.push_notification(Notification::error(error), cx);
                        return false;
                    }

                    view_ok.update(cx, |this, cx| {
                        match index {
                            Some(idx) if idx < this.custom_highlights.len() => {
                                this.custom_highlights[idx] = rule;
                            }
                            _ => this.custom_highlights.push(rule),
                        }
                        this.emit_custom_highlights_changed(cx);
                        cx.notify();
                    });
                    true
                })
        });
    }

    /// 获取搜索值
    pub fn search_value(&self, cx: &App) -> String {
        self.search_input_state.read(cx).value().to_string()
    }

    /// 设置搜索值
    pub fn set_search_value(&self, value: &str, window: &mut Window, cx: &mut Context<Self>) {
        let value = value.to_string();
        self.search_input_state.update(cx, |state, cx| {
            state.set_value(&value, window, cx);
        });
    }

    /// 设置主题（用户点击主题时调用）
    fn set_theme(&mut self, theme: TerminalTheme, cx: &mut Context<Self>) {
        self.current_theme = theme.clone();
        cx.emit(SettingsPanelEvent::ThemeChanged(theme));
        cx.notify();
    }

    fn colors(&self) -> TerminalColors {
        self.current_theme.colors()
    }

    /// 渲染搜索区域
    fn render_search_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let muted_fg = colors.muted_foreground;

        v_flex().gap_3().p_3().child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted_fg)
                        .child(t!("Settings.search").to_uppercase()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(Input::new(&self.search_input_state).small().w_full())
                        .child(
                            Button::new("search-prev")
                                .icon(IconName::ChevronUp)
                                .ghost()
                                .small()
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsPanelEvent::SearchPrevious);
                                })),
                        )
                        .child(
                            Button::new("search-next")
                                .icon(IconName::ChevronDown)
                                .ghost()
                                .small()
                                .on_click(cx.listener(|_this, _, _window, cx| {
                                    cx.emit(SettingsPanelEvent::SearchNext);
                                })),
                        ),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(muted_fg)
                        .child(t!("Settings.search_shortcuts_hint")),
                ),
        )
    }

    /// 渲染主题项
    fn render_theme_item(&self, theme: TerminalTheme, cx: &mut Context<Self>) -> AnyElement {
        let is_current = self.current_theme.name == theme.name;
        let theme_for_click = theme.clone();
        let colors = self.colors();
        let accent = colors.accent;
        let accent_fg = colors.accent_foreground;
        let muted = colors.muted;
        let border = colors.border;
        let theme_display_name = theme.display_name();

        div()
            .id(SharedString::from(format!("theme-{}", theme.name)))
            .w_full()
            .flex()
            .items_center()
            .gap_3()
            .px_3()
            .py_2()
            .rounded_md()
            .cursor_pointer()
            .when(is_current, |style| style.bg(accent).text_color(accent_fg))
            .when(!is_current, |style| style.hover(|style| style.bg(muted)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _window, cx| {
                    this.set_theme(theme_for_click.clone(), cx);
                }),
            )
            .child(
                h_flex()
                    .gap_1()
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_md()
                            .bg(theme.background)
                            .border_1()
                            .border_color(border),
                    )
                    .child(
                        div()
                            .w(px(16.0))
                            .h(px(16.0))
                            .rounded_md()
                            .bg(theme.foreground)
                            .border_1()
                            .border_color(border),
                    ),
            )
            .child(div().flex_1().text_sm().child(theme_display_name))
            .when(is_current, |item| {
                item.child(Icon::new(IconName::Check).with_size(Size::Small))
            })
            .into_any_element()
    }

    /// 渲染字体设置区域
    fn render_font_section(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let fg = colors.foreground;
        let muted_fg = colors.muted_foreground;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            // 字体大小
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.font_size").to_uppercase()),
                    )
                    .child(
                        NumberInput::new(&self.font_size_input_state)
                            .small()
                            .suffix(div().text_xs().text_color(muted_fg).child("px")),
                    ),
            )
            // 字体选择
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.font_family").to_uppercase()),
                    )
                    .child(
                        Select::new(&self.font_select_state)
                            .small()
                            .text_color(fg)
                            .placeholder(t!("Settings.font_family_placeholder")),
                    ),
            )
    }

    fn render_scrollback_section(&self, _cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted_fg = colors.muted_foreground;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.scrollback_lines").to_uppercase()),
                    )
                    .child(
                        NumberInput::new(&self.scrollback_lines_input_state)
                            .small()
                            .suffix(
                                div()
                                    .text_xs()
                                    .text_color(muted_fg)
                                    .child(t!("Settings.lines")),
                            ),
                    )
                    .child(div().text_xs().text_color(muted_fg).child(t!(
                        "Settings.scrollback_lines_help",
                        min = AppSettings::MIN_TERMINAL_SCROLLBACK_LINES,
                        max = AppSettings::MAX_TERMINAL_SCROLLBACK_LINES
                    ))),
            )
    }

    /// 渲染光标设置区域
    fn render_cursor_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted_fg = colors.muted_foreground;
        let cursor_blink = self.cursor_blink;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.cursor").to_uppercase()),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.cursor_blink")))
                            .child(
                                Switch::new("cursor-blink-switch")
                                    .checked(cursor_blink)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.cursor_blink = *checked;
                                        cx.emit(SettingsPanelEvent::CursorBlinkChanged(*checked));
                                    })),
                            ),
                    ),
            )
    }

    fn render_safety_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted_fg = colors.muted_foreground;

        let confirm_multiline = self.confirm_multiline_paste;
        let confirm_high_risk = self.confirm_high_risk_command;
        let auto_copy = self.auto_copy;
        let autocomplete_enabled = self.autocomplete_enabled;
        let suggestion_popup_enabled = self.suggestion_popup_enabled;
        let middle_click_paste = self.middle_click_paste;
        let right_click_paste = self.right_click_paste;
        let paste_image_upload = self.paste_image_upload;
        let vim_scroll_to_arrow_keys = self.vim_scroll_to_arrow_keys;
        let selection_highlight = self.selection_highlight;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.safety").to_uppercase()),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .child(t!("Settings.confirm_multiline_paste")),
                            )
                            .child(
                                Switch::new("confirm-multiline-paste-switch")
                                    .checked(confirm_multiline)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.confirm_multiline_paste = *checked;
                                        cx.emit(SettingsPanelEvent::ConfirmMultilinePasteChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.right_click_paste")))
                            .child(
                                Switch::new("right-click-paste-switch")
                                    .checked(right_click_paste)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.right_click_paste = *checked;
                                        cx.emit(SettingsPanelEvent::RightClickPasteChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.paste_image_upload")))
                            .child(
                                Switch::new("paste-image-upload-switch")
                                    .checked(paste_image_upload)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.paste_image_upload = *checked;
                                        cx.emit(SettingsPanelEvent::PasteImageUploadChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .child(t!("Settings.confirm_high_risk_command")),
                            )
                            .child(
                                Switch::new("confirm-high-risk-command-switch")
                                    .checked(confirm_high_risk)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.confirm_high_risk_command = *checked;
                                        cx.emit(SettingsPanelEvent::ConfirmHighRiskCommandChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.auto_copy")))
                            .child(
                                Switch::new("auto-copy-switch")
                                    .checked(auto_copy)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.auto_copy = *checked;
                                        cx.emit(SettingsPanelEvent::AutoCopyChanged(*checked));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.autocomplete")))
                            .child(
                                Switch::new("terminal-autocomplete-switch")
                                    .checked(autocomplete_enabled)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.autocomplete_enabled = *checked;
                                        cx.emit(SettingsPanelEvent::AutocompleteChanged(*checked));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.suggestion_popup")))
                            .child(
                                Switch::new("terminal-suggestion-popup-switch")
                                    .checked(suggestion_popup_enabled)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.suggestion_popup_enabled = *checked;
                                        cx.emit(SettingsPanelEvent::SuggestionPopupChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(div().text_sm().child(t!("Settings.middle_click_paste")))
                            .child(
                                Switch::new("middle-click-paste-switch")
                                    .checked(middle_click_paste)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.middle_click_paste = *checked;
                                        cx.emit(SettingsPanelEvent::MiddleClickPasteChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .child(t!("Settings.vim_scroll_to_arrow_keys")),
                            )
                            .child(
                                Switch::new("vim-scroll-to-arrow-keys-switch")
                                    .checked(vim_scroll_to_arrow_keys)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.vim_scroll_to_arrow_keys = *checked;
                                        cx.emit(SettingsPanelEvent::VimScrollToArrowKeysChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div().text_sm().child(t!("Settings.selection_highlight")),
                            )
                            .child(
                                Switch::new("selection-highlight-switch")
                                    .checked(selection_highlight)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.selection_highlight = *checked;
                                        cx.emit(SettingsPanelEvent::SelectionHighlightChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    ),
            )
    }

    fn render_session_logging_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted_fg = colors.muted_foreground;
        let auto_session_logging = self.auto_session_logging;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.session_logging").to_uppercase()),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .child(t!("Settings.automatic_session_logging")),
                            )
                            .child(
                                Switch::new("automatic-session-logging-switch")
                                    .checked(auto_session_logging)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.auto_session_logging = *checked;
                                        cx.emit(SettingsPanelEvent::AutoSessionLoggingChanged(
                                            *checked,
                                        ));
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(t!("Settings.automatic_session_logging_help")),
                    ),
            )
    }

    /// 渲染文件管理器设置区域（仅 SSH 终端有文件管理器时显示）
    fn render_file_manager_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted_fg = colors.muted_foreground;
        let sync_path = self.sync_path;

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("Settings.file_manager_section").to_uppercase()),
                    )
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .child(
                                div()
                                    .text_sm()
                                    .child(t!("Settings.sync_path_with_terminal")),
                            )
                            .child(
                                Switch::new("sync-path-switch")
                                    .checked(sync_path)
                                    .small()
                                    .on_click(cx.listener(|this, checked: &bool, _window, cx| {
                                        this.sync_path = *checked;
                                        cx.emit(SettingsPanelEvent::SyncPathChanged(*checked));
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(t!("Settings.sync_path_help")),
                    ),
            )
    }

    fn render_highlight_rule_row(
        &self,
        index: usize,
        rule: &TerminalHighlightRule,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let colors = self.colors();
        let border = colors.border;
        let muted = colors.muted;
        let muted_fg = colors.muted_foreground;
        let enabled = rule.enabled;
        let pattern = rule.pattern.clone();
        let note = if rule.note.trim().is_empty() {
            t!("CustomHighlight.no_note").to_string()
        } else {
            rule.note.clone()
        };
        let priority = rule.priority;
        let fg_preview = rule
            .foreground
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());
        let bg_preview = rule
            .background
            .as_deref()
            .and_then(|value| try_parse_color(value).ok());

        v_flex()
            .gap_2()
            .p_2()
            .rounded_md()
            .border_1()
            .border_color(border)
            .bg(muted)
            .child(
                h_flex()
                    .w_full()
                    .gap_2()
                    .items_center()
                    .justify_between()
                    .child(
                        h_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .items_center()
                            .child(
                                Switch::new(SharedString::from(format!(
                                    "custom-highlight-enabled-{index}"
                                )))
                                .checked(enabled)
                                .small()
                                .on_click(cx.listener(
                                    move |this, checked: &bool, _window, cx| {
                                        if let Some(rule) = this.custom_highlights.get_mut(index) {
                                            rule.enabled = *checked;
                                            this.emit_custom_highlights_changed(cx);
                                            cx.notify();
                                        }
                                    },
                                )),
                            )
                            .child(
                                div()
                                    .flex_1()
                                    .min_w_0()
                                    .text_sm()
                                    .overflow_hidden()
                                    .text_ellipsis()
                                    .child(pattern.clone()),
                            ),
                    )
                    .child(
                        h_flex()
                            .flex_shrink_0()
                            .gap_1()
                            .ml_2()
                            .items_center()
                            .child(
                                Button::new(SharedString::from(format!(
                                    "custom-highlight-edit-{index}"
                                )))
                                .label(t!("CustomHighlight.actions.edit").to_string())
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(
                                    move |this, _, window, cx| {
                                        this.open_rule_editor(Some(index), window, cx);
                                    },
                                )),
                            )
                            .child(
                                Button::new(SharedString::from(format!(
                                    "custom-highlight-delete-{index}"
                                )))
                                .label(t!("CustomHighlight.actions.delete").to_string())
                                .ghost()
                                .xsmall()
                                .on_click(cx.listener(
                                    move |this, _, _window, cx| {
                                        if index < this.custom_highlights.len() {
                                            this.custom_highlights.remove(index);
                                            this.emit_custom_highlights_changed(cx);
                                            cx.notify();
                                        }
                                    },
                                )),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .flex_wrap()
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .text_xs()
                            .text_color(muted_fg)
                            .overflow_hidden()
                            .text_ellipsis()
                            .child(note),
                    )
                    .child(div().flex_shrink_0().text_xs().text_color(muted_fg).child(
                        t!("CustomHighlight.priority_value", priority = priority).to_string(),
                    ))
                    .when_some(fg_preview, |this, color| {
                        this.child(
                            div()
                                .w(px(14.0))
                                .h(px(14.0))
                                .rounded_sm()
                                .bg(color)
                                .border_1()
                                .border_color(border),
                        )
                    })
                    .when_some(bg_preview, |this, color| {
                        this.child(
                            div()
                                .w(px(14.0))
                                .h(px(14.0))
                                .rounded_sm()
                                .bg(color)
                                .border_1()
                                .border_color(border),
                        )
                    }),
            )
            .into_any_element()
    }

    fn render_custom_highlight_section(
        &self,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted = colors.muted;
        let muted_fg = colors.muted_foreground;
        let rows: Vec<AnyElement> = self
            .custom_highlights
            .iter()
            .enumerate()
            .map(|(index, rule)| self.render_highlight_rule_row(index, rule, cx))
            .collect();

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                h_flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_xs()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(muted_fg)
                            .child(t!("CustomHighlight.title").to_uppercase()),
                    )
                    .child(
                        Button::new("custom-highlight-add")
                            .label(t!("CustomHighlight.add_rule").to_string())
                            .ghost()
                            .xsmall()
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.open_rule_editor(None, window, cx);
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(muted_fg)
                    .child(t!("CustomHighlight.description")),
            )
            .child(
                div()
                    .rounded_md()
                    .bg(muted)
                    .p_2()
                    .children(if rows.is_empty() {
                        vec![
                            div()
                                .text_xs()
                                .text_color(muted_fg)
                                .child(t!("CustomHighlight.empty"))
                                .into_any_element(),
                        ]
                    } else {
                        rows
                    }),
            )
    }

    /// 渲染主题选择区域
    fn render_theme_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let colors = self.colors();
        let border = colors.border;
        let muted = colors.muted;
        let muted_fg = colors.muted_foreground;
        let theme_items = TerminalTheme::all(cx.theme())
            .into_iter()
            .map(|theme| self.render_theme_item(theme, cx))
            .collect::<Vec<_>>();

        v_flex()
            .gap_3()
            .p_3()
            .border_t_1()
            .border_color(border)
            .child(
                div()
                    .text_xs()
                    .font_weight(FontWeight::MEDIUM)
                    .text_color(muted_fg)
                    .child(t!("Settings.theme").to_uppercase()),
            )
            .child(
                div()
                    .id("theme-list")
                    .rounded_md()
                    .bg(muted)
                    .p_1()
                    .children(theme_items),
            )
    }
}

fn parse_optional_hex_color(value: Option<&str>) -> Option<Hsla> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .and_then(|value| Hsla::parse_hex(value).ok())
}

fn serialize_optional_color(value: Option<Hsla>) -> Option<String> {
    value.map(|color| color.to_hex())
}

fn validate_rule_for_save(rule: &TerminalHighlightRule) -> Result<(), String> {
    if rule.pattern.trim().is_empty() {
        return Err(t!("CustomHighlight.errors.pattern_required").to_string());
    }
    if rule.foreground.is_none() && rule.background.is_none() {
        return Err(t!("CustomHighlight.errors.color_required").to_string());
    }
    regex::Regex::new(&rule.pattern)
        .map_err(|error| t!("CustomHighlight.errors.invalid_regex", error = error).to_string())?;
    if let Some(foreground) = rule.foreground.as_deref() {
        try_parse_color(foreground).map_err(|error| {
            t!("CustomHighlight.errors.invalid_foreground", error = error).to_string()
        })?;
    }
    if let Some(background) = rule.background.as_deref() {
        try_parse_color(background).map_err(|error| {
            t!("CustomHighlight.errors.invalid_background", error = error).to_string()
        })?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{parse_optional_hex_color, serialize_optional_color, terminal_font_options};
    use gpui::Hsla;
    use one_core::settings::{CustomFont, default_grid_monospace_font_family};

    #[test]
    fn parse_optional_hex_color_returns_none_for_invalid_input() {
        assert!(parse_optional_hex_color(None).is_none());
        assert!(parse_optional_hex_color(Some("bad-color")).is_none());
    }

    #[test]
    fn parse_and_serialize_optional_hex_color_round_trip() {
        let color = parse_optional_hex_color(Some("#64C8FF")).expect("应解析有效 hex");
        let serialized = serialize_optional_color(Some(color)).expect("应序列化颜色");
        let reparsed = parse_optional_hex_color(Some(&serialized));

        assert!(serialized.starts_with('#'));
        assert!(matches!(serialized.len(), 7 | 9));
        assert!(reparsed.is_some());
    }

    #[test]
    fn serialize_optional_color_returns_none_when_empty() {
        assert_eq!(serialize_optional_color(None::<Hsla>), None);
    }

    #[test]
    fn terminal_font_options_include_only_custom_monospace_families() {
        let installed = vec!["Custom Mono".to_string()];
        let fonts = terminal_font_options(
            &[
                CustomFont {
                    path: "/tmp/NotoSansSC-VF.ttf".to_string(),
                    families: vec!["Noto Sans SC".to_string()],
                    monospace_families: Vec::new(),
                },
                CustomFont {
                    path: "/tmp/CustomMono.ttf".to_string(),
                    families: vec!["Custom Mono".to_string()],
                    monospace_families: vec!["Custom Mono".to_string()],
                },
            ],
            &installed,
        );
        let values = fonts
            .into_iter()
            .map(|font| font.value.to_string())
            .collect::<Vec<_>>();

        assert!(values.iter().any(|font| font == "Custom Mono"));
        assert!(!values.iter().any(|font| font == "Noto Sans SC"));
    }

    #[test]
    fn terminal_font_options_exclude_builtin_cjk_ui_fonts() {
        let fonts = terminal_font_options(&[], &[]);
        let values = fonts
            .into_iter()
            .map(|font| font.value.to_string())
            .collect::<Vec<_>>();

        assert!(!values.iter().any(|font| font == "Noto Sans Mono CJK SC"));
        assert!(!values.iter().any(|font| font == "Source Han Mono SC"));
        assert!(!values.iter().any(|font| font == "Noto Sans CJK SC"));
        assert!(!values.iter().any(|font| font == "Source Han Sans SC"));
        assert!(!values.iter().any(|font| font == "Microsoft YaHei"));
        assert!(!values.iter().any(|font| font == "PingFang SC"));
        assert!(!values.iter().any(|font| font == "SimSun"));
    }

    #[test]
    fn terminal_font_options_exclude_custom_fallback_only_fonts() {
        let installed = vec!["Custom Mono".to_string()];
        let fonts = terminal_font_options(
            &[CustomFont {
                path: "/tmp/CjkFonts.ttc".to_string(),
                families: vec!["PingFang SC".to_string()],
                monospace_families: vec![
                    "Noto Sans Mono CJK SC".to_string(),
                    "PingFang SC".to_string(),
                    "Custom Mono".to_string(),
                ],
            }],
            &installed,
        );
        let values = fonts
            .into_iter()
            .map(|font| font.value.to_string())
            .collect::<Vec<_>>();

        assert!(values.iter().any(|font| font == "Custom Mono"));
        assert!(!values.iter().any(|font| font == "Noto Sans Mono CJK SC"));
        assert!(!values.iter().any(|font| font == "PingFang SC"));
    }

    #[test]
    fn terminal_font_options_mark_missing_fonts_without_changing_values() {
        let default = default_grid_monospace_font_family();
        let installed = vec![default.to_string()];
        let fonts = terminal_font_options(
            &[CustomFont {
                path: "/tmp/CustomMono.ttf".to_string(),
                families: vec!["Custom Mono".to_string()],
                monospace_families: vec!["Custom Mono".to_string()],
            }],
            &installed,
        );

        assert!(
            fonts
                .iter()
                .any(|font| { font.value.as_ref() == default && font.label.as_ref() == default })
        );
        assert!(fonts.iter().any(|font| {
            font.value.as_ref() == "Fira Code" && font.label.as_ref() == "Fira Code (未安装)"
        }));
        assert!(fonts.iter().any(|font| {
            font.value.as_ref() == "Custom Mono" && font.label.as_ref() == "Custom Mono (未安装)"
        }));
    }

    #[test]
    fn terminal_theme_list_uses_single_panel_scroll_container() {
        let source = include_str!("settings_panel.rs");
        let section_start = source
            .find("fn render_theme_section")
            .expect("应存在终端主题区域");
        let section_end = source[section_start..]
            .find("\nfn parse_optional_hex_color")
            .map(|offset| section_start + offset)
            .expect("应找到终端主题区域结尾");
        let theme_section = &source[section_start..section_end];
        let panel_render = source
            .rfind("impl Render for SettingsPanel")
            .map(|offset| &source[offset..])
            .expect("应存在设置面板渲染实现");

        assert!(theme_section.contains("TerminalTheme::all(cx.theme())"));
        assert!(theme_section.contains(".id(\"theme-list\")"));
        assert!(theme_section.contains(".children(theme_items)"));
        assert!(!theme_section.contains(".max_h("));
        assert!(!theme_section.contains(".overflow_y_scroll"));
        assert!(panel_render.contains(".id(\"settings-panel-scroll\")"));
        assert!(panel_render.contains(".overflow_y_scrollbar()"));
    }

    #[test]
    fn settings_panel_source_avoids_hard_coded_user_facing_strings() {
        let source = include_str!("settings_panel.rs");
        let forbidden_snippets = [
            format!(".child({:?})", "Settings"),
            format!(".placeholder({:?})", "Search..."),
            format!(".child({:?})", "SEARCH"),
            format!(".child({:?})", "Press ⌘G for next, ⇧⌘G for previous"),
            format!(".child({:?})", "FONT SIZE"),
            format!(".child({:?})", "FONT FAMILY"),
            format!(".placeholder({:?})", "Select font..."),
            format!(".child({:?})", "THEME"),
        ];

        for snippet in forbidden_snippets {
            assert!(
                !source.contains(&snippet),
                "settings_panel.rs still contains hard-coded UI text snippet: {snippet}"
            );
        }
    }
}

impl EventEmitter<SettingsPanelEvent> for SettingsPanel {}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let has_file_manager = self.has_file_manager;
        let colors = self.colors();

        v_flex()
            .size_full()
            .bg(colors.background)
            .text_color(colors.foreground)
            .child(
                div().flex_1().w_full().min_h_0().overflow_hidden().child(
                    div()
                        .id("settings-panel-scroll")
                        .size_full()
                        .overflow_y_scrollbar()
                        .child(
                            v_flex()
                                .flex_shrink_0()
                                .pb_4()
                                .child(self.render_search_section(cx))
                                .child(self.render_font_section(cx))
                                .child(self.render_scrollback_section(cx))
                                .child(self.render_cursor_section(cx))
                                .child(self.render_safety_section(cx))
                                .child(self.render_session_logging_section(cx))
                                .when(has_file_manager, |el| {
                                    el.child(self.render_file_manager_section(cx))
                                })
                                .child(self.render_custom_highlight_section(window, cx))
                                .child(self.render_theme_section(cx)),
                        ),
                ),
            )
    }
}
