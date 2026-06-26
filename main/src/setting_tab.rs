use std::borrow::Cow;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::app_init::is_valid_system_hotkey;
use crate::auth::get_auth_service;
use crate::license::{get_license_service, offline_license_public_key};
use crate::settings::llm_providers_view::LlmProvidersView;
use crate::settings::mcp_settings::mcp_setting_group;
use crate::update;
use font_kit::{file_type::FileType, font::Font};
use gpui::http_client::{AsyncBody, Method, Request};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, AppContext, AsyncApp, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    FontWeight, InteractiveElement, IntoElement, KeyDownEvent, Keystroke, ParentElement,
    PathPromptOptions, Render, SharedString, Styled, WeakEntity, Window, div,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, IconName, IndexPath, Sizable, Size, Theme, ThemeMode, TitleBar,
    WindowExt,
    button::{Button, ButtonVariants as _},
    clipboard::Clipboard,
    group_box::GroupBoxVariant,
    h_flex,
    input::{Input, InputState},
    kbd::Kbd,
    scroll::ScrollableElement,
    select::{Select, SelectItem, SelectState},
    setting::{NumberFieldOptions, SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    switch::Switch,
    v_flex,
};
use one_core::gpui_tokio::Tokio;
use one_core::keybindings::action_id;
use one_core::llm::manager::GlobalProviderState;
use one_core::popup_window::{PopupWindowOptions, open_popup_window};
pub const DEFAULT_SYSTEM_HOTKEY_MACOS: &str = "cmd-alt-m";
pub const DEFAULT_SYSTEM_HOTKEY_OTHER: &str = "ctrl-alt-m";

pub use one_core::settings::{
    AppSettings, CustomFont, DatabaseOpenMode, GlobalCurrentUser, GlobalProxySettings,
    LargeTextCellEditorOpenMode, ProxyType,
};
use one_core::tab_container::{TabContent, TabContentEvent};
use one_core::utils::auto_save_config::AutoSaveConfig;
use reqwest_client::ReqwestClient;
use rust_i18n::t;
use terminal_view::TerminalTheme;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

fn builtin_app_font_options() -> Vec<(SharedString, SharedString)> {
    [
        "Arial",
        "Helvetica",
        "Times New Roman",
        "Courier New",
        "Noto Sans CJK SC",
        "Source Han Sans SC",
        "Microsoft YaHei",
        "PingFang SC",
        "SimSun",
    ]
    .into_iter()
    .map(|font| (font.into(), font.into()))
    .collect()
}

fn app_font_options(cx: &App) -> Vec<(SharedString, SharedString)> {
    merge_font_options_with_custom_fonts(
        builtin_app_font_options(),
        &AppSettings::global(cx).custom_fonts,
        FontFamilyKind::Any,
    )
}

fn builtin_monospace_font_options() -> Vec<(SharedString, SharedString)> {
    TerminalTheme::available_monospace_fonts()
        .into_iter()
        .map(|font| (font.into(), font.into()))
        .collect()
}

fn monospace_font_options(cx: &App) -> Vec<(SharedString, SharedString)> {
    merge_font_options_with_custom_fonts(
        builtin_monospace_font_options(),
        &AppSettings::global(cx).custom_fonts,
        FontFamilyKind::Monospace,
    )
}

#[derive(Clone, Copy)]
enum FontFamilyKind {
    Any,
    Monospace,
}

fn merge_font_options_with_custom_fonts(
    mut options: Vec<(SharedString, SharedString)>,
    custom_fonts: &[CustomFont],
    kind: FontFamilyKind,
) -> Vec<(SharedString, SharedString)> {
    let custom_families = custom_fonts.iter().flat_map(|font| match kind {
        FontFamilyKind::Any => font.families.iter(),
        FontFamilyKind::Monospace => font.monospace_families.iter(),
    });
    for family in custom_families {
        if family.trim().is_empty() || options.iter().any(|(value, _)| value.as_ref() == family) {
            continue;
        }
        options.push((family.clone().into(), family.clone().into()));
    }
    options
}

const FONT_FILE_EXTENSIONS: &[&str] = &["ttf", "otf", "ttc", "otc"];

fn is_supported_font_file(path: &Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .is_some_and(|extension| {
            FONT_FILE_EXTENSIONS
                .iter()
                .any(|candidate| extension.eq_ignore_ascii_case(candidate))
        })
}

fn read_font_file(path: &Path) -> Result<Vec<u8>, String> {
    if !is_supported_font_file(path) {
        return Err(t!("Settings.General.Font.unsupported_font_file").to_string());
    }
    std::fs::read(path).map_err(|err| err.to_string())
}

fn load_custom_font_path(path: &Path, cx: &mut App) -> Result<(), String> {
    let bytes = read_font_file(path)?;
    load_custom_font_bytes(bytes, cx)
}

fn load_custom_font_bytes(bytes: Vec<u8>, cx: &mut App) -> Result<(), String> {
    cx.text_system()
        .add_fonts(vec![Cow::Owned(bytes)])
        .map_err(|err| err.to_string())
}

fn load_custom_fonts(fonts: &[CustomFont], cx: &mut App) -> usize {
    fonts
        .iter()
        .filter(|font| load_custom_font_path(Path::new(&font.path), cx).is_ok())
        .count()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ParsedFontFamilies {
    families: Vec<String>,
    monospace_families: Vec<String>,
}

fn parse_font_families(bytes: &[u8]) -> ParsedFontFamilies {
    let font_data = Arc::new(bytes.to_vec());
    let mut parsed = ParsedFontFamilies::default();

    let indexes = match Font::analyze_bytes(Arc::clone(&font_data)) {
        Ok(FileType::Single) => 0..1,
        Ok(FileType::Collection(count)) => 0..count,
        Err(_) => return parsed,
    };

    for index in indexes {
        if let Ok(font) = Font::from_bytes(Arc::clone(&font_data), index) {
            let family = font.family_name();
            push_unique_font_family(&mut parsed.families, family.trim());
            if font.is_monospace() {
                push_unique_font_family(&mut parsed.monospace_families, family.trim());
            }
        }
    }

    parsed
}

fn push_unique_font_family(families: &mut Vec<String>, family: &str) {
    if !family.is_empty() && !families.iter().any(|existing| existing == family) {
        families.push(family.to_string());
    }
}

fn import_custom_fonts(paths: Vec<PathBuf>, cx: &mut App) -> String {
    let mut settings = AppSettings::current(cx);
    let mut loaded = 0usize;
    let mut monospace_count = 0usize;

    for path in paths {
        let Ok(bytes) = read_font_file(&path) else {
            continue;
        };
        let families = parse_font_families(&bytes);
        if load_custom_font_bytes(bytes, cx).is_err() {
            continue;
        }

        let path = path.to_string_lossy().to_string();
        monospace_count += families.monospace_families.len();
        upsert_custom_font(&mut settings.custom_fonts, path, families);
        loaded += 1;
    }

    if loaded > 0 {
        settings.save();
        cx.set_global(settings);
        t!(
            "Settings.General.Font.custom_fonts_import_success_with_monospace",
            count = loaded,
            monospace_count = monospace_count
        )
        .to_string()
    } else {
        t!("Settings.General.Font.custom_fonts_import_empty").to_string()
    }
}

fn upsert_custom_font(
    custom_fonts: &mut Vec<CustomFont>,
    path: String,
    families: ParsedFontFamilies,
) {
    if let Some(existing) = custom_fonts.iter_mut().find(|font| font.path == path) {
        if !families.families.is_empty() {
            existing.families = families.families;
            existing.monospace_families = families.monospace_families;
        }
    } else {
        custom_fonts.push(CustomFont {
            path,
            families: families.families,
            monospace_families: families.monospace_families,
        });
    }
}

pub fn init_settings(cx: &mut App) {
    let settings = AppSettings::load();
    // 初始化自动保存配置全局状态
    cx.set_global(AutoSaveConfig::new(
        settings.enable_sql_auto_save,
        settings.sql_auto_save_interval,
    ));
    settings.apply(cx);
    load_custom_fonts(&settings.custom_fonts, cx);
    init_tracing(&settings);
    let http_client = build_app_http_client(&settings.global_proxy).expect("HTTP 客户端初始化失败");
    cx.set_http_client(http_client);
    cx.set_global(settings);
}

fn init_tracing(settings: &AppSettings) {
    let env_filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));

    match crate::onetcli_app::configured_log_file_path(&settings.log_file_path) {
        Ok(log_file_path) => match crate::onetcli_app::log_file_appender(&log_file_path) {
            Ok(file_appender) => {
                let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
                Box::leak(Box::new(guard));
                tracing_subscriber::registry()
                    .with(tracing_subscriber::fmt::layer())
                    .with(tracing_subscriber::fmt::layer().with_writer(non_blocking))
                    .with(env_filter)
                    .init();
            }
            Err(err) => {
                tracing_subscriber::registry()
                    .with(tracing_subscriber::fmt::layer())
                    .with(env_filter)
                    .init();
                tracing::error!(path = %log_file_path.display(), error = %err, "日志文件初始化失败");
            }
        },
        Err(err) => {
            tracing_subscriber::registry()
                .with(tracing_subscriber::fmt::layer())
                .with(env_filter)
                .init();
            tracing::error!(error = %err, "默认日志目录初始化失败");
        }
    }
}

