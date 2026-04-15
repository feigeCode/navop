use gpui::{App, AppContext, Context, Entity, EventEmitter};
use one_core::storage::get_config_dir;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tracing::error;

const TERMINAL_SETTINGS_FILE: &str = "terminal-settings.json";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TerminalSettings {
    pub font_size: f32,
    pub auto_copy: bool,
    pub enable_autocomplete: bool,
    pub middle_click_paste: bool,
    pub sync_path_with_terminal: bool,
    pub theme: String,
    pub cursor_blink: bool,
    pub confirm_multiline_paste: bool,
    pub confirm_high_risk_command: bool,
}

impl Default for TerminalSettings {
    fn default() -> Self {
        Self {
            font_size: 15.0,
            auto_copy: true,
            enable_autocomplete: true,
            middle_click_paste: true,
            sync_path_with_terminal: false,
            theme: "ocean".to_string(),
            cursor_blink: false,
            confirm_multiline_paste: true,
            confirm_high_risk_command: true,
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
    current: TerminalSettings,
    path: Option<PathBuf>,
}

impl TerminalSettingsStore {
    fn new(current: TerminalSettings, path: Option<PathBuf>) -> Self {
        Self { current, path }
    }

    pub fn snapshot(&self) -> TerminalSettings {
        self.current.clone()
    }

    pub fn replace(&mut self, next: TerminalSettings, cx: &mut Context<Self>) {
        if self.current == next {
            return;
        }
        if let Some(path) = &self.path {
            if let Err(err) = save_settings_to_path(path, &next) {
                error!("failed to save terminal settings: {err}");
            }
        }
        let previous = self.current.clone();
        self.current = next;
        cx.emit(TerminalSettingsEvent::Changed {
            previous,
            current: self.current.clone(),
        });
    }
}

impl EventEmitter<TerminalSettingsEvent> for TerminalSettingsStore {}

#[derive(Clone)]
pub struct GlobalTerminalSettings(pub Entity<TerminalSettingsStore>);

impl gpui::Global for GlobalTerminalSettings {}

pub fn init_settings(cx: &mut App, legacy_seed: Option<TerminalSettings>) {
    let path = terminal_settings_path().ok();
    let initial = path
        .as_deref()
        .map(|path| resolve_initial_settings(path, legacy_seed))
        .unwrap_or_default();
    if let Some(global) = cx.try_global::<GlobalTerminalSettings>().cloned() {
        global.0.update(cx, |store, cx| {
            store.path = path;
            store.replace(initial, cx);
        });
    } else {
        let store = cx.new(|_| TerminalSettingsStore::new(initial, path));
        cx.set_global(GlobalTerminalSettings(store));
    }
}

pub fn current_settings(cx: &App) -> TerminalSettings {
    cx.try_global::<GlobalTerminalSettings>()
        .map(|global| global.0.read(cx).snapshot())
        .unwrap_or_default()
}

pub fn update_settings<T>(
    cx: &mut Context<T>,
    updater: impl FnOnce(&mut TerminalSettings),
) -> Option<TerminalSettings> {
    let store = cx.try_global::<GlobalTerminalSettings>()?.0.clone();
    let mut updated = None;
    store.update(cx, |store, cx| {
        let mut next = store.snapshot();
        updater(&mut next);
        store.replace(next.clone(), cx);
        updated = Some(next);
    });
    updated
}

fn terminal_settings_path() -> anyhow::Result<PathBuf> {
    let config_dir = get_config_dir()?;
    if !config_dir.exists() {
        std::fs::create_dir_all(&config_dir)?;
    }
    Ok(config_dir.join(TERMINAL_SETTINGS_FILE))
}

fn resolve_initial_settings(path: &Path, legacy_seed: Option<TerminalSettings>) -> TerminalSettings {
    if let Some(settings) = load_settings_from_path(path) {
        return settings;
    }

    if let Some(legacy) = legacy_seed {
        if let Err(err) = save_settings_to_path(path, &legacy) {
            error!("failed to migrate legacy terminal settings: {err}");
        }
        return legacy;
    }

    TerminalSettings::default()
}

fn load_settings_from_path(path: &Path) -> Option<TerminalSettings> {
    let json = std::fs::read_to_string(path).ok()?;
    serde_json::from_str(&json).ok()
}

fn save_settings_to_path(path: &Path, settings: &TerminalSettings) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)?;
    std::fs::write(path, json)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        load_settings_from_path, resolve_initial_settings, save_settings_to_path, TerminalSettings,
        TerminalSettingsStore,
    };
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
        let settings = TerminalSettings {
            font_size: 18.0,
            auto_copy: false,
            enable_autocomplete: false,
            middle_click_paste: false,
            sync_path_with_terminal: true,
            theme: "forest".to_string(),
            cursor_blink: true,
            confirm_multiline_paste: false,
            confirm_high_risk_command: false,
        };

        save_settings_to_path(&path, &settings).expect("应写入 terminal settings");
        let loaded = load_settings_from_path(&path).expect("应读回 terminal settings");

        assert_eq!(loaded, settings);
    }

    #[test]
    fn terminal_settings_use_legacy_seed_when_json_absent() {
        let path = temp_file_path("terminal-settings-legacy-seed");
        let legacy = TerminalSettings {
            font_size: 17.0,
            theme: "light".to_string(),
            sync_path_with_terminal: true,
            ..TerminalSettings::default()
        };

        let resolved = resolve_initial_settings(&path, Some(legacy.clone()));

        assert_eq!(resolved, legacy);
        let persisted = load_settings_from_path(&path).expect("迁移后应写出新文件");
        assert_eq!(persisted, legacy);
    }

    #[test]
    fn terminal_settings_store_replace_is_noop_when_snapshot_unchanged() {
        let initial = TerminalSettings::default();
        let store = TerminalSettingsStore::new(initial.clone(), None);

        assert_eq!(store.snapshot(), initial);
        assert!(store.path.is_none());
    }
}
