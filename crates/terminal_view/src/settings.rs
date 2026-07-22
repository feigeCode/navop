use crate::highlight_presets::{builtin_highlight_rules, merge_builtin_highlight_rules};
use crate::theme::normalize_terminal_primary_font;
use gpui::{App, AppContext, Context, Entity, EventEmitter};
use one_core::settings::AppSettings;
use one_core::storage::get_config_dir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::error;

const TERMINAL_SETTINGS_FILE: &str = "terminal-settings.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminalHighlightRule {
    pub id: String,
    pub enabled: bool,
    pub pattern: String,
    pub foreground: Option<String>,
    pub background: Option<String>,
    pub priority: u8,
    pub note: String,
}

impl TerminalHighlightRule {
    pub fn validate(&self) -> Result<(), String> {
        if self.pattern.trim().is_empty() {
            return Err("正则不能为空".into());
        }
        if self.foreground.is_none() && self.background.is_none() {
            return Err("至少设置一种颜色".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalSettings {
    pub font_size: f32,
    #[serde(default = "default_terminal_font_family")]
    pub font_family: String,
    pub auto_copy: bool,
    pub enable_autocomplete: bool,
    pub middle_click_paste: bool,
    pub right_click_paste: bool,
    pub paste_image_upload: bool,
    pub sync_path_with_terminal: bool,
    pub cursor_blink: bool,
    pub confirm_multiline_paste: bool,
    pub confirm_high_risk_command: bool,
    /// 在 alt-screen TUI(vim/less/man 等)中把鼠标滚轮事件转为方向键发送给 PTY,
    /// 让 vim 等程序不开启鼠标报告也能滚动,同时保留终端原生选区/复制能力。
    #[serde(default = "default_vim_scroll_to_arrow_keys")]
    pub vim_scroll_to_arrow_keys: bool,
    #[serde(default)]
    pub builtin_highlights_initialized: bool,
    #[serde(default)]
    pub custom_highlights: Vec<TerminalHighlightRule>,
}

fn default_vim_scroll_to_arrow_keys() -> bool {
    true
}

fn default_terminal_font_family() -> String {
    AppSettings::default().terminal_font_family
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self::from_parts(&AppSettings::default(), &TerminalLocalSettings::default())
    }
}

impl TerminalSettings {
    fn from_parts(app_settings: &AppSettings, local_settings: &TerminalLocalSettings) -> Self {
        Self {
            font_size: app_settings.terminal_font_size as f32,
            font_family: normalize_terminal_primary_font(&app_settings.terminal_font_family),
            auto_copy: app_settings.terminal_auto_copy,
            enable_autocomplete: app_settings.terminal_enable_autocomplete,
            middle_click_paste: app_settings.terminal_middle_click_paste,
            right_click_paste: app_settings.terminal_right_click_paste,
            paste_image_upload: app_settings.terminal_paste_image_upload,
            sync_path_with_terminal: app_settings.terminal_sync_path_with_terminal,
            cursor_blink: app_settings.terminal_cursor_blink,
            confirm_multiline_paste: app_settings.terminal_confirm_multiline_paste,
            confirm_high_risk_command: app_settings.terminal_confirm_high_risk_command,
            vim_scroll_to_arrow_keys: local_settings.vim_scroll_to_arrow_keys,
            builtin_highlights_initialized: local_settings.builtin_highlights_initialized,
            custom_highlights: local_settings.custom_highlights.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
struct TerminalLocalSettings {
    #[serde(default = "default_vim_scroll_to_arrow_keys")]
    vim_scroll_to_arrow_keys: bool,
    #[serde(default)]
    builtin_highlights_initialized: bool,
    #[serde(default)]
    custom_highlights: Vec<TerminalHighlightRule>,
}

impl Default for TerminalLocalSettings {
    fn default() -> Self {
        Self {
            vim_scroll_to_arrow_keys: default_vim_scroll_to_arrow_keys(),
            builtin_highlights_initialized: true,
            custom_highlights: builtin_highlight_rules(),
        }
    }
}

impl From<&TerminalSettings> for TerminalLocalSettings {
    fn from(settings: &TerminalSettings) -> Self {
        Self {
            vim_scroll_to_arrow_keys: settings.vim_scroll_to_arrow_keys,
            builtin_highlights_initialized: settings.builtin_highlights_initialized,
            custom_highlights: settings.custom_highlights.clone(),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum TerminalSettingsEvent {
    Changed {
        previous: TerminalSettings,
        current: TerminalSettings,
    },
}

pub struct TerminalSettingsStore {
    current: TerminalLocalSettings,
    path: Option<PathBuf>,
}

impl TerminalSettingsStore {
    fn new(current: TerminalLocalSettings, path: Option<PathBuf>) -> Self {
        Self { current, path }
    }

    fn snapshot(&self) -> TerminalLocalSettings {
        self.current.clone()
    }

    fn replace(&mut self, next: TerminalLocalSettings, cx: &mut Context<Self>) {
        if self.current == next {
            return;
        }
        if let Some(path) = &self.path {
            if let Err(err) = save_settings_to_path(path, &next) {
                error!("failed to save terminal settings: {err}");
            }
        }
        let app_settings = AppSettings::current(cx);
        let previous = TerminalSettings::from_parts(&app_settings, &self.current);
        self.current = next;
        let current = TerminalSettings::from_parts(&app_settings, &self.current);
        cx.emit(TerminalSettingsEvent::Changed { previous, current });
    }
}

impl EventEmitter<TerminalSettingsEvent> for TerminalSettingsStore {}

#[derive(Clone)]
pub struct GlobalTerminalLocalSettings(pub Entity<TerminalSettingsStore>);

impl gpui::Global for GlobalTerminalLocalSettings {}

pub fn init_settings(cx: &mut App) {
    let path = terminal_settings_path().ok();
    let initial = path
        .as_deref()
        .map(resolve_initial_settings)
        .unwrap_or_default();
    if let Some(global) = cx.try_global::<GlobalTerminalLocalSettings>().cloned() {
        global.0.update(cx, |store, cx| {
            store.path = path;
            store.replace(initial, cx);
        });
    } else {
        let store = cx.new(|_| TerminalSettingsStore::new(initial, path));
        cx.set_global(GlobalTerminalLocalSettings(store));
    }
}

pub fn current_settings(cx: &App) -> TerminalSettings {
    let app_settings = AppSettings::current(cx);
    let local_settings = cx
        .try_global::<GlobalTerminalLocalSettings>()
        .map(|global| global.0.read(cx).snapshot())
        .unwrap_or_default();
    TerminalSettings::from_parts(&app_settings, &local_settings)
}

pub fn update_settings<T>(
    cx: &mut Context<T>,
    updater: impl FnOnce(&mut TerminalSettings),
) -> Option<TerminalSettings> {
    let previous = current_settings(cx);
    let mut next = previous.clone();
    updater(&mut next);
    next.font_family = normalize_terminal_primary_font(&next.font_family);
    if previous == next {
        return Some(next);
    }

    update_app_settings(&previous, &next, cx);

    if let Some(global) = cx.try_global::<GlobalTerminalLocalSettings>().cloned() {
        let next_local = TerminalLocalSettings::from(&next);
        global
            .0
            .update(cx, |store, cx| store.replace(next_local, cx));
    }

    Some(next)
}

fn terminal_settings_path() -> anyhow::Result<PathBuf> {
    let config_dir = get_config_dir()?;
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }
    Ok(config_dir.join(TERMINAL_SETTINGS_FILE))
}

fn resolve_initial_settings(path: &Path) -> TerminalLocalSettings {
    if let Some(settings) = load_settings_from_path(path) {
        let (migrated, changed) = initialize_builtin_highlights(settings);
        if changed {
            if let Err(err) = save_settings_to_path(path, &migrated) {
                error!("failed to save terminal settings after builtin highlight migration: {err}");
            }
        }
        return migrated;
    }

    TerminalLocalSettings::default()
}

fn initialize_builtin_highlights(
    mut settings: TerminalLocalSettings,
) -> (TerminalLocalSettings, bool) {
    if settings.builtin_highlights_initialized {
        return (settings, false);
    }

    settings.custom_highlights = merge_builtin_highlight_rules(&settings.custom_highlights);
    settings.builtin_highlights_initialized = true;
    (settings, true)
}

fn load_settings_from_path(path: &Path) -> Option<TerminalLocalSettings> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn save_settings_to_path(path: &Path, settings: &TerminalLocalSettings) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, json)?;
    Ok(())
}

fn update_app_settings<T>(
    previous: &TerminalSettings,
    next: &TerminalSettings,
    cx: &mut Context<T>,
) {
    if terminal_app_fields_equal(previous, next) {
        return;
    }

    AppSettings::update_and_save(cx, |settings| {
        settings.terminal_font_size = next.font_size as f64;
        settings.terminal_font_family = next.font_family.clone();
        settings.terminal_auto_copy = next.auto_copy;
        settings.terminal_enable_autocomplete = next.enable_autocomplete;
        settings.terminal_middle_click_paste = next.middle_click_paste;
        settings.terminal_right_click_paste = next.right_click_paste;
        settings.terminal_paste_image_upload = next.paste_image_upload;
        settings.terminal_sync_path_with_terminal = next.sync_path_with_terminal;
        settings.terminal_cursor_blink = next.cursor_blink;
        settings.terminal_confirm_multiline_paste = next.confirm_multiline_paste;
        settings.terminal_confirm_high_risk_command = next.confirm_high_risk_command;
    });
}

fn terminal_app_fields_equal(left: &TerminalSettings, right: &TerminalSettings) -> bool {
    left.font_size == right.font_size
        && left.font_family == right.font_family
        && left.auto_copy == right.auto_copy
        && left.enable_autocomplete == right.enable_autocomplete
        && left.middle_click_paste == right.middle_click_paste
        && left.right_click_paste == right.right_click_paste
        && left.paste_image_upload == right.paste_image_upload
        && left.sync_path_with_terminal == right.sync_path_with_terminal
        && left.cursor_blink == right.cursor_blink
        && left.confirm_multiline_paste == right.confirm_multiline_paste
        && left.confirm_high_risk_command == right.confirm_high_risk_command
}

#[cfg(test)]
mod tests {
    use super::{
        TerminalHighlightRule, TerminalLocalSettings, TerminalSettings, TerminalSettingsStore,
        load_settings_from_path, resolve_initial_settings, save_settings_to_path,
    };
    use crate::theme::default_monospace_font;
    use one_core::settings::AppSettings;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_file_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("系统时间应晚于 UNIX 纪元")
            .as_nanos();
        std::env::temp_dir().join(format!("onetcli-{name}-{nanos}.json"))
    }

    #[test]
    fn terminal_settings_save_round_trip_preserves_values() {
        let path = temp_file_path("terminal-settings-round-trip");
        let settings = TerminalLocalSettings {
            vim_scroll_to_arrow_keys: false,
            builtin_highlights_initialized: true,
            custom_highlights: Vec::new(),
        };

        save_settings_to_path(&path, &settings).expect("应写入 terminal settings");
        let loaded = load_settings_from_path(&path).expect("应读回 terminal settings");

        assert_eq!(loaded, settings);
    }

    #[test]
    fn terminal_settings_reads_local_fields_from_legacy_json() {
        let path = temp_file_path("terminal-settings-legacy-json");
        let legacy = TerminalSettings {
            font_size: 17.0,
            sync_path_with_terminal: true,
            builtin_highlights_initialized: false,
            custom_highlights: Vec::new(),
            ..TerminalSettings::default()
        };
        let json = serde_json::to_string_pretty(&legacy).expect("应序列化旧版 terminal settings");
        std::fs::write(&path, json).expect("应写入旧版 terminal settings");

        let resolved = resolve_initial_settings(&path);

        assert!(resolved.builtin_highlights_initialized);
        assert_ne!(resolved.custom_highlights, legacy.custom_highlights);
        let persisted = load_settings_from_path(&path).expect("迁移后应写出新文件");
        assert_eq!(persisted, resolved);
    }

    #[test]
    fn terminal_settings_store_replace_is_noop_when_snapshot_unchanged() {
        let initial = TerminalLocalSettings::default();
        let store = TerminalSettingsStore::new(initial.clone(), None);

        assert_eq!(store.snapshot(), initial);
        assert!(store.path.is_none());
    }

    #[test]
    fn terminal_settings_default_includes_builtin_highlight_rules() {
        let settings = TerminalSettings::default();

        assert!(settings.builtin_highlights_initialized);
        assert!(!settings.custom_highlights.is_empty());
        assert!(
            settings
                .custom_highlights
                .iter()
                .any(|rule| rule.id == "preset:ip_addresses:ipv4")
        );
    }

    #[test]
    fn terminal_settings_reads_font_family_from_app_settings() {
        let app_settings = AppSettings {
            terminal_font_family: "JetBrains Mono".to_string(),
            ..AppSettings::default()
        };

        let settings =
            TerminalSettings::from_parts(&app_settings, &TerminalLocalSettings::default());

        assert_eq!("JetBrains Mono", settings.font_family);
    }

    #[test]
    fn terminal_settings_reads_right_click_paste_from_app_settings() {
        let app_settings = AppSettings {
            terminal_right_click_paste: true,
            ..AppSettings::default()
        };

        let settings =
            TerminalSettings::from_parts(&app_settings, &TerminalLocalSettings::default());

        assert!(settings.right_click_paste);
    }

    #[test]
    fn terminal_settings_reads_paste_image_upload_from_app_settings() {
        let app_settings = AppSettings {
            terminal_paste_image_upload: false,
            ..AppSettings::default()
        };

        let settings =
            TerminalSettings::from_parts(&app_settings, &TerminalLocalSettings::default());

        assert!(!settings.paste_image_upload);
    }

    #[test]
    fn terminal_settings_normalizes_fallback_only_primary_font() {
        let app_settings = AppSettings {
            terminal_font_family: "PingFang SC".to_string(),
            ..AppSettings::default()
        };

        let settings =
            TerminalSettings::from_parts(&app_settings, &TerminalLocalSettings::default());

        assert_eq!(default_monospace_font(), settings.font_family);
    }

    #[test]
    fn terminal_settings_existing_file_is_migrated_to_builtin_rules_once() {
        let path = temp_file_path("terminal-settings-builtin-migration");
        let legacy = TerminalLocalSettings {
            builtin_highlights_initialized: false,
            custom_highlights: vec![TerminalHighlightRule {
                id: "custom:user-rule".into(),
                enabled: true,
                pattern: "\\bhello\\b".into(),
                foreground: Some("#00ff00".into()),
                background: None,
                priority: 30,
                note: "custom".into(),
            }],
            ..TerminalLocalSettings::default()
        };
        save_settings_to_path(&path, &legacy).expect("应写入旧版 terminal settings");

        let resolved = resolve_initial_settings(&path);

        assert!(resolved.builtin_highlights_initialized);
        assert!(
            resolved
                .custom_highlights
                .iter()
                .any(|rule| rule.id == "custom:user-rule")
        );
        assert!(
            resolved
                .custom_highlights
                .iter()
                .any(|rule| rule.id == "preset:ip_addresses:ipv4")
        );
    }

    #[test]
    fn terminal_settings_round_trip_preserves_custom_highlights() {
        let path = temp_file_path("terminal-settings-highlights-round-trip");
        let settings = TerminalLocalSettings {
            custom_highlights: vec![TerminalHighlightRule {
                id: "rule-1".into(),
                enabled: true,
                pattern: "\\berror\\b".into(),
                foreground: Some("#ff0000".into()),
                background: Some("#1f1f1f".into()),
                priority: 42,
                note: "Errors".into(),
            }],
            builtin_highlights_initialized: true,
            ..TerminalLocalSettings::default()
        };

        save_settings_to_path(&path, &settings).expect("应写入 terminal settings");
        let loaded = load_settings_from_path(&path).expect("应读回 terminal settings");

        assert_eq!(loaded.custom_highlights, settings.custom_highlights);
    }
}