pub(crate) fn build_app_http_client(
    proxy: &GlobalProxySettings,
) -> Result<Arc<ReqwestClient>, String> {
    if proxy.enabled {
        let proxy_url = proxy.to_proxy_url()?;
        ReqwestClient::proxy_and_user_agent(proxy_url, "onetcli")
            .map(Arc::new)
            .map_err(|err| format!("HTTP 客户端初始化失败: {}", err))
    } else {
        ReqwestClient::user_agent("onetcli")
            .map(Arc::new)
            .map_err(|err| format!("HTTP 客户端初始化失败: {}", err))
    }
}

pub struct SettingsPanel {
    focus_handle: FocusHandle,
    llm_providers_view: Entity<LlmProvidersView>,
    size: Size,
    group_variant: GroupBoxVariant,
}

impl SettingsPanel {
    pub fn new(_window: &mut Window, cx: &mut Context<Self>) -> Self {
        let llm_providers_view = cx.new(|cx| LlmProvidersView::new(cx));
        Self {
            focus_handle: cx.focus_handle(),
            llm_providers_view,
            size: Size::default(),
            group_variant: GroupBoxVariant::Outline,
        }
    }

    fn setting_pages(&self, _window: &mut Window, cx: &App) -> Vec<SettingPage> {
        let llm_view = self.llm_providers_view.clone();
        let default_settings = AppSettings::default();
        let default_system_hotkey = AppSettings::default().current_system_hotkey().to_string();
        let app_font_options = app_font_options(cx);
        let font_options = monospace_font_options(cx);

        vec![
            SettingPage::new(t!("Settings.General.title"))
                .resettable(true)
                .default_open(true)
                .groups(vec![
                    SettingGroup::new()
                        .title(t!("Settings.General.Language.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Language.ui_language"),
                                SettingField::dropdown(
                                    vec![
                                        (
                                            "zh-CN".into(),
                                            t!("Settings.General.Language.zh_cn").into(),
                                        ),
                                        (
                                            "zh-HK".into(),
                                            t!("Settings.General.Language.zh_hk").into(),
                                        ),
                                        ("en".into(), t!("Settings.General.Language.en").into()),
                                    ],
                                    |cx: &App| {
                                        SharedString::from(AppSettings::global(cx).locale.clone())
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        let locale = val.to_string();
                                        gpui_component::set_locale(&locale);
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.locale = locale;
                                        });
                                    },
                                )
                                .default_value(SharedString::from(default_settings.locale)),
                            )
                            .description(
                                t!("Settings.General.Language.ui_language_desc").to_string(),
                            ),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Appearance.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Appearance.dark_mode"),
                                SettingField::switch(
                                    |cx: &App| cx.theme().mode.is_dark(),
                                    |val: bool, cx: &mut App| {
                                        let mode = if val {
                                            ThemeMode::Dark
                                        } else {
                                            ThemeMode::Light
                                        };
                                        Theme::global_mut(cx).mode = mode;
                                        Theme::change(mode, None, cx);

                                        let theme_mode = if val {
                                            "dark".to_string()
                                        } else {
                                            "light".to_string()
                                        };
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.theme_mode = theme_mode;
                                        });
                                    },
                                )
                                .default_value(false),
                            )
                            .description(
                                t!("Settings.General.Appearance.dark_mode_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Appearance.auto_switch_theme"),
                                SettingField::checkbox(
                                    |cx: &App| AppSettings::global(cx).auto_switch_theme,
                                    |val: bool, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.auto_switch_theme = val;
                                        });
                                    },
                                )
                                .default_value(default_settings.auto_switch_theme),
                            )
                            .description(
                                t!("Settings.General.Appearance.auto_switch_theme_desc")
                                    .to_string(),
                            ),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Font.group_title"))
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.font_family"),
                                SettingField::dropdown(
                                    app_font_options,
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).font_family.clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.font_family = val.to_string();
                                        });
                                    },
                                )
                                .default_value(SharedString::from(default_settings.font_family)),
                            )
                            .description(t!("Settings.General.Font.font_family_desc").to_string()),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.sql_editor_font_family"),
                                SettingField::dropdown(
                                    font_options.clone(),
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).sql_editor_font_family.clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.sql_editor_font_family = val.to_string();
                                        });
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings.sql_editor_font_family,
                                )),
                            )
                            .description(
                                t!("Settings.General.Font.sql_editor_font_family_desc").to_string(),
                            ),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.table_preview_font_family"),
                                SettingField::dropdown(
                                    font_options.clone(),
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx)
                                                .table_preview_font_family
                                                .clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.table_preview_font_family = val.to_string();
                                        });
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings.table_preview_font_family,
                                )),
                            )
                            .description(
                                t!("Settings.General.Font.table_preview_font_family_desc")
                                    .to_string(),
                            ),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.terminal_font_family"),
                                SettingField::dropdown(
                                    font_options.clone(),
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).terminal_font_family.clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.terminal_font_family = val.to_string();
                                        });
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings.terminal_font_family,
                                )),
                            )
                            .description(
                                t!("Settings.General.Font.terminal_font_family_desc").to_string(),
                            ),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.custom_fonts"),
                                SettingField::render(|options, _window, _cx| {
                                    Button::new("settings-import-custom-fonts")
                                        .icon(IconName::File)
                                        .label(t!("Settings.General.Font.import_custom_fonts"))
                                        .with_size(options.size)
                                        .on_click(|_, window, cx| {
                                            let target_window = window.window_handle();
                                            let future = cx.prompt_for_paths(PathPromptOptions {
                                                files: true,
                                                directories: false,
                                                multiple: true,
                                                prompt: Some(
                                                    t!("Settings.General.Font.select_font_files")
                                                        .to_string()
                                                        .into(),
                                                ),
                                            });

                                            window
                                                .spawn(cx, async move |cx| {
                                                    if let Ok(Ok(Some(paths))) = future.await {
                                                        let _ = cx.update(
                                                            |_view, cx: &mut App| {
                                                                let message =
                                                                    import_custom_fonts(paths, cx);
                                                                let _ = cx.update_window(
                                                                    target_window,
                                                                    |_, window, cx| {
                                                                        window.push_notification(
                                                                            message, cx,
                                                                        );
                                                                        window.refresh();
                                                                    },
                                                                );
                                                            },
                                                        );
                                                    }
                                                })
                                                .detach();
                                        })
                                }),
                            )
                            .description(
                                t!("Settings.General.Font.custom_fonts_desc").to_string(),
                            ),
                        )
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Font.font_size"),
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 8.0,
                                        max: 72.0,
                                        ..Default::default()
                                    },
                                    |cx: &App| AppSettings::global(cx).font_size,
                                    |val: f64, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.font_size = val;
                                        });
                                    },
                                )
                                .default_value(default_settings.font_size),
                            )
                            .description(t!("Settings.General.Font.font_size_desc").to_string()),
                        ),
                    SettingGroup::new()
                        .title(t!("Settings.General.Database.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Database.open_mode"),
                                SettingField::dropdown(
                                    vec![
                                        (
                                            "single".into(),
                                            t!("Settings.General.Database.open_mode_single").into(),
                                        ),
                                        (
                                            "workspace".into(),
                                            t!("Settings.General.Database.open_mode_workspace")
                                                .into(),
                                        ),
                                    ],
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).database_open_mode.as_str(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.database_open_mode =
                                                DatabaseOpenMode::from_str(&val);
                                        });
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings.database_open_mode.as_str(),
                                )),
                            )
                            .description(
                                t!("Settings.General.Database.open_mode_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.large_text_editor_open_mode"),
                                SettingField::dropdown(
                                    vec![
                                        (
                                            "sidebar_preview".into(),
                                            t!(
                                                "Settings.General.Database.large_text_editor_open_mode_sidebar"
                                            )
                                            .into(),
                                        ),
                                        (
                                            "dialog".into(),
                                            t!(
                                                "Settings.General.Database.large_text_editor_open_mode_dialog"
                                            )
                                            .into(),
                                        ),
                                    ],
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx)
                                                .large_text_cell_editor_open_mode
                                                .as_str(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        let mode =
                                            LargeTextCellEditorOpenMode::from_str(val.as_ref());
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.large_text_cell_editor_open_mode = mode;
                                        });
                                    },
                                )
                                .default_value(SharedString::from(
                                    default_settings
                                        .large_text_cell_editor_open_mode
                                        .as_str(),
                                )),
                            )
                            .description(
                                t!("Settings.General.Database.large_text_editor_open_mode_desc")
                                    .to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.auto_save"),
                                SettingField::switch(
                                    |cx: &App| AppSettings::global(cx).enable_sql_auto_save,
                                    |val: bool, cx: &mut App| {
                                        let interval =
                                            AppSettings::global(cx).sql_auto_save_interval;
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.enable_sql_auto_save = val;
                                        });
                                        AppSettings::update_auto_save_config(
                                            val,
                                            interval,
                                            cx,
                                        );
                                    },
                                )
                                .default_value(default_settings.enable_sql_auto_save),
                            )
                            .description(
                                t!("Settings.General.Database.auto_save_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.auto_save_interval"),
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 1.0,
                                        max: 60.0,
                                        step: 1.0,
                                    },
                                    |cx: &App| AppSettings::global(cx).sql_auto_save_interval,
                                    |val: f64, cx: &mut App| {
                                        let enabled = AppSettings::global(cx).enable_sql_auto_save;
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.sql_auto_save_interval = val;
                                        });
                                        AppSettings::update_auto_save_config(
                                            enabled,
                                            val,
                                            cx,
                                        );
                                    },
                                )
                                .default_value(default_settings.sql_auto_save_interval),
                            )
                            .description(
                                t!("Settings.General.Database.auto_save_interval_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.sql_query_max_rows"),
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 0.0,
                                        max: 1_000_000.0,
                                        step: 100.0,
                                    },
                                    |cx: &App| AppSettings::global(cx).sql_query_max_rows as f64,
                                    |val: f64, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.sql_query_max_rows = val as u32;
                                        });
                                    },
                                )
                                .default_value(default_settings.sql_query_max_rows as f64),
                            )
                            .description(
                                t!("Settings.General.Database.sql_query_max_rows_desc").to_string(),
                            ),
                            SettingItem::new(
                                t!("Settings.General.Database.table_row_height"),
                                SettingField::number_input(
                                    NumberFieldOptions {
                                        min: 24.0,
                                        max: 100.0,
                                        step: 2.0,
                                    },
                                    |cx: &App| AppSettings::global(cx).table_row_height as f64,
                                    |val: f64, cx: &mut App| {
                                        let height = val as u32;
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.table_row_height = height;
                                        });
                                    },
                                )
                                .default_value(default_settings.table_row_height as f64),
                            )
                            .description(
                                t!("Settings.General.Database.table_row_height_desc").to_string(),
                            ),
                        ]),
                    mcp_setting_group(&default_settings.mcp),
                    SettingGroup::new()
                        .title(t!("Settings.General.Log.group_title"))
                        .item(
                            SettingItem::new(
                                t!("Settings.General.Log.file_path"),
                                SettingField::input(
                                    |cx: &App| {
                                        SharedString::from(
                                            AppSettings::global(cx).log_file_path.clone(),
                                        )
                                    },
                                    |val: SharedString, cx: &mut App| {
                                        let log_file_path = val.trim().to_string();
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.log_file_path = log_file_path;
                                        });
                                    },
                                )
                                .default_value(SharedString::from("")),
                            )
                            .description(t!("Settings.General.Log.file_path_desc").to_string()),
                        ),
                    SettingGroup::new()
                        .title(t!("Settings.General.Update.group_title"))
                        .items(vec![
                            SettingItem::new(
                                t!("Settings.General.Update.auto_update"),
                                SettingField::switch(
                                    |cx: &App| AppSettings::global(cx).auto_update,
                                    |val: bool, cx: &mut App| {
                                        AppSettings::update_and_save(cx, |settings| {
                                            settings.auto_update = val;
                                        });
                                    },
                                )
                                .default_value(default_settings.auto_update),
                            )
                            .description(
                                t!("Settings.General.Update.auto_update_desc").to_string(),
                            ),
                            SettingItem::render(move |_options, _window, cx| {
                                render_manual_update_check_item(cx)
                            }),
                        ]),
                    SettingGroup::new()
                        .title(t!("Settings.General.Proxy.group_title"))
                        .item(SettingItem::render(move |_options, _window, cx| {
                            render_global_proxy_settings_item(cx)
                        })),
                ]),
            // 快捷键页面
            SettingPage::new(t!("Settings.Shortcuts.title")).group(
                SettingGroup::new().item(SettingItem::render(move |_options, window, cx| {
                    render_shortcuts_section(default_system_hotkey.clone(), window, cx)
                })),
            ),
            SettingPage::new(t!("LlmProviders.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, _window, _cx| {
                    llm_view.clone().into_any_element()
                }),
            )),
            // 账户设置页
            SettingPage::new(t!("Settings.Account.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, window, cx| render_account_section(window, cx)),
            )),
            // 关于页面
            SettingPage::new(t!("Settings.About.title")).group(SettingGroup::new().item(
                SettingItem::render(move |_options, _window, cx| render_about_section(cx)),
            )),
        ]
    }
}

