//! 独立产品的设置分区：终端设置页。
//!
//! 独立产品没有主产品的设置系统，因此这里自建一页终端设置，直接读写
//! `one_core::settings::AppSettings`。终端视图通过 `observe_global_in::<AppSettings>`
//! 监听全局设置，保存后立即生效，不需要额外的应用钩子。
//!
//! 这一页是终端所有**全局默认值**的唯一落脚点：终端侧边栏只管当前会话，
//! 想改「以后新开的终端长什么样」必须回到这里。

use domain_api::SettingsSectionProvider;

mod local_terminal;
mod local_terminal_profiles;
use app_kit::integration::desktop_notification::{
    NOTIFICATION_SETTINGS_URL, NotificationAuthorization,
};
use gpui::{
    AnyElement, App, InteractiveElement, IntoElement, ParentElement, SharedString,
    StatefulInteractiveElement, Styled, Window, div, px,
};
use gpui_component::ActiveTheme;
use gpui_component::Sizable;
use gpui_component::WindowExt as _;
use gpui_component::button::{Button, ButtonVariants as _};
use gpui_component::notification::Notification;
use gpui_component::setting::{
    NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage,
};
use gpui_component::{h_flex, v_flex};
use one_core::settings::{
    AppSettings, TerminalDesktopNotificationDuration, TerminalDesktopNotificationMode,
    monospace_font_options,
};
use one_core::storage::StoredTerminalEncoding;
use rust_i18n::t;
use terminal_view::{
    MAX_FONT_SIZE, MIN_FONT_SIZE, TerminalTheme, available_monospace_fonts, current_settings,
    update_vim_scroll_to_arrow_keys,
};

/// 终端设置页在设置列表中的排序（排在数据领域之后）。
const TERMINAL_SETTINGS_ORDER: i32 = 20;

/// 终端设置分区的提供方。
pub(crate) struct ShellSettingsProvider;

impl SettingsSectionProvider for ShellSettingsProvider {
    fn pages(&self, cx: &mut App) -> Vec<(i32, SettingPage)> {
        vec![(TERMINAL_SETTINGS_ORDER, terminal_settings_page(cx))]
    }
}

/// 终端设置页：外观、回滚与日志、剪贴板、输入辅助、危险操作确认、界面与按键、
/// 系统通知（按平台能力展示），最后是本地终端默认 shell 与自定义终端。
fn terminal_settings_page(cx: &mut App) -> SettingPage {
    let profile = AppSettings::global(cx).local_terminal_profile.clone();
    let defaults = AppSettings::default();
    let mut groups = vec![
        terminal_appearance_group(cx, &defaults),
        terminal_scrolling_group(&defaults),
        terminal_clipboard_group(&defaults),
        terminal_input_group(&defaults),
        terminal_safety_group(&defaults),
        terminal_interface_group(&defaults),
    ];
    let notification_support = app_kit::integration::desktop_notification::support();
    if notification_support.system_notification {
        groups.push(terminal_notification_group(&defaults, notification_support));
    }
    groups.push(local_terminal::local_terminal_setting_group(&profile));

    SettingPage::new(t!("Settings.terminal.title"))
        .default_open(true)
        .resettable(true)
        .groups(groups)
}

/// 外观：字体、字号、配色、光标与行内的辅助标记。
fn terminal_appearance_group(cx: &mut App, defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.appearance_group"))
        .items(vec![
            font_family_item(cx, defaults),
            font_size_item(defaults),
            theme_item(defaults),
            bool_item(
                "cursor_blink",
                defaults.terminal_cursor_blink,
                |settings| settings.terminal_cursor_blink,
                |settings, value| settings.terminal_cursor_blink = value,
            ),
            bool_item(
                "selection_highlight",
                defaults.terminal_selection_highlight,
                |settings| settings.terminal_selection_highlight,
                |settings, value| settings.terminal_selection_highlight = value,
            ),
            bool_item(
                "show_line_numbers",
                defaults.terminal_show_line_numbers,
                |settings| settings.terminal_show_line_numbers,
                |settings, value| settings.terminal_show_line_numbers = value,
            ),
            bool_item(
                "show_line_timestamps",
                defaults.terminal_show_timestamps,
                |settings| settings.terminal_show_timestamps,
                |settings, value| settings.terminal_show_timestamps = value,
            ),
        ])
}

