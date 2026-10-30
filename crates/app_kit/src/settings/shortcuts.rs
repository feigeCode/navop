//! 快捷键设置页：列出所有可配置动作，允许直接编辑键位字符串。

use crate::actions::{self, ShortcutEntry};
use gpui::{App, SharedString, Window};
use gpui_component::button::Button;
use gpui_component::setting::{SettingField, SettingGroup, SettingItem, SettingPage};

const RESET_HINT: &str =
    "多个键位用逗号分隔，例如 cmd-n, ctrl-n；清空表示禁用。修改后重启应用生效。";
/// 当前尚未接线到实际动作的快捷键（独立产品暂不实现复制标签与切换器）。
const UNSUPPORTED_ACTIONS: &[&str] = &[
    one_core::keybindings::action_id::APP_DUPLICATE_TAB,
    one_core::keybindings::action_id::APP_OPEN_TAB_SWITCHER,
];

/// 快捷键设置页。
pub fn shortcuts_page(cx: &App) -> SettingPage {
    let entries = actions::entries();
    let items: Vec<SettingItem> = entries
        .iter()
        .map(|entry| shortcut_item(entry, cx))
        .collect();
    SettingPage::new("快捷键")
        .description("自定义本产品的快捷键")
        .groups([SettingGroup::new()
            .title("应用")
            .items(items)
            .item(reset_item())])
}

fn shortcut_item(entry: &'static ShortcutEntry, cx: &App) -> SettingItem {
    let default_text = entry.defaults().join(", ");
    let customized = if actions::is_customized(cx, entry) {
        "（已自定义）"
    } else {
        ""
    };
    let unsupported = if UNSUPPORTED_ACTIONS.contains(&entry.action_id) {
        "（当前版本暂不支持，键位保留）"
    } else {
        ""
    };
    let description = format!(
        "{}{}默认：{}。{RESET_HINT}{unsupported}",
        entry.description, customized, default_text
    );
    SettingItem::new(
        entry.title,
        SettingField::input(
            move |cx| actions::display(cx, entry),
            move |value: SharedString, cx: &mut App| apply_shortcut(entry, value.as_ref(), cx),
        ),
    )
    .description(description)
}

fn apply_shortcut(entry: &'static ShortcutEntry, value: &str, cx: &mut App) {
    let specs: Vec<String> = value
        .split(',')
        .map(|spec| spec.trim().to_string())
        .filter(|spec| !spec.is_empty())
        .collect();
    actions::set(cx, entry, specs);
}

/// 恢复全部默认键位。
pub fn reset_all(cx: &mut App) {
    for entry in actions::entries() {
        actions::reset(cx, entry);
    }
}

/// 「全部恢复默认」按钮。
fn reset_item() -> SettingItem {
    SettingItem::render(|_options, _window: &mut Window, _cx: &mut App| {
        Button::new("reset-all-shortcuts")
            .label("全部恢复默认")
            .on_click(|_, _, cx| reset_all(cx))
    })
}