impl Focusable for SettingsPanel {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl EventEmitter<TabContentEvent> for SettingsPanel {}

impl TabContent for SettingsPanel {
    fn content_key(&self) -> &'static str {
        "Settings"
    }

    fn title(&self, _cx: &App) -> SharedString {
        SharedString::from(t!("Common.settings"))
    }

    fn icon(&self, _cx: &App) -> Option<Icon> {
        Some(IconName::SettingColor.color())
    }

    fn closeable(&self, _cx: &App) -> bool {
        true
    }

    fn on_activate(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        if !cx.has_global::<AppSettings>() {
            init_settings(cx);
        }
    }
}

impl Render for SettingsPanel {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !cx.has_global::<AppSettings>() {
            init_settings(cx);
        }

        div().track_focus(&self.focus_handle).size_full().child(
            Settings::new("main-app-settings")
                .with_size(self.size)
                .with_group_variant(self.group_variant)
                .pages(self.setting_pages(window, cx)),
        )
    }
}

fn render_manual_update_check_item(cx: &mut App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .child(t!("Settings.General.Update.check_now").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Update.check_now_desc").to_string()),
                ),
        )
        .child(
            Button::new("settings-check-update")
                .icon(IconName::Refresh)
                .label(t!("Settings.General.Update.check_now"))
                .on_click(|_, window, cx| {
                    update::check_for_updates_manually(window, cx);
                }),
        )
        .into_any_element()
}

fn render_global_proxy_settings_item(cx: &mut App) -> gpui::AnyElement {
    h_flex()
        .w_full()
        .justify_between()
        .items_center()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .child(t!("Settings.General.Proxy.title").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.Proxy.description").to_string()),
                ),
        )
        .child(
            Button::new("settings-global-proxy")
                .icon(IconName::Globe)
                .label(t!("Settings.General.Proxy.open").to_string())
                .on_click(|_, _window, cx| {
                    show_global_proxy_settings_window(cx);
                }),
        )
        .into_any_element()
}

#[derive(Clone, PartialEq)]
struct ProxyTypeOption {
    value: ProxyType,
    label: SharedString,
}

