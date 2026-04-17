//! 终端设置面板
//!
//! 提供搜索、字体设置和主题切换功能

use gpui::prelude::FluentBuilder;
use gpui::FontWeight;
use gpui::{
    div, px, AnyElement, App, AppContext, Context, Entity, EventEmitter, FocusHandle, Focusable,
    Hsla, InteractiveElement, IntoElement, MouseButton, ParentElement, Render, SharedString,
    StatefulInteractiveElement, Styled, Subscription, Window,
};
use gpui_component::{
    button::{Button, ButtonVariants},
    color_picker::{ColorPicker, ColorPickerState},
    dialog::DialogButtonProps,
    h_flex,
    input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    notification::Notification,
    scroll::ScrollableElement,
    select::{Select, SelectEvent, SelectState},
    switch::Switch,
    try_parse_color, v_flex, ActiveTheme, Colorize, Icon, IconName, Sizable, Size, WindowExt,
};
use rust_i18n::t;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{
    theme::{TerminalTheme, MAX_FONT_SIZE, MIN_FONT_SIZE},
    TerminalHighlightRule,
};

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
    /// 光标闪烁变更
    CursorBlinkChanged(bool),
    /// 非 bracketed 模式下，多行粘贴确认开关
    ConfirmMultilinePasteChanged(bool),
    /// 高危命令确认开关
    ConfirmHighRiskCommandChanged(bool),
    /// 选中自动复制开关
    AutoCopyChanged(bool),
    /// 自动补全开关
    AutocompleteChanged(bool),
    /// 中键粘贴开关
    MiddleClickPasteChanged(bool),
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
    /// 字体选择状态
    font_select_state: Entity<SelectState<Vec<SharedString>>>,
    /// 当前主题
    current_theme: TerminalTheme,
    /// 字体大小输入变更抑制
    suppress_font_size_change: bool,
    /// 光标闪烁开关
    cursor_blink: bool,
    /// 非 bracketed 模式下，多行粘贴确认
    confirm_multiline_paste: bool,
    /// 高危命令确认
    confirm_high_risk_command: bool,
    /// 选中自动复制
    auto_copy: bool,
    /// 自动补全
    autocomplete_enabled: bool,
    /// 中键粘贴
    middle_click_paste: bool,
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
        has_file_manager: bool,
        auto_copy: bool,
        autocomplete_enabled: bool,
        middle_click_paste: bool,
        sync_path: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let search_input_state = cx.new(|cx| InputState::new(window, cx).placeholder("Search..."));

        // 字体大小输入框
        let font_size = f32::from(initial_theme.font_size);
        let font_size_input_state = cx.new(|cx| InputState::new(window, cx).placeholder("13"));
        font_size_input_state.update(cx, |state: &mut InputState, cx| {
            state.set_value(&format!("{:.0}", font_size), window, cx);
        });

        // 字体选择列表
        let fonts: Vec<SharedString> = TerminalTheme::available_monospace_fonts()
            .into_iter()
            .map(|f| SharedString::from(f))
            .collect();

        // 找到当前字体的索引
        let current_font = initial_theme.font_family.to_string();
        let selected_index = fonts
            .iter()
            .position(|f| f.as_ref() == current_font)
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
                InputEvent::PressEnter { secondary } => {
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
                        this.current_theme.font_size = px(clamped);
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
                    let current = f32::from(this.current_theme.font_size);
                    let new_size = match action {
                        StepAction::Increment => (current + 1.0).min(MAX_FONT_SIZE),
                        StepAction::Decrement => (current - 1.0).max(MIN_FONT_SIZE),
                    };
                    this.current_theme.font_size = px(new_size);
                    font_size_entity2.update(cx, |state: &mut InputState, cx| {
                        state.set_value(&format!("{:.0}", new_size), window, cx);
                    });
                    cx.emit(SettingsPanelEvent::FontSizeChanged(new_size));
                }
            },
        ));

        // 订阅字体选择事件
        subscriptions.push(cx.subscribe_in(
            &font_select_state,
            window,
            move |this, _state, event: &SelectEvent<Vec<SharedString>>, _window, cx| {
                if let SelectEvent::Confirm(Some(font)) = event {
                    this.current_theme.font_family = font.clone();
                    cx.emit(SettingsPanelEvent::FontFamilyChanged(font.to_string()));
                }
            },
        ));

        Self {
            search_input_state,
            font_size_input_state,
            font_select_state,
            current_theme: initial_theme.clone(),
            suppress_font_size_change: false,
            cursor_blink: false,
            confirm_multiline_paste: true,
            confirm_high_risk_command: true,
            auto_copy,
            autocomplete_enabled,
            middle_click_paste,
            sync_path,
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
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // 更新字体大小输入框
        let font_size = f32::from(theme.font_size);
        self.suppress_font_size_change = true;
        self.font_size_input_state.update(cx, |state, cx| {
            state.set_value(&format!("{:.0}", font_size), window, cx);
        });
        self.suppress_font_size_change = false;

        // 更新字体选择
        let font_family = theme.font_family.clone();
        self.font_select_state.update(cx, |state, cx| {
            state.set_selected_value(&font_family, window, cx);
        });

        self.current_theme = theme;
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

    pub fn set_middle_click_paste(&mut self, enabled: bool, cx: &mut Context<Self>) {
        self.middle_click_paste = enabled;
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
                                                            state.set_optional_value(
                                                                None, window, cx,
                                                            );
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
                                                            state.set_optional_value(
                                                                None, window, cx,
                                                            );
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
        // 更新当前主题
        self.current_theme = theme.clone();
        cx.emit(SettingsPanelEvent::ThemeChanged(theme));
        cx.notify();
    }

    /// 渲染头部
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_bg = cx.theme().muted;
        let fg = cx.theme().foreground;

        h_flex()
            .flex_shrink_0()
            .w_full()
            .h(px(40.0))
            .px_3()
            .items_center()
            .justify_between()
            .border_b_1()
            .border_color(border)
            .bg(muted_bg)
            .child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(
                        Icon::new(IconName::Settings)
                            .with_size(Size::Small)
                            .text_color(fg),
                    )
                    .child(
                        div()
                            .text_sm()
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(fg)
                            .child("Settings"),
                    ),
            )
            .child(
                Button::new("close-settings-panel")
                    .icon(IconName::Close)
                    .ghost()
                    .xsmall()
                    .on_click(cx.listener(|_this, _, _, cx| {
                        cx.emit(SettingsPanelEvent::Close);
                    })),
            )
    }

    /// 渲染搜索区域
    fn render_search_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let muted_fg = cx.theme().muted_foreground;

        v_flex().gap_3().p_3().child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(muted_fg)
                        .child("SEARCH"),
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
                        .child("Press ⌘G for next, ⇧⌘G for previous"),
                ),
        )
    }

    /// 渲染主题项
    fn render_theme_item(&self, theme: TerminalTheme, cx: &mut Context<Self>) -> AnyElement {
        let current_theme_name = self.current_theme.name;
        let is_current = current_theme_name == theme.name;
        let theme_for_click = theme.clone();
        let accent = cx.theme().accent;
        let accent_fg = cx.theme().accent_foreground;
        let muted = cx.theme().muted;
        let border = cx.theme().border;
        let theme_i18n_key = format!("Theme.{}", theme.name);
        let theme_display_name = t!(&theme_i18n_key).to_string();

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
            .when(!is_current, |style| style.hover(|s| s.bg(muted)))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, _, _window, cx| {
                    this.set_theme(theme_for_click.clone(), cx);
                }),
            )
            // 颜色预览
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
            // 主题名称
            .child(div().flex_1().text_sm().child(theme_display_name))
            .when(is_current, |item| {
                item.child(Icon::new(IconName::Check).with_size(Size::Small))
            })
            .into_any_element()
    }

    /// 渲染字体设置区域
    fn render_font_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let fg = cx.theme().foreground;
        let muted_fg = cx.theme().muted_foreground;

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
                            .child("FONT SIZE"),
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
                            .child("FONT FAMILY"),
                    )
                    .child(
                        Select::new(&self.font_select_state)
                            .small()
                            .text_color(fg)
                            .placeholder("Select font..."),
                    ),
            )
    }

    /// 渲染光标设置区域
    fn render_cursor_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
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
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;

        let confirm_multiline = self.confirm_multiline_paste;
        let confirm_high_risk = self.confirm_high_risk_command;
        let auto_copy = self.auto_copy;
        let autocomplete_enabled = self.autocomplete_enabled;
        let middle_click_paste = self.middle_click_paste;

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
                    ),
            )
    }

    /// 渲染文件管理器设置区域（仅 SSH 终端有文件管理器时显示）
    fn render_file_manager_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted_fg = cx.theme().muted_foreground;
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
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let muted_fg = cx.theme().muted_foreground;
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
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let muted_fg = cx.theme().muted_foreground;
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
                        vec![div()
                            .text_xs()
                            .text_color(muted_fg)
                            .child(t!("CustomHighlight.empty"))
                            .into_any_element()]
                    } else {
                        rows
                    }),
            )
    }

    /// 渲染主题选择区域
    fn render_theme_section(&mut self, cx: &mut Context<Self>) -> impl IntoElement {
        let border = cx.theme().border;
        let muted = cx.theme().muted;
        let muted_fg = cx.theme().muted_foreground;

        // 预先收集所有主题项
        let theme_items: Vec<AnyElement> = TerminalTheme::all()
            .into_iter()
            .map(|theme| self.render_theme_item(theme, cx))
            .collect();

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
                    .child("THEME"),
            )
            .child(
                div()
                    .id("theme-list-scroll")
                    .max_h(px(300.0))
                    .overflow_y_scrollbar()
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
    use super::{parse_optional_hex_color, serialize_optional_color};
    use gpui::Hsla;

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

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.render_header(cx))
            .child(
                div()
                    .id("settings-panel-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .child(
                        v_flex()
                            .flex_shrink_0()
                            .child(self.render_search_section(cx))
                            .child(self.render_font_section(cx))
                            .child(self.render_cursor_section(cx))
                            .child(self.render_safety_section(cx))
                            .when(has_file_manager, |el| {
                                el.child(self.render_file_manager_section(cx))
                            })
                            .child(self.render_custom_highlight_section(window, cx))
                            .child(self.render_theme_section(cx)),
                    ),
            )
    }
}