/// 回滚行数与自动会话日志。
fn terminal_scrolling_group(defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.scrolling_group"))
        .items(vec![
            scrollback_lines_item(defaults),
            bool_item(
                "auto_session_logging",
                defaults.terminal_auto_session_logging,
                |settings| settings.terminal_auto_session_logging,
                |settings, value| settings.terminal_auto_session_logging = value,
            ),
        ])
}

/// 剪贴板：选中复制、中键/右键粘贴、粘贴图片上传、路径与终端同步。
fn terminal_clipboard_group(defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.clipboard_group"))
        .items(vec![
            bool_item(
                "auto_copy",
                defaults.terminal_auto_copy,
                |settings| settings.terminal_auto_copy,
                |settings, value| settings.terminal_auto_copy = value,
            ),
            bool_item(
                "middle_click_paste",
                defaults.terminal_middle_click_paste,
                |settings| settings.terminal_middle_click_paste,
                |settings, value| settings.terminal_middle_click_paste = value,
            ),
            bool_item(
                "right_click_paste",
                defaults.terminal_right_click_paste,
                |settings| settings.terminal_right_click_paste,
                |settings, value| settings.terminal_right_click_paste = value,
            ),
            bool_item(
                "paste_image_upload",
                defaults.terminal_paste_image_upload,
                |settings| settings.terminal_paste_image_upload,
                |settings, value| settings.terminal_paste_image_upload = value,
            ),
            bool_item(
                "sync_path",
                defaults.terminal_sync_path_with_terminal,
                |settings| settings.terminal_sync_path_with_terminal,
                |settings, value| settings.terminal_sync_path_with_terminal = value,
            ),
        ])
}

/// 输入辅助：命令补全与候选弹层。
fn terminal_input_group(defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.input_group"))
        .items(vec![
            bool_item(
                "autocomplete",
                defaults.terminal_enable_autocomplete,
                |settings| settings.terminal_enable_autocomplete,
                |settings, value| settings.terminal_enable_autocomplete = value,
            ),
            bool_item(
                "suggestion_popup",
                defaults.terminal_show_suggestion_popup,
                |settings| settings.terminal_show_suggestion_popup,
                |settings, value| settings.terminal_show_suggestion_popup = value,
            ),
            suggestion_popup_opacity_item(defaults),
            default_terminal_encoding_item(defaults),
        ])
}

/// 危险操作的二次确认。
fn terminal_safety_group(defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.safety_group"))
        .items(vec![
            bool_item(
                "confirm_multiline_paste",
                defaults.terminal_confirm_multiline_paste,
                |settings| settings.terminal_confirm_multiline_paste,
                |settings, value| settings.terminal_confirm_multiline_paste = value,
            ),
            bool_item(
                "confirm_high_risk_command",
                defaults.terminal_confirm_high_risk_command,
                |settings| settings.terminal_confirm_high_risk_command,
                |settings, value| settings.terminal_confirm_high_risk_command = value,
            ),
        ])
}

/// 界面与按键：悬浮工具条、vim/TUI 滚轮映射。
fn terminal_interface_group(defaults: &AppSettings) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.terminal.interface_group"))
        .items(vec![
            bool_item(
                "show_pane_toolbar",
                defaults.terminal_show_pane_toolbar,
                |settings| settings.terminal_show_pane_toolbar,
                |settings, value| settings.terminal_show_pane_toolbar = value,
            ),
            vim_scroll_to_arrow_keys_item(),
        ])
}

/// 系统通知：触发时机、按平台能力展示的停留时长 / 提示音、测试通知与 Codex CLI 配置提示。
///
/// 各平台能力差异见 [`app_kit::integration::desktop_notification::NotificationSupport`]：
/// macOS 的通知停留时长由系统决定、Linux 的提示音由通知守护进程决定，对应的选项直接不展示，
/// 免得用户改了一个不会生效的设置。
fn terminal_notification_group(
    defaults: &AppSettings,
    support: app_kit::integration::desktop_notification::NotificationSupport,
) -> SettingGroup {
    let mut items = vec![desktop_notification_mode_item(defaults)];
    if support.duration {
        items.push(desktop_notification_duration_item(defaults));
    }
    if support.sound {
        items.push(desktop_notification_sound_item(defaults));
    }
    items.push(desktop_notification_test_item());
    items.push(desktop_notification_codex_tip_item());

    SettingGroup::new()
        .title(t!("Settings.terminal.desktop_notification_group"))
        .items(items)
}