impl SelectItem for ProxyTypeOption {
    type Value = ProxyType;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.value
    }
}

struct GlobalProxySettingsView {
    focus_handle: FocusHandle,
    enabled: bool,
    proxy_type_select: Entity<SelectState<Vec<ProxyTypeOption>>>,
    host_input: Entity<InputState>,
    port_input: Entity<InputState>,
    username_input: Entity<InputState>,
    password_input: Entity<InputState>,
    testing: bool,
    status_message: Option<(bool, String)>,
}

impl GlobalProxySettingsView {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let current = AppSettings::global(cx).global_proxy.clone();
        let proxy_types = vec![
            ProxyTypeOption {
                value: ProxyType::Http,
                label: "HTTP".into(),
            },
            ProxyTypeOption {
                value: ProxyType::Https,
                label: "HTTPS".into(),
            },
            ProxyTypeOption {
                value: ProxyType::Socks5,
                label: "SOCKS5".into(),
            },
        ];
        let selected_index = match current.proxy_type {
            ProxyType::Http => 0,
            ProxyType::Https => 1,
            ProxyType::Socks5 => 2,
        };
        let proxy_type_select = cx.new(|cx| {
            SelectState::new(
                proxy_types,
                Some(IndexPath::new(selected_index)),
                window,
                cx,
            )
        });
        let host_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("127.0.0.1");
            if !current.host.is_empty() {
                state.set_value(current.host.clone(), window, cx);
            }
            state
        });
        let port_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx).placeholder("1080");
            state.set_value(current.port.to_string(), window, cx);
            state
        });
        let username_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Settings.General.Proxy.username_placeholder"));
            if !current.username.is_empty() {
                state.set_value(current.username.clone(), window, cx);
            }
            state
        });
        let password_input = cx.new(|cx| {
            let mut state = InputState::new(window, cx)
                .placeholder(t!("Settings.General.Proxy.password_placeholder"));
            if !current.password.is_empty() {
                state.set_value(current.password.clone(), window, cx);
            }
            state
        });

        Self {
            focus_handle: cx.focus_handle(),
            enabled: current.enabled,
            proxy_type_select,
            host_input,
            port_input,
            username_input,
            password_input,
            testing: false,
            status_message: None,
        }
    }

    fn build_proxy_settings(&self, cx: &App) -> GlobalProxySettings {
        GlobalProxySettings {
            enabled: self.enabled,
            proxy_type: self
                .proxy_type_select
                .read(cx)
                .selected_value()
                .copied()
                .unwrap_or_default(),
            host: self
                .host_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            port: self
                .port_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .parse::<u16>()
                .unwrap_or(0),
            username: self
                .username_input
                .read(cx)
                .text()
                .to_string()
                .trim()
                .to_string(),
            password: self.password_input.read(cx).text().to_string(),
        }
    }

    fn render_form_row(
        &self,
        label: String,
        child: impl IntoElement,
        disabled: bool,
        cx: &App,
    ) -> gpui::AnyElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(
                div()
                    .w(gpui::px(120.0))
                    .text_sm()
                    .text_color(cx.theme().foreground)
                    .child(label),
            )
            .child(
                div()
                    .flex_1()
                    .child(child)
                    .when(disabled, |this| this.opacity(0.55)),
            )
            .into_any_element()
    }

    fn on_test(&mut self, cx: &mut Context<Self>) {
        if self.testing || !self.enabled {
            return;
        }

        let proxy_settings = self.build_proxy_settings(cx);
        let client = match build_app_http_client(&proxy_settings) {
            Ok(client) => client,
            Err(err) => {
                self.status_message = Some((false, err));
                cx.notify();
                return;
            }
        };

        self.testing = true;
        self.status_message = None;
        cx.notify();

        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let test_task = Tokio::spawn(cx, async move {
                let http_client: Arc<dyn gpui::http_client::HttpClient> = client;
                test_proxy_connectivity(http_client).await
            });

            let result = test_task
                .await
                .unwrap_or_else(|err| Err(format!("代理测试任务执行失败: {}", err)));

            let _ = this.update(cx, |view, cx| {
                view.testing = false;
                view.status_message = Some(match result {
                    Ok(()) => (true, t!("Settings.General.Proxy.test_success").to_string()),
                    Err(err) => (false, err),
                });
                cx.notify();
            });
        })
        .detach();
    }

    fn on_save(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.testing {
            return;
        }

        let proxy_settings = self.build_proxy_settings(cx);
        let new_client = match build_app_http_client(&proxy_settings) {
            Ok(client) => client,
            Err(err) => {
                self.status_message = Some((false, err));
                cx.notify();
                return;
            }
        };

        apply_global_proxy_settings(proxy_settings, new_client, cx);

        window.push_notification(t!("Settings.General.Proxy.save_success").to_string(), cx);
        window.remove_window();
    }

    fn on_cancel(&mut self, window: &mut Window, _cx: &mut Context<Self>) {
        if self.testing {
            return;
        }
        window.remove_window();
    }
}

impl Focusable for GlobalProxySettingsView {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GlobalProxySettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let disabled = !self.enabled;

        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .child(
                TitleBar::new().child(
                    div()
                        .flex()
                        .items_center()
                        .justify_center()
                        .flex_1()
                        .text_sm()
                        .font_weight(FontWeight::MEDIUM)
                        .child(t!("Settings.General.Proxy.dialog_title").to_string()),
                ),
            )
            .child(
                div().flex_1().min_h_0().overflow_y_scrollbar().p_4().child(
                    v_flex()
                        .gap_4()
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(t!("Settings.General.Proxy.dialog_desc").to_string()),
                        )
                        .child(
                            h_flex()
                                .justify_between()
                                .items_center()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .child(t!("Settings.General.Proxy.enable").to_string()),
                                )
                                .child(
                                    Switch::new("global-proxy-enabled")
                                        .checked(self.enabled)
                                        .on_click(cx.listener(|view, checked, _, cx| {
                                            view.enabled = *checked;
                                            view.status_message = None;
                                            cx.notify();
                                        })),
                                ),
                        )
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.type").to_string(),
                            Select::new(&self.proxy_type_select).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.host").to_string(),
                            Input::new(&self.host_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.port").to_string(),
                            Input::new(&self.port_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(self.render_form_row(
                            t!("Settings.General.Proxy.username").to_string(),
                            Input::new(&self.username_input).disabled(disabled),
                            disabled,
                            cx,
                        ))
                        .child(
                            self.render_form_row(
                                t!("Settings.General.Proxy.password").to_string(),
                                Input::new(&self.password_input)
                                    .mask_toggle()
                                    .disabled(disabled),
                                disabled,
                                cx,
                            ),
                        )
                        .when_some(self.status_message.clone(), |this, (success, message)| {
                            this.child(
                                div()
                                    .text_sm()
                                    .text_color(if success {
                                        cx.theme().muted_foreground
                                    } else {
                                        cx.theme().danger
                                    })
                                    .child(message),
                            )
                        }),
                ),
            )
            .child(
                h_flex()
                    .flex_shrink_0()
                    .justify_end()
                    .gap_2()
                    .p_4()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        Button::new("proxy-test")
                            .small()
                            .label(if self.testing {
                                t!("Settings.General.Proxy.testing").to_string()
                            } else {
                                t!("Settings.General.Proxy.test").to_string()
                            })
                            .disabled(self.testing || !self.enabled)
                            .on_click(cx.listener(|view, _, _, cx| {
                                view.on_test(cx);
                            })),
                    )
                    .child(
                        Button::new("proxy-cancel")
                            .small()
                            .label(t!("Common.cancel").to_string())
                            .disabled(self.testing)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.on_cancel(window, cx);
                            })),
                    )
                    .child(
                        Button::new("proxy-save")
                            .small()
                            .primary()
                            .label(t!("Common.save").to_string())
                            .disabled(self.testing)
                            .on_click(cx.listener(|view, _, window, cx| {
                                view.on_save(window, cx);
                            })),
                    ),
            )
    }
}

fn show_global_proxy_settings_window(cx: &mut App) {
    open_popup_window(
        PopupWindowOptions::new(t!("Settings.General.Proxy.dialog_title").to_string())
            .size(560.0, 460.0),
        move |window, cx| cx.new(|cx| GlobalProxySettingsView::new(window, cx)),
        cx,
    );
}

