//! 独立产品的快捷键元数据：动作 id、默认键位与展示名。
//!
//! 设置页的「快捷键」分组与 `integration::shortcuts` 的键位注册共用这一份定义，
//! 用户改过的键位写入 `AppSettings::custom_keybindings`（按动作 id 存档）。

use gpui::{App, SharedString};
use one_core::keybindings::{action_id, shortcuts_for};
use one_core::settings::AppSettings;

/// 一个可配置的快捷键动作。
pub struct ShortcutEntry {
    pub action_id: &'static str,
    pub title: &'static str,
    pub description: &'static str,
    mac_defaults: &'static [&'static str],
    other_defaults: &'static [&'static str],
}

impl ShortcutEntry {
    /// 当前平台的默认键位。
    pub fn defaults(&self) -> &'static [&'static str] {
        if cfg!(target_os = "macos") {
            self.mac_defaults
        } else {
            self.other_defaults
        }
    }
}

const ENTRIES: &[ShortcutEntry] = &[
    ShortcutEntry {
        action_id: action_id::HOME_NEW_CONNECTION,
        title: "新建连接",
        description: "打开新建连接向导",
        mac_defaults: &["cmd-n"],
        other_defaults: &["ctrl-n"],
    },
    ShortcutEntry {
        action_id: "app.open_settings",
        title: "打开设置",
        description: "打开本产品的设置页",
        mac_defaults: &["cmd-,"],
        other_defaults: &["ctrl-,"],
    },
    ShortcutEntry {
        action_id: action_id::APP_CLOSE_ACTIVE_TAB,
        title: "关闭当前标签",
        description: "关闭当前激活的标签",
        mac_defaults: &["cmd-shift-w", "alt-shift-w"],
        other_defaults: &["alt-shift-w"],
    },
    ShortcutEntry {
        action_id: action_id::APP_SWITCH_NEXT_TAB,
        title: "下一个标签",
        description: "循环切换到右侧标签",
        mac_defaults: &["ctrl-tab"],
        other_defaults: &["ctrl-tab"],
    },
    ShortcutEntry {
        action_id: action_id::APP_SWITCH_PREVIOUS_TAB,
        title: "上一个标签",
        description: "循环切换到左侧标签",
        mac_defaults: &["ctrl-shift-tab"],
        other_defaults: &["ctrl-shift-tab"],
    },
    ShortcutEntry {
        action_id: action_id::APP_DUPLICATE_TAB,
        title: "复制当前标签",
        description: "为当前标签打开一个相同内容的副本",
        mac_defaults: &["cmd-shift-t"],
        other_defaults: &["alt-shift-t"],
    },
    ShortcutEntry {
        action_id: action_id::APP_OPEN_TAB_SWITCHER,
        title: "标签切换器",
        description: "打开标签快速切换面板",
        mac_defaults: &["cmd-j"],
        other_defaults: &["ctrl-j"],
    },
    ShortcutEntry {
        action_id: action_id::WINDOW_TOGGLE_ZOOM,
        title: "缩放面板",
        description: "最大化/还原当前面板",
        mac_defaults: &["shift-escape"],
        other_defaults: &["shift-escape"],
    },
    ShortcutEntry {
        action_id: action_id::APP_QUIT,
        title: "退出应用",
        description: "关闭所有窗口并退出",
        mac_defaults: &["cmd-q"],
        other_defaults: &["alt-f4"],
    },
];

/// 全部可配置动作。
pub fn entries() -> &'static [ShortcutEntry] {
    ENTRIES
}

/// 当前生效的键位（含用户覆盖）。
pub fn current(cx: &App, entry: &ShortcutEntry) -> Vec<String> {
    shortcuts_for(cx, entry.action_id, entry.defaults())
}

/// 当前键位的展示文本，例如 `cmd-n, ctrl-n`。
pub fn display(cx: &App, entry: &ShortcutEntry) -> SharedString {
    let shortcuts = current(cx, entry);
    if shortcuts.is_empty() {
        return SharedString::new_static("未设置");
    }
    SharedString::from(shortcuts.join(", "))
}

/// 写入用户覆盖；空字符串表示禁用该动作。返回是否发生变化。
pub fn set(cx: &mut App, entry: &ShortcutEntry, specs: Vec<String>) -> bool {
    let normalized: Vec<String> = specs
        .into_iter()
        .map(|spec| spec.trim().to_string())
        .filter(|spec| !spec.is_empty())
        .collect();
    if current(cx, entry) == normalized {
        return false;
    }
    let action_id = entry.action_id.to_string();
    AppSettings::update_and_save(cx, |settings| {
        settings.custom_keybindings.insert(action_id, normalized);
    });
    true
}

/// 恢复某个动作的默认键位。返回是否发生变化。
pub fn reset(cx: &mut App, entry: &ShortcutEntry) -> bool {
    if !AppSettings::global(cx)
        .custom_keybindings
        .contains_key(entry.action_id)
    {
        return false;
    }
    let action_id = entry.action_id.to_string();
    AppSettings::update_and_save(cx, |settings| {
        settings.custom_keybindings.remove(&action_id);
    });
    true
}

/// 是否存在用户覆盖。
pub fn is_customized(cx: &App, entry: &ShortcutEntry) -> bool {
    AppSettings::global(cx)
        .custom_keybindings
        .contains_key(entry.action_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: &str) -> &'static ShortcutEntry {
        entries()
            .iter()
            .find(|entry| entry.action_id == id)
            .expect("entry exists")
    }

    #[test]
    fn entries_have_unique_action_ids() {
        let mut ids: Vec<&str> = entries().iter().map(|entry| entry.action_id).collect();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(total, ids.len(), "action id 不应重复");
    }

    #[test]
    fn defaults_are_never_empty() {
        for entry in entries() {
            assert!(
                !entry.defaults().is_empty(),
                "{} 缺少默认键位",
                entry.action_id
            );
        }
    }

    #[test]
    fn lookup_by_action_id_works() {
        assert_eq!(entry(action_id::HOME_NEW_CONNECTION).title, "新建连接");
        assert!(
            entries()
                .iter()
                .all(|entry| !entry.title.is_empty() && !entry.description.is_empty())
        );
    }
}