/// 终端等宽字体下拉。
///
/// 选项 = 终端精选的内置等宽字体 + 用户导入的等宽家族 + 系统已装字体，
/// 合并规则与「SQL 编辑器字体」「表格预览字体」共用
/// `one_core::settings::monospace_font_options`；没装的字体在标签上标出来，
/// 否则用户会选到一个渲染时被静默改回默认值的家族。
fn font_family_item(cx: &App, defaults: &AppSettings) -> SettingItem {
    let installed_font_names = cx.text_system().all_font_names();
    let custom_font_families = AppSettings::global(cx)
        .custom_fonts
        .iter()
        .flat_map(|font| font.monospace_families.iter().cloned())
        .collect::<Vec<_>>();
    let missing_suffix = t!("Settings.terminal.font_family_missing").to_string();
    let options = monospace_font_options(
        &available_monospace_fonts(),
        &installed_font_names,
        &custom_font_families,
    )
    .into_iter()
    .map(|(family, installed)| {
        let label = if installed {
            SharedString::from(family.clone())
        } else {
            format!("{family}{missing_suffix}").into()
        };
        (SharedString::from(family), label)
    })
    .collect();
    SettingItem::new(
        t!("Settings.terminal.font_family"),
        SettingField::scrollable_dropdown(
            options,
            |cx: &App| SharedString::from(AppSettings::global(cx).terminal_font_family.clone()),
            |value: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_font_family = value.to_string();
                });
            },
        )
        .default_value(SharedString::from(defaults.terminal_font_family.clone())),
    )
    .description(t!("Settings.terminal.font_family_description").to_string())
}

/// 终端字号；范围与终端视图的缩放边界保持一致。
fn font_size_item(defaults: &AppSettings) -> SettingItem {
    let options = NumberFieldOptions {
        min: MIN_FONT_SIZE as f64,
        max: MAX_FONT_SIZE as f64,
        ..Default::default()
    };
    SettingItem::new(
        t!("Settings.terminal.font_size"),
        SettingField::number_input(
            options,
            |cx: &App| AppSettings::global(cx).terminal_font_size,
            |value: f64, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_font_size = value;
                });
            },
        )
        .default_value(defaults.terminal_font_size),
    )
    .description(t!("Settings.terminal.font_size_description").to_string())
}

/// 终端配色：渲染成一排主题卡片，与应用外观页的调色板同构。
///
/// 列表来自 `TerminalTheme::all`：「跟随应用」排第一，跟随当前应用主题动态取色，
/// 其余是终端自带的调色板。卡片背景/前景/圆点都用该主题自己的颜色，
/// 选中项用应用主色描边，点一下即写入并立刻生效。
fn theme_item(defaults: &AppSettings) -> SettingItem {
    let default_theme = defaults.terminal_theme.clone();
    let reset_theme = default_theme.clone();
    SettingItem::render(move |_options, _window, cx| render_terminal_theme_picker(cx))
        .keywords([t!("Settings.terminal.theme").to_string()])
        .on_reset(
            move |cx| AppSettings::global(cx).terminal_theme != default_theme,
            move |_window, cx| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_theme = reset_theme.clone();
                });
            },
        )
}

/// 主题卡片尺寸与应用外观页保持一致，避免两处配色选择器观感不同。
const THEME_CARD_WIDTH: f32 = 132.0;
const THEME_CARD_HEIGHT: f32 = 64.0;
const THEME_MARKER_SIZE: f32 = 12.0;
const THEME_PREVIEW_HEIGHT: f32 = 8.0;

fn render_terminal_theme_picker(cx: &mut App) -> AnyElement {
    let selected = AppSettings::global(cx).terminal_theme.clone();
    let cards = TerminalTheme::all(cx.theme())
        .into_iter()
        .map(|theme| render_terminal_theme_card(theme, &selected, cx));
    v_flex()
        .w_full()
        .gap_2()
        .child(div().child(t!("Settings.terminal.theme").to_string()))
        .child(h_flex().flex_wrap().gap_2().children(cards))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(t!("Settings.terminal.theme_description").to_string()),
        )
        .into_any_element()
}