fn apply_global_proxy_settings(
    proxy_settings: GlobalProxySettings,
    http_client: Arc<ReqwestClient>,
    cx: &mut App,
) {
    AppSettings::update_and_save(cx, |settings| {
        settings.global_proxy = proxy_settings.clone();
    });
    apply_global_http_client(&proxy_settings, http_client, cx);
}

fn apply_global_http_client(
    proxy_settings: &GlobalProxySettings,
    http_client: Arc<ReqwestClient>,
    cx: &mut App,
) {
    let auth_service = get_auth_service(cx);
    let http_for_auth: Arc<dyn gpui::http_client::HttpClient> = http_client.clone();
    auth_service.replace_http_client(http_for_auth);

    if let Some(provider_state) = cx.try_global::<GlobalProviderState>() {
        provider_state.set_cloud_client(auth_service.cloud_client());
        if let Err(err) = provider_state.set_proxy_settings(proxy_settings) {
            tracing::error!(error = %err, "LLM 代理设置同步失败");
        }
        provider_state.manager().clear_cache();
    }

    cx.set_http_client(http_client);
}

async fn test_proxy_connectivity(
    http_client: Arc<dyn gpui::http_client::HttpClient>,
) -> Result<(), String> {
    let request = Request::builder()
        .method(Method::HEAD)
        .uri("https://www.gstatic.com/generate_204")
        .header("User-Agent", "onetcli-updater")
        .body(AsyncBody::empty())
        .map_err(|err| format!("构建代理测试请求失败: {}", err))?;

    let response = http_client
        .send(request)
        .await
        .map_err(|err| format!("代理连接测试失败: {}", err))?;

    if !response.status().is_success() {
        return Err(format!("代理测试返回异常状态码: {}", response.status()));
    }

    Ok(())
}

/// 渲染账户设置区域
fn render_account_section(_window: &mut Window, cx: &App) -> gpui::AnyElement {
    let user = GlobalCurrentUser::get_user(cx);

    if let Some(user) = user {
        // 已登录状态：显示用户信息和登出按钮
        let email: SharedString = user.email.clone().into();
        let display_name: SharedString = user
            .username
            .clone()
            .unwrap_or_else(|| {
                user.email
                    .split('@')
                    .next()
                    .unwrap_or(&user.email)
                    .to_string()
            })
            .into();

        v_flex()
            .gap_4()
            .p_4()
            // 用户信息区域
            .child(
                v_flex()
                    .gap_2()
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Settings.Account.username").to_string()),
                            )
                            .child(div().text_sm().child(display_name)),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .items_center()
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(t!("Settings.Account.email").to_string()),
                            )
                            .child(div().text_sm().child(email)),
                    ),
            )
            // 登出按钮
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("import-license-button")
                            .icon(IconName::File)
                            .label(t!("License.import_offline").to_string())
                            .on_click(move |_, window, cx| {
                                let public_key = match offline_license_public_key() {
                                    Ok(key) => key,
                                    Err(msg) => {
                                        window.push_notification(msg, cx);
                                        return;
                                    }
                                };
                                let license_service = get_license_service(cx);
                                let future = cx.prompt_for_paths(PathPromptOptions {
                                    files: true,
                                    directories: false,
                                    multiple: false,
                                    prompt: Some(t!("License.select_file").to_string().into()),
                                });

                                window
                                    .spawn(cx, async move |cx| {
                                        if let Ok(Ok(Some(paths))) = future.await {
                                            if let Some(path) = paths.into_iter().next() {
                                                let result = license_service
                                                    .import_offline_license_from_path(
                                                        &path,
                                                        &public_key,
                                                        None,
                                                    );
                                                let message = match result {
                                                    Ok(_) => {
                                                        t!("License.import_success").to_string()
                                                    }
                                                    Err(err) => t!(
                                                        "License.import_failed",
                                                        error = err.to_string()
                                                    )
                                                    .to_string(),
                                                };
                                                let _ = cx.update(|_view, cx: &mut App| {
                                                    if let Some(window_id) = cx.active_window() {
                                                        let _ = cx.update_window(
                                                            window_id,
                                                            |_, window, cx| {
                                                                window
                                                                    .push_notification(message, cx);
                                                            },
                                                        );
                                                    }
                                                });
                                            }
                                        }
                                    })
                                    .detach();
                            }),
                    )
                    .child(
                        Button::new("logout-button")
                            .icon(IconName::Close)
                            .label(t!("Auth.logout"))
                            .danger()
                            .on_click(move |_, _window, cx| {
                                // 清除 License
                                get_license_service(cx).clear();

                                // 执行登出
                                let auth = get_auth_service(cx);
                                cx.spawn(async move |cx: &mut AsyncApp| {
                                    auth.sign_out().await;
                                    cx.update(|cx| {
                                        GlobalCurrentUser::set_user(None, cx);
                                    });
                                })
                                .detach();
                            }),
                    ),
            )
            .into_any_element()
    } else {
        // 未登录状态：显示提示信息
        v_flex()
            .gap_2()
            .p_4()
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("Settings.Account.not_logged_in").to_string()),
            )
            .into_any_element()
    }
}

// ============================================================================
// 快捷键设置页
// ============================================================================

/// 快捷键条目
struct ShortcutEntry {
    /// macOS 快捷键字符串（Keystroke::parse 格式）
    keys_macos: &'static [&'static str],
    /// Windows/Linux 快捷键字符串（Keystroke::parse 格式）
    keys_other: &'static [&'static str],
    /// 国际化翻译 key
    label_key: &'static str,
    /// 绑定层 action id；为空表示只展示，不支持自定义
    action_id: Option<&'static str>,
    /// 是否为系统级热键
    system_hotkey: bool,
}

/// 快捷键分组
struct ShortcutGroup {
    title_key: &'static str,
    entries: &'static [ShortcutEntry],
}

const WINDOW_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["cmd-q"],
        keys_other: &["alt-f4"],
        label_key: "Settings.Shortcuts.quit_app",
        action_id: Some(action_id::APP_QUIT),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &[DEFAULT_SYSTEM_HOTKEY_MACOS],
        keys_other: &[DEFAULT_SYSTEM_HOTKEY_OTHER],
        label_key: "Settings.Shortcuts.minimize_window",
        action_id: None,
        system_hotkey: true,
    },
    ShortcutEntry {
        keys_macos: &["ctrl-cmd-f"],
        keys_other: &["alt-enter"],
        label_key: "Settings.Shortcuts.toggle_fullscreen",
        action_id: Some(action_id::WINDOW_TOGGLE_FULLSCREEN),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["ctrl-cmd-t"],
        keys_other: &["ctrl-alt-t"],
        label_key: "Settings.Shortcuts.toggle_always_on_top",
        action_id: Some(action_id::WINDOW_TOGGLE_ALWAYS_ON_TOP),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["shift-escape"],
        keys_other: &["shift-escape"],
        label_key: "Settings.Shortcuts.toggle_zoom",
        action_id: Some(action_id::WINDOW_TOGGLE_ZOOM),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["ctrl-w"],
        keys_other: &["ctrl-w"],
        label_key: "Settings.Shortcuts.close_panel",
        action_id: Some(action_id::WINDOW_CLOSE_PANEL),
        system_hotkey: false,
    },
];

const TAB_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["cmd-1"],
        keys_other: &["alt-1"],
        label_key: "Settings.Shortcuts.switch_tab_n",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["shift-cmd-t"],
        keys_other: &["alt-shift-t"],
        label_key: "Settings.Shortcuts.duplicate_tab",
        action_id: Some(action_id::APP_DUPLICATE_TAB),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-o"],
        keys_other: &["alt-o"],
        label_key: "Settings.Shortcuts.quick_open",
        action_id: Some(action_id::HOME_QUICK_OPEN),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-n"],
        keys_other: &["alt-n"],
        label_key: "Settings.Shortcuts.new_connection",
        action_id: Some(action_id::HOME_NEW_CONNECTION),
        system_hotkey: false,
    },
];