fn render_terminal_theme_card(
    theme: TerminalTheme,
    selected: &str,
    cx: &App,
) -> impl IntoElement {
    let is_selected = theme.name == selected;
    let name = theme.name;
    let label = theme.display_name();
    let background = theme.background;
    let foreground = theme.foreground;
    let accent = theme.cursor;
    div()
        .id(SharedString::from(format!("terminal-theme-card-{name}")))
        .w(px(THEME_CARD_WIDTH))
        .h(px(THEME_CARD_HEIGHT))
        .p_2()
        .rounded(cx.theme().radius)
        .border(px(if is_selected { 2.0 } else { 1.0 }))
        .border_color(if is_selected {
            cx.theme().primary
        } else {
            cx.theme().border
        })
        .bg(background)
        .text_color(foreground)
        .cursor_pointer()
        .on_click(move |_, _, cx| {
            AppSettings::update_and_save(cx, |settings| {
                settings.terminal_theme = name.to_string();
            });
        })
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().size(px(THEME_MARKER_SIZE)).rounded_full().bg(accent))
                .child(div().text_xs().truncate().child(label)),
        )
        .child(
            div()
                .mt_2()
                .h(px(THEME_PREVIEW_HEIGHT))
                .rounded(cx.theme().radius)
                .bg(accent),
        )
}

/// 终端回滚行数；保存时由 `AppSettings` 归一到合法区间。
fn scrollback_lines_item(defaults: &AppSettings) -> SettingItem {
    let options = NumberFieldOptions {
        min: AppSettings::MIN_TERMINAL_SCROLLBACK_LINES as f64,
        max: AppSettings::MAX_TERMINAL_SCROLLBACK_LINES as f64,
        ..Default::default()
    };
    SettingItem::new(
        t!("Settings.terminal.scrollback_lines"),
        SettingField::number_input(
            options,
            |cx: &App| AppSettings::global(cx).terminal_scrollback_lines as f64,
            |value: f64, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_scrollback_lines = value as usize;
                });
            },
        )
        .default_value(defaults.terminal_scrollback_lines as f64),
    )
    .description(t!("Settings.terminal.scrollback_lines_description").to_string())
}

/// 候选弹层不透明度；与终端侧边栏里那条滑块共用同一区间。
/// 新建 SSH 连接 / 快速连接使用的默认终端编码（issue #345）。
///
/// 用 [`StoredTerminalEncoding::label`] 作为下拉值，避免把 serde 名字暴露给用户；
/// 回填时按 label 找回枚举。
fn default_terminal_encoding_item(defaults: &AppSettings) -> SettingItem {
    let options = StoredTerminalEncoding::all()
        .iter()
        .copied()
        .map(|encoding| {
            (
                SharedString::from(encoding.label()),
                SharedString::from(encoding.label()),
            )
        })
        .collect::<Vec<_>>();
    SettingItem::new(
        t!("Settings.terminal.default_encoding"),
        SettingField::dropdown(
            options,
            |cx: &App| {
                SharedString::from(AppSettings::global(cx).terminal_default_encoding.label())
            },
            |value: SharedString, cx: &mut App| {
                let encoding = StoredTerminalEncoding::all()
                    .iter()
                    .copied()
                    .find(|encoding| encoding.label() == value.as_ref())
                    .unwrap_or_default();
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_default_encoding = encoding;
                });
            },
        )
        .default_value(SharedString::from(
            defaults.terminal_default_encoding.label(),
        )),
    )
    .description(t!("Settings.terminal.default_encoding_description").to_string())
}

fn suggestion_popup_opacity_item(defaults: &AppSettings) -> SettingItem {
    let options = NumberFieldOptions {
        min: 0.1,
        max: 1.0,
        ..Default::default()
    };
    SettingItem::new(
        t!("Settings.terminal.suggestion_popup_opacity"),
        SettingField::number_input(
            options,
            |cx: &App| AppSettings::global(cx).terminal_suggestion_popup_opacity as f64,
            |value: f64, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_suggestion_popup_opacity = value.clamp(0.1, 1.0) as f32;
                });
            },
        )
        .default_value(defaults.terminal_suggestion_popup_opacity as f64),
    )
    .description(t!("Settings.terminal.suggestion_popup_opacity_description").to_string())
}

/// vim/TUI 里把鼠标滚轮映射成方向键。
///
/// 这一项存在本地终端设置文件里，读写都要走 `terminal_view` 的入口。
fn vim_scroll_to_arrow_keys_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.vim_scroll_to_arrow_keys"),
        SettingField::switch(
            |cx: &App| current_settings(cx).vim_scroll_to_arrow_keys,
            |value: bool, cx: &mut App| update_vim_scroll_to_arrow_keys(cx, value),
        )
        .default_value(true),
    )
    .description(t!("Settings.terminal.vim_scroll_to_arrow_keys_description").to_string())
}