const CONNECTION_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["up", "left"],
        keys_other: &["up", "left"],
        label_key: "Settings.Shortcuts.connection_previous_type",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["down", "right"],
        keys_other: &["down", "right"],
        label_key: "Settings.Shortcuts.connection_next_type",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["enter"],
        keys_other: &["enter"],
        label_key: "Settings.Shortcuts.connection_open_type",
        action_id: None,
        system_hotkey: false,
    },
];

const TERMINAL_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["tab"],
        keys_other: &["tab"],
        label_key: "Settings.Shortcuts.terminal_send_tab",
        action_id: Some(action_id::TERMINAL_SEND_TAB),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["shift-tab"],
        keys_other: &["shift-tab"],
        label_key: "Settings.Shortcuts.terminal_send_shift_tab",
        action_id: Some(action_id::TERMINAL_SEND_SHIFT_TAB),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-c"],
        keys_other: &["ctrl-shift-c"],
        label_key: "Settings.Shortcuts.terminal_copy",
        action_id: Some(action_id::TERMINAL_COPY),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-v"],
        keys_other: &["ctrl-shift-v", "shift-insert"],
        label_key: "Settings.Shortcuts.terminal_paste",
        action_id: Some(action_id::TERMINAL_PASTE),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-a"],
        keys_other: &["ctrl-shift-a"],
        label_key: "Settings.Shortcuts.terminal_select_all",
        action_id: Some(action_id::TERMINAL_SELECT_ALL),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-k"],
        keys_other: &["ctrl-l"],
        label_key: "Settings.Shortcuts.terminal_clear_screen",
        action_id: Some(action_id::TERMINAL_CLEAR_SCREEN),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["escape"],
        keys_other: &["escape"],
        label_key: "Settings.Shortcuts.terminal_clear_selection",
        action_id: Some(action_id::TERMINAL_CLEAR_SELECTION),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-f"],
        keys_other: &["ctrl-shift-f"],
        label_key: "Settings.Shortcuts.terminal_search",
        action_id: Some(action_id::TERMINAL_SEARCH_FORWARD),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-g"],
        keys_other: &["ctrl-shift-g"],
        label_key: "Settings.Shortcuts.terminal_search_previous",
        action_id: Some(action_id::TERMINAL_SEARCH_BACKWARD),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-+", "cmd-="],
        keys_other: &["ctrl-+", "ctrl-="],
        label_key: "Settings.Shortcuts.terminal_zoom_in",
        action_id: Some(action_id::TERMINAL_INCREASE_FONT),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd--"],
        keys_other: &["ctrl--"],
        label_key: "Settings.Shortcuts.terminal_zoom_out",
        action_id: Some(action_id::TERMINAL_DECREASE_FONT),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-0"],
        keys_other: &["ctrl-0"],
        label_key: "Settings.Shortcuts.terminal_zoom_reset",
        action_id: Some(action_id::TERMINAL_RESET_FONT),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["f7"],
        keys_other: &["f7"],
        label_key: "Settings.Shortcuts.terminal_toggle_vi",
        action_id: Some(action_id::TERMINAL_TOGGLE_VI_MODE),
        system_hotkey: false,
    },
];

const DATABASE_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["cmd-f"],
        keys_other: &["ctrl-f"],
        label_key: "Settings.Shortcuts.database_focus_search",
        action_id: Some(action_id::DB_FOCUS_SEARCH),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-shift-enter"],
        keys_other: &["ctrl-shift-enter"],
        label_key: "Settings.Shortcuts.database_open_table_query",
        action_id: Some(action_id::DB_OPEN_TABLE_QUERY),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-enter", "ctrl-enter"],
        keys_other: &["cmd-enter", "ctrl-enter"],
        label_key: "Settings.Shortcuts.sql_run_query",
        action_id: Some(action_id::SQL_RUN_QUERY),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-shift-enter", "ctrl-shift-enter"],
        keys_other: &["cmd-shift-enter", "ctrl-shift-enter"],
        label_key: "Settings.Shortcuts.sql_run_all_query",
        action_id: Some(action_id::SQL_RUN_ALL_QUERY),
        system_hotkey: false,
    },
];

const TABLE_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["up", "down", "left", "right"],
        keys_other: &["up", "down", "left", "right"],
        label_key: "Settings.Shortcuts.table_move_selection",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["home", "end"],
        keys_other: &["home", "end"],
        label_key: "Settings.Shortcuts.table_first_last",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["pageup", "pagedown"],
        keys_other: &["pageup", "pagedown"],
        label_key: "Settings.Shortcuts.table_page",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["tab", "shift-tab"],
        keys_other: &["tab", "shift-tab"],
        label_key: "Settings.Shortcuts.table_next_previous_cell",
        action_id: None,
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-c"],
        keys_other: &["ctrl-c"],
        label_key: "Settings.Shortcuts.table_copy",
        action_id: Some(action_id::TABLE_COPY),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-v"],
        keys_other: &["ctrl-v"],
        label_key: "Settings.Shortcuts.table_paste",
        action_id: Some(action_id::TABLE_PASTE),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-a"],
        keys_other: &["ctrl-a"],
        label_key: "Settings.Shortcuts.table_select_all",
        action_id: Some(action_id::TABLE_SELECT_ALL),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["escape"],
        keys_other: &["escape"],
        label_key: "Settings.Shortcuts.table_cancel",
        action_id: Some(action_id::TABLE_CANCEL),
        system_hotkey: false,
    },
];

const REMOTE_EDITOR_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["cmd-f"],
        keys_other: &["ctrl-f"],
        label_key: "Settings.Shortcuts.remote_editor_search",
        action_id: Some(action_id::REMOTE_EDITOR_SEARCH),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-r"],
        keys_other: &["ctrl-r"],
        label_key: "Settings.Shortcuts.remote_editor_replace",
        action_id: Some(action_id::REMOTE_EDITOR_REPLACE),
        system_hotkey: false,
    },
];

const REDIS_CLI_SHORTCUTS: &[ShortcutEntry] = &[
    ShortcutEntry {
        keys_macos: &["ctrl-l"],
        keys_other: &["ctrl-l"],
        label_key: "Settings.Shortcuts.redis_clear_output",
        action_id: Some(action_id::REDIS_CLEAR_OUTPUT),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-c"],
        keys_other: &["ctrl-c"],
        label_key: "Settings.Shortcuts.redis_copy",
        action_id: Some(action_id::REDIS_COPY),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-v"],
        keys_other: &["ctrl-v"],
        label_key: "Settings.Shortcuts.redis_paste",
        action_id: Some(action_id::REDIS_PASTE),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["cmd-a"],
        keys_other: &["ctrl-a"],
        label_key: "Settings.Shortcuts.redis_select_all",
        action_id: Some(action_id::REDIS_SELECT_ALL),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["escape"],
        keys_other: &["escape"],
        label_key: "Settings.Shortcuts.redis_clear_selection",
        action_id: Some(action_id::REDIS_CLEAR_SELECTION),
        system_hotkey: false,
    },
    ShortcutEntry {
        keys_macos: &["tab"],
        keys_other: &["tab"],
        label_key: "Settings.Shortcuts.redis_complete_command",
        action_id: Some(action_id::REDIS_COMPLETE_COMMAND),
        system_hotkey: false,
    },
];

const SHORTCUT_GROUPS: &[ShortcutGroup] = &[
    ShortcutGroup {
        title_key: "Settings.Shortcuts.window",
        entries: WINDOW_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.tabs",
        entries: TAB_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.connection_dialog",
        entries: CONNECTION_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.terminal",
        entries: TERMINAL_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.database",
        entries: DATABASE_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.table",
        entries: TABLE_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.remote_editor",
        entries: REMOTE_EDITOR_SHORTCUTS,
    },
    ShortcutGroup {
        title_key: "Settings.Shortcuts.redis_cli",
        entries: REDIS_CLI_SHORTCUTS,
    },
];

#[derive(Clone)]
struct ShortcutCaptureState {
    active_editor_id: Option<&'static str>,
    invalid_capture: bool,
    focus_handle: FocusHandle,
}

fn shortcut_specs_for_entry(entry: &ShortcutEntry, cx: &App) -> Vec<String> {
    if entry.system_hotkey {
        return vec![AppSettings::global(cx).current_system_hotkey().to_string()];
    }
    if let Some(action_id) = entry.action_id {
        if let Some(shortcuts) = AppSettings::global(cx).custom_keybindings.get(action_id) {
            if !shortcuts.is_empty() {
                return shortcuts.clone();
            }
        }
    }

    let specs = if cfg!(target_os = "macos") {
        entry.keys_macos
    } else {
        entry.keys_other
    };
    specs.iter().map(|spec| spec.to_string()).collect()
}

fn render_shortcut_value(key_str: &str, cx: &App) -> gpui::AnyElement {
    match Keystroke::parse(key_str) {
        Ok(keystroke) => Kbd::new(keystroke).into_any_element(),
        Err(_) => div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(key_str.to_string())
            .into_any_element(),
    }
}

fn render_shortcut_values(key_specs: &[String], cx: &App) -> gpui::AnyElement {
    h_flex()
        .gap_1()
        .flex_wrap()
        .justify_end()
        .children(key_specs.iter().map(|key| render_shortcut_value(key, cx)))
        .into_any_element()
}

fn set_current_system_hotkey(spec: String, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        #[cfg(target_os = "macos")]
        {
            settings.system_hotkey_macos = spec;
        }
        #[cfg(not(target_os = "macos"))]
        {
            settings.system_hotkey_other = spec;
        }
    });
    crate::app_init::refresh_system_hotkey(cx);
}

fn set_custom_keybinding(action_id: &str, spec: String, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings
            .custom_keybindings
            .insert(action_id.to_string(), vec![spec]);
    });
    crate::onetcli_app::refresh_keybindings(cx);
}

fn reset_custom_keybinding(action_id: &str, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings.custom_keybindings.remove(action_id);
    });
    crate::onetcli_app::refresh_keybindings(cx);
}

fn shortcut_spec_from_keystroke(keystroke: &Keystroke) -> Option<String> {
    let key = keystroke.key.as_str();
    if matches!(key, "ctrl" | "control" | "alt" | "shift" | "cmd" | "win") {
        return None;
    }

    let mut tokens: Vec<&str> = Vec::with_capacity(5);
    if keystroke.modifiers.control {
        tokens.push("ctrl");
    }
    if keystroke.modifiers.alt {
        tokens.push("alt");
    }
    if keystroke.modifiers.shift {
        tokens.push("shift");
    }
    if keystroke.modifiers.platform {
        tokens.push("cmd");
    }
    tokens.push(key);
    Some(tokens.join("-"))
}

fn capture_shortcut(
    event: &KeyDownEvent,
    action_id: Option<&'static str>,
    system_hotkey: bool,
    state: &Entity<ShortcutCaptureState>,
    window: &mut Window,
    cx: &mut App,
) {
    window.prevent_default();
    cx.stop_propagation();

    if event.keystroke.key == "escape" {
        state.update(cx, |state, cx| {
            state.active_editor_id = None;
            state.invalid_capture = false;
            cx.notify();
        });
        return;
    }

    let Some(spec) = shortcut_spec_from_keystroke(&event.keystroke) else {
        return;
    };

    let is_valid = if system_hotkey {
        is_valid_system_hotkey(&spec)
    } else {
        Keystroke::parse(&spec).is_ok()
    };

    if !is_valid {
        state.update(cx, |state, cx| {
            state.invalid_capture = true;
            cx.notify();
        });
        return;
    }

    if system_hotkey {
        set_current_system_hotkey(spec, cx);
    } else if let Some(action_id) = action_id {
        set_custom_keybinding(action_id, spec, cx);
    }
    state.update(cx, |state, cx| {
        state.active_editor_id = None;
        state.invalid_capture = false;
        cx.notify();
    });
}

fn render_shortcut_editor(
    entry: &'static ShortcutEntry,
    key_specs: &[String],
    default_system_hotkey: String,
    state: Entity<ShortcutCaptureState>,
    cx: &mut App,
) -> gpui::AnyElement {
    let editor_id = entry.label_key;
    let editing = state.read(cx).active_editor_id == Some(editor_id);
    let invalid_capture = state.read(cx).invalid_capture;
    let focus_handle = state.read(cx).focus_handle.clone();

    if editing {
        return h_flex()
            .gap_2()
            .items_center()
            .track_focus(&focus_handle)
            .on_key_down({
                let state = state.clone();
                move |event, window, cx| {
                    capture_shortcut(
                        event,
                        entry.action_id,
                        entry.system_hotkey,
                        &state,
                        window,
                        cx,
                    )
                }
            })
            .child(
                div()
                    .px_2()
                    .py_1()
                    .rounded_md()
                    .border_1()
                    .border_color(if invalid_capture {
                        cx.theme().danger
                    } else {
                        cx.theme().border
                    })
                    .text_sm()
                    .text_color(if invalid_capture {
                        cx.theme().danger
                    } else {
                        cx.theme().muted_foreground
                    })
                    .child(if invalid_capture {
                        t!("Settings.Shortcuts.invalid_hotkey").to_string()
                    } else {
                        t!("Settings.Shortcuts.press_shortcut").to_string()
                    }),
            )
            .child(
                Button::new("system-hotkey-cancel")
                    .label(t!("Common.cancel").to_string())
                    .ghost()
                    .xsmall()
                    .on_click({
                        let state = state.clone();
                        move |_, _, cx| {
                            state.update(cx, |state, cx| {
                                state.active_editor_id = None;
                                state.invalid_capture = false;
                                cx.notify();
                            });
                        }
                    }),
            )
            .into_any_element();
    }

    h_flex()
        .gap_2()
        .items_center()
        .child(render_shortcut_values(key_specs, cx))
        .child(
            h_flex()
                .gap_1()
                .invisible()
                .group_hover("shortcut-row", |this| this.visible())
                .child(
                    Button::new("system-hotkey-edit")
                        .icon(IconName::Edit)
                        .ghost()
                        .xsmall()
                        .tooltip(t!("Common.edit").to_string())
                        .on_click({
                            let state = state.clone();
                            let focus_handle = focus_handle.clone();
                            move |_, window, cx| {
                                state.update(cx, |state, cx| {
                                    state.active_editor_id = Some(editor_id);
                                    state.invalid_capture = false;
                                    cx.notify();
                                });
                                focus_handle.focus(window, cx);
                            }
                        }),
                )
                .child(
                    Button::new("system-hotkey-reset")
                        .icon(IconName::Refresh)
                        .ghost()
                        .xsmall()
                        .tooltip(t!("Settings.Shortcuts.reset").to_string())
                        .on_click(move |_, _, cx| {
                            if entry.system_hotkey {
                                set_current_system_hotkey(default_system_hotkey.clone(), cx);
                            } else if let Some(action_id) = entry.action_id {
                                reset_custom_keybinding(action_id, cx);
                            }
                        }),
                ),
        )
        .into_any_element()
}

/// 渲染快捷键说明页面
fn render_shortcuts_section(
    default_system_hotkey: String,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let capture_state =
        window.use_keyed_state("system-hotkey-capture", cx, |_, cx| ShortcutCaptureState {
            active_editor_id: None,
            invalid_capture: false,
            focus_handle: cx.focus_handle(),
        });
    let mut container = v_flex().gap_4().p_4().child(
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(t!("Settings.Shortcuts.system_hotkey_desc").to_string()),
    );

    for group in SHORTCUT_GROUPS {
        let mut group_container = v_flex().gap_2();

        // 分组标题
        group_container = group_container.child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .child(t!(group.title_key).to_string()),
        );

        // 快捷键列表
        let mut list = v_flex().gap_1().pl_2();

        for entry in group.entries {
            let key_specs = shortcut_specs_for_entry(entry, cx);
            let value = if entry.system_hotkey || entry.action_id.is_some() {
                render_shortcut_editor(
                    entry,
                    &key_specs,
                    default_system_hotkey.clone(),
                    capture_state.clone(),
                    cx,
                )
            } else {
                render_shortcut_values(&key_specs, cx)
            };

            list = list.child(
                h_flex()
                    .group("shortcut-row")
                    .items_center()
                    .justify_between()
                    .gap_3()
                    .py_1()
                    .child(
                        div()
                            .flex_1()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(t!(entry.label_key).to_string()),
                    )
                    .child(value),
            );
        }

        group_container = group_container.child(list);
        container = container.child(group_container);
    }

    container.into_any_element()
}

#[cfg(test)]
mod tests {
    use gpui::http_client::HttpClient;