/// 复用的布尔开关项：标题与描述靠语言键前缀拼出来，读写直连 `AppSettings`。
fn bool_item(
    key: &str,
    default_value: bool,
    read: fn(&AppSettings) -> bool,
    write: fn(&mut AppSettings, bool),
) -> SettingItem {
    let title_key = format!("Settings.terminal.{key}");
    let description_key = format!("Settings.terminal.{key}_description");
    SettingItem::new(
        t!(title_key.as_str()).to_string(),
        SettingField::switch(
            move |cx: &App| read(AppSettings::global(cx)),
            move |value: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| write(settings, value));
            },
        )
        .default_value(default_value),
    )
    .description(t!(description_key.as_str()).to_string())
}

/// 终端系统通知（OSC 9 / OSC 777）弹出时机。
const CODEX_NOTIFICATION_CONFIG_SNIPPET: &str =
    "[tui]\nnotifications = true\nnotification_method = \"osc9\"\nnotification_condition = \"always\"";

fn desktop_notification_mode_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.desktop_notification_mode"),
        SettingField::dropdown(
            vec![
                (
                    SharedString::from(
                        TerminalDesktopNotificationMode::UnfocusedOrInactiveTab.as_str(),
                    ),
                    t!("Settings.terminal.desktop_notification_unfocused_or_inactive_tab").into(),
                ),
                (
                    SharedString::from(
                        TerminalDesktopNotificationMode::UnfocusedWindowOnly.as_str(),
                    ),
                    t!("Settings.terminal.desktop_notification_unfocused_window_only").into(),
                ),
                (
                    SharedString::from(TerminalDesktopNotificationMode::Always.as_str()),
                    t!("Settings.terminal.desktop_notification_always").into(),
                ),
                (
                    SharedString::from(TerminalDesktopNotificationMode::Off.as_str()),
                    t!("Settings.terminal.desktop_notification_off").into(),
                ),
            ],
            |cx: &App| {
                SharedString::from(
                    AppSettings::global(cx)
                        .terminal_desktop_notification_mode
                        .as_str(),
                )
            },
            |value: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_desktop_notification_mode =
                        TerminalDesktopNotificationMode::from_str(value.as_ref());
                });
            },
        )
        .default_value(SharedString::from(
            defaults.terminal_desktop_notification_mode.as_str(),
        )),
    )
    .description(t!("Settings.terminal.desktop_notification_mode_description").to_string())
}

fn desktop_notification_duration_item(defaults: &AppSettings) -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.desktop_notification_duration"),
        SettingField::dropdown(
            vec![
                (
                    SharedString::from(TerminalDesktopNotificationDuration::Long.as_str()),
                    t!("Settings.terminal.desktop_notification_duration_long").into(),
                ),
                (
                    SharedString::from(TerminalDesktopNotificationDuration::Short.as_str()),
                    t!("Settings.terminal.desktop_notification_duration_short").into(),
                ),
            ],
            |cx: &App| {
                SharedString::from(
                    AppSettings::global(cx)
                        .terminal_desktop_notification_duration
                        .as_str(),
                )
            },
            |value: SharedString, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.terminal_desktop_notification_duration =
                        TerminalDesktopNotificationDuration::from_str(value.as_ref());
                });
            },
        )
        .default_value(SharedString::from(
            defaults.terminal_desktop_notification_duration.as_str(),
        )),
    )
    .description(t!("Settings.terminal.desktop_notification_duration_description").to_string())
}

fn desktop_notification_sound_item(defaults: &AppSettings) -> SettingItem {
    bool_item(
        "desktop_notification_sound",
        defaults.terminal_desktop_notification_sound,
        |settings| settings.terminal_desktop_notification_sound,
        |settings, value| settings.terminal_desktop_notification_sound = value,
    )
}

fn desktop_notification_test_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.terminal.desktop_notification_test"),
        SettingField::render(|_options, _window, _cx| {
            Button::new("terminal-notification-send-test")
                .label(t!("Settings.terminal.desktop_notification_test_button"))
                .small()
                .outline()
                .on_click(|_, window, cx| {
                    let settings = AppSettings::global(cx);
                    let authorization =
                        app_kit::integration::desktop_notification::send_test_notification(
                            settings.terminal_desktop_notification_duration,
                            settings.terminal_desktop_notification_sound,
                        );
                    if authorization == NotificationAuthorization::Denied {
                        push_notification_permission_denied(window, cx);
                    }
                })
        }),
    )
    .description(t!("Settings.terminal.desktop_notification_test_description").to_string())
}