    use super::{
        AppSettings, CustomFont, FontFamilyKind, GlobalProxySettings, ProxyType,
        build_app_http_client, builtin_monospace_font_options, is_supported_font_file,
        merge_font_options_with_custom_fonts, parse_font_families,
    };
    use std::path::Path;

    #[test]
    fn global_proxy_settings_build_proxy_url_without_auth() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Socks5,
            host: "127.0.0.1".to_string(),
            port: 7890,
            username: String::new(),
            password: String::new(),
        };

        let proxy_url = settings
            .to_proxy_url()
            .expect("代理 URL 应构建成功")
            .expect("启用代理时应返回 URL");

        assert_eq!(proxy_url.as_str(), "socks5://127.0.0.1:7890");
    }

    #[test]
    fn global_proxy_settings_build_proxy_url_with_auth() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Http,
            host: "proxy.example.com".to_string(),
            port: 8080,
            username: "demo-user".to_string(),
            password: "demo-pass".to_string(),
        };

        let proxy_url = settings
            .to_proxy_url()
            .expect("代理 URL 应构建成功")
            .expect("启用代理时应返回 URL");

        assert_eq!(
            proxy_url.as_str(),
            "http://demo-user:demo-pass@proxy.example.com:8080/"
        );
    }

    #[test]
    fn app_settings_defaults_custom_keybindings_for_legacy_config() {
        let settings: AppSettings = serde_json::from_str("{}").unwrap();

        assert!(settings.custom_keybindings.is_empty());
    }

    #[test]
    fn supported_font_file_detection_accepts_common_font_extensions() {
        assert!(is_supported_font_file(Path::new("NotoSansCJK-Regular.ttc")));
        assert!(is_supported_font_file(Path::new("JetBrainsMono.ttf")));
        assert!(is_supported_font_file(Path::new("SourceHanSans.otf")));
        assert!(!is_supported_font_file(Path::new("font.zip")));
    }

    #[test]
    fn monospace_font_options_include_only_cjk_monospace_fonts() {
        let values = builtin_monospace_font_options()
            .into_iter()
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>();

        assert!(values.iter().any(|value| value == "Noto Sans Mono CJK SC"));
        assert!(values.iter().any(|value| value == "Source Han Mono SC"));
        assert!(!values.iter().any(|value| value == "Noto Sans CJK SC"));
        assert!(!values.iter().any(|value| value == "Source Han Sans SC"));
        assert!(!values.iter().any(|value| value == "Microsoft YaHei"));
        assert!(!values.iter().any(|value| value == "PingFang SC"));
        assert!(!values.iter().any(|value| value == "SimSun"));
    }

    #[test]
    fn imported_font_families_are_added_to_font_options() {
        let options = merge_font_options_with_custom_fonts(
            builtin_monospace_font_options(),
            &[CustomFont {
                path: "/tmp/NotoSansCJK-Regular.ttc".to_string(),
                families: vec![
                    "Noto Sans Mono CJK SC".to_string(),
                    "Custom Mono SC".to_string(),
                ],
                monospace_families: vec![
                    "Noto Sans Mono CJK SC".to_string(),
                    "Custom Mono SC".to_string(),
                ],
            }],
            FontFamilyKind::Monospace,
        );

        let values = options
            .into_iter()
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>();

        assert!(values.iter().any(|value| value == "Custom Mono SC"));
        assert_eq!(
            1,
            values
                .iter()
                .filter(|value| value.as_str() == "Noto Sans Mono CJK SC")
                .count()
        );
    }

    #[test]
    fn monospace_font_options_ignore_non_monospace_custom_fonts() {
        let options = merge_font_options_with_custom_fonts(
            builtin_monospace_font_options(),
            &[CustomFont {
                path: "/tmp/NotoSansSC-VF.ttf".to_string(),
                families: vec!["Noto Sans SC".to_string()],
                monospace_families: Vec::new(),
            }],
            FontFamilyKind::Monospace,
        );

        let values = options
            .into_iter()
            .map(|(value, _)| value.to_string())
            .collect::<Vec<_>>();

        assert!(!values.iter().any(|value| value == "Noto Sans SC"));
    }

    #[test]
    fn parse_font_families_ignores_invalid_font_bytes() {
        assert_eq!(parse_font_families(b"not a font"), Default::default());
    }

    #[test]
    fn disabled_global_proxy_settings_return_none() {
        let settings = GlobalProxySettings {
            enabled: false,
            ..GlobalProxySettings::default()
        };

        let proxy_url = settings.to_proxy_url().expect("禁用代理时不应返回错误");

        assert!(proxy_url.is_none());
    }

    #[test]
    fn build_app_http_client_uses_no_app_proxy_when_proxy_disabled() {
        let settings = GlobalProxySettings {
            enabled: false,
            host: "127.0.0.1".to_string(),
            port: 7890,
            ..GlobalProxySettings::default()
        };

        let client = build_app_http_client(&settings).expect("禁用代理时 HTTP client 应创建成功");

        assert_eq!(client.proxy(), None);
        assert_eq!(
            client.user_agent().and_then(|value| value.to_str().ok()),
            Some("onetcli")
        );
    }

    #[test]
    fn build_app_http_client_uses_current_app_proxy_when_proxy_enabled() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Http,
            host: "127.0.0.1".to_string(),
            port: 7891,
            ..GlobalProxySettings::default()
        };
        let expected = settings
            .to_proxy_url()
            .expect("代理 URL 应构建成功")
            .expect("启用代理应返回 URL");

        let client = build_app_http_client(&settings).expect("启用代理时 HTTP client 应创建成功");

        assert_eq!(client.proxy(), Some(&expected));
    }

    #[test]
    fn global_proxy_settings_validate_required_fields() {
        let settings = GlobalProxySettings {
            enabled: true,
            proxy_type: ProxyType::Https,
            host: String::new(),
            port: 0,
            username: String::new(),
            password: String::new(),
        };

        let err = settings.validate().expect_err("缺少主机和端口时应校验失败");

        assert!(err.contains("主机"));
    }
}

/// GitHub 开源地址
const GITHUB_URL: &str = "https://github.com/feigeCode/onetcli";

/// 渲染关于页面
fn render_about_section(cx: &App) -> gpui::AnyElement {
    let version = env!("CARGO_PKG_VERSION");
    let muted = cx.theme().muted_foreground;

    let disclaimer_items: Vec<String> = (1..=5)
        .map(|i| {
            let key = format!("Settings.About.disclaimer_item_{}", i);
            let text = t!(&key).to_string();
            format!("{}. {}", i, text)
        })
        .collect();

    let data_safety_items: Vec<String> = (1..=3)
        .map(|i| {
            let key = format!("Settings.About.data_safety_item_{}", i);
            let text = t!(&key).to_string();
            format!("• {}", text)
        })
        .collect();

    v_flex()
        .gap_4()
        .p_4()
        // 版本信息
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().text_sm().child(format!(
                    "{}: {}",
                    t!("Settings.About.version"),
                    version
                ))),
        )
        // GitHub 开源地址
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .child(format!("{}: ", t!("Settings.About.opensource_label"))),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().link)
                        .child(GITHUB_URL),
                )
                .child(Clipboard::new("about-copy-github-url").value(GITHUB_URL))
                .child(
                    Button::new("about-open-github")
                        .icon(IconName::ExternalLink)
                        .xsmall()
                        .ghost()
                        .on_click(|_: &ClickEvent, _, cx| {
                            cx.open_url(GITHUB_URL);
                        }),
                ),
        )
        // 免责声明
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("Settings.About.disclaimer_title").to_string()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(muted)
                        .child(t!("Settings.About.disclaimer_status").to_string()),
                )
                .child(
                    v_flex().gap_1().pl_2().children(
                        disclaimer_items
                            .into_iter()
                            .map(|item| div().text_sm().text_color(muted).child(item)),
                    ),
                ),
        )
        // 数据与安全提示
        .child(
            v_flex()
                .gap_2()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .child(t!("Settings.About.data_safety_title").to_string()),
                )
                .child(
                    v_flex().gap_1().pl_2().children(
                        data_safety_items
                            .into_iter()
                            .map(|item| div().text_sm().text_color(muted).child(item)),
                    ),
                ),
        )
        .into_any_element()
}