/// 通知权限被拒绝时给出可见反馈。
///
/// 系统在授权被拒绝时依然让投递「成功」，只是以 `effectiveStyle: none` 丢掉通知，
/// 用户在界面上只会看到「点了没反应」，所以这里必须显式提示并给出直达入口。
fn push_notification_permission_denied(window: &mut Window, cx: &mut App) {
    window.push_notification(
        Notification::warning(
            t!("Settings.terminal.desktop_notification_permission_denied_body").to_string(),
        )
        .title(t!("Settings.terminal.desktop_notification_permission_denied_title").to_string())
        .action(|_, _, _| {
            Button::new("terminal-notification-open-settings")
                .label(
                    t!("Settings.terminal.desktop_notification_permission_denied_open_settings")
                        .to_string(),
                )
                .small()
                .on_click(|_, _, cx| cx.open_url(NOTIFICATION_SETTINGS_URL))
        }),
        cx,
    );
}

fn desktop_notification_codex_tip_item() -> SettingItem {
    SettingItem::render(|_options, _window, cx| {
        let theme = cx.theme();
        v_flex()
            .w_full()
            .gap_2()
            .p_3()
            .rounded_md()
            .border_1()
            .border_color(theme.border)
            .bg(theme.secondary)
            .child(
                h_flex()
                    .w_full()
                    .justify_between()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::MEDIUM)
                            .text_color(theme.foreground)
                            .child(t!("Settings.terminal.desktop_notification_codex_tip_title")),
                    )
                    .child(
                        Button::new("terminal-notification-copy-codex-config")
                            .label(t!("Settings.terminal.desktop_notification_codex_tip_copy"))
                            .tooltip(t!(
                                "Settings.terminal.desktop_notification_codex_tip_tooltip"
                            ))
                            .small()
                            .ghost()
                            .on_click(|_, _, cx| {
                                cx.write_to_clipboard(gpui::ClipboardItem::new_string(
                                    CODEX_NOTIFICATION_CONFIG_SNIPPET.to_string(),
                                ));
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(theme.muted_foreground)
                    .child(t!("Settings.terminal.desktop_notification_codex_tip_desc")),
            )
            .child(
                div()
                    .w_full()
                    .px_2p5()
                    .py_2()
                    .rounded_sm()
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.background)
                    .font_family(theme.mono_font_family.clone())
                    .text_xs()
                    .text_color(theme.foreground)
                    .child(CODEX_NOTIFICATION_CONFIG_SNIPPET),
            )
    })
    .keywords([
        "codex".to_string(),
        "osc9".to_string(),
        "config.toml".to_string(),
    ])
}

#[cfg(test)]
mod tests {
    /// 终端配色必须是卡片选择器，和应用外观页的调色板一致；
    /// 早先这里是下拉框，和外观页观感割裂。
    #[test]
    fn terminal_theme_renders_cards_instead_of_a_dropdown() {
        let source = include_str!("settings.rs");
        assert!(
            source.contains(concat!("render_terminal_theme", "_picker(cx)")),
            "终端配色必须走卡片选择器"
        );
        assert!(
            source.contains(concat!("terminal-theme-card", "-")),
            "卡片需要稳定的 id，便于点击与测试定位"
        );
        // 下拉写法会把 value 转成字符串再写回 terminal_theme；
        // 断言里拆成两段拼接，避免注释/断言文本命中自身。
        assert!(
            !source.contains(concat!("settings.terminal_theme", " = value.to_string()")),
            "终端配色不应再退回下拉框"
        );
    }

    /// 卡片列表要覆盖「跟随应用」+ 终端自带调色板，不能只列预设。
    #[test]
    fn terminal_theme_cards_follow_the_shared_theme_list() {
        let source = include_str!("settings.rs");
        let picker = source
            .split("fn render_terminal_theme_picker")
            .nth(1)
            .expect("render_terminal_theme_picker 必须存在")
            .split("fn render_terminal_theme_card")
            .next()
            .expect("picker 有结束标记");
        assert!(
            picker.contains("TerminalTheme::all(cx.theme())"),
            "主题列表必须来自 TerminalTheme::all，与应用主题联动"
        );
    }
}
