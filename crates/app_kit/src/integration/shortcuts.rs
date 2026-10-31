//! 独立产品共用的快捷键：动作定义、默认键位绑定、动作实现。
//!
//! 键位表只有一份：`crate::actions::entries()`。这里做的是「动作 ID → gpui Action」
//! 的映射，再交给 `one_core::keybindings` 按用户设置解析出真正生效的键位。设置页
//! 里改过的快捷键因此会在这里生效，不需要另维护一张表。
//!
//! 三件事刻意不绑：复制标签（`TabContainer::duplicate_tab` 是私有实现）、标签切换器
//! （`open_tab_switcher` 需要主页面配合）、以及主窗口专属的动作 —— 独立产品没有
//! 等价语义，绑上去只会看起来像坏了。

use gpui::{App, KeyBinding, Window, actions};
use gpui_component::dock::ToggleZoom;
use one_core::keybindings::{action_id, rebind_keybindings};
use one_core::tab_container::GlobalTabContainer;
use std::sync::OnceLock;

actions!(
    shortcuts,
    [
        NewConnection,
        OpenSettings,
        CloseActiveTab,
        SwitchNextTab,
        SwitchPreviousTab,
        QuitApp
    ]
);

/// `actions::ShortcutEntry` 里「打开设置」用的动作 ID（全量产品也是这个字符串）。
const OPEN_SETTINGS_ACTION_ID: &str = "app.open_settings";

/// 动作处理函数只注册一次：重复注册会让一次按键弹出两个对话框。
static ACTION_HANDLERS: OnceLock<()> = OnceLock::new();

/// 给当前应用装上全局快捷键。可重复调用：`bind_keys` 会覆盖同名绑定。
pub fn install(cx: &mut App) {
    let bindings = keybindings(cx);
    cx.bind_keys(bindings);
    if ACTION_HANDLERS.set(()).is_ok() {
        install_action_handlers(cx);
    }
}

/// 按用户设置解析每个动作的键位。没有配置时使用 `actions` 里的默认值。
fn keybindings(cx: &App) -> Vec<KeyBinding> {
    crate::actions::entries()
        .iter()
        .flat_map(|entry| bindings_for(entry, cx))
        .collect()
}

/// 独立产品真正能执行的动作。`actions` 表里的其余动作没有等价实现，见模块注释。
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortcutAction {
    NewConnection,
    OpenSettings,
    CloseActiveTab,
    SwitchNextTab,
    SwitchPreviousTab,
    QuitApp,
    ToggleZoom,
}

/// 动作 ID → 可执行动作。返回 `None` 表示这条快捷键在独立产品里被有意跳过。
fn action_for(id: &str) -> Option<ShortcutAction> {
    match id {
        action_id::HOME_NEW_CONNECTION => Some(ShortcutAction::NewConnection),
        OPEN_SETTINGS_ACTION_ID => Some(ShortcutAction::OpenSettings),
        action_id::APP_CLOSE_ACTIVE_TAB => Some(ShortcutAction::CloseActiveTab),
        action_id::APP_SWITCH_NEXT_TAB => Some(ShortcutAction::SwitchNextTab),
        action_id::APP_SWITCH_PREVIOUS_TAB => Some(ShortcutAction::SwitchPreviousTab),
        action_id::APP_QUIT => Some(ShortcutAction::QuitApp),
        action_id::WINDOW_TOGGLE_ZOOM => Some(ShortcutAction::ToggleZoom),
        _ => None,
    }
}

fn bindings_for(entry: &crate::actions::ShortcutEntry, cx: &App) -> Vec<KeyBinding> {
    let Some(action) = action_for(entry.action_id) else {
        return Vec::new();
    };
    let defaults = entry.defaults();

    match action {
        ShortcutAction::NewConnection => {
            rebind_keybindings(cx, entry.action_id, defaults, None, NewConnection)
        }
        ShortcutAction::OpenSettings => {
            rebind_keybindings(cx, entry.action_id, defaults, None, OpenSettings)
        }
        ShortcutAction::CloseActiveTab => {
            rebind_keybindings(cx, entry.action_id, defaults, None, CloseActiveTab)
        }
        ShortcutAction::SwitchNextTab => {
            rebind_keybindings(cx, entry.action_id, defaults, None, SwitchNextTab)
        }
        ShortcutAction::SwitchPreviousTab => {
            rebind_keybindings(cx, entry.action_id, defaults, None, SwitchPreviousTab)
        }
        ShortcutAction::QuitApp => rebind_keybindings(cx, entry.action_id, defaults, None, QuitApp),
        ShortcutAction::ToggleZoom => {
            rebind_keybindings(cx, entry.action_id, defaults, None, ToggleZoom)
        }
    }
}

fn install_action_handlers(cx: &mut App) {
    cx.on_action(|_: &NewConnection, cx| {
        with_active_window(cx, |window, cx| crate::open_new_connection(window, cx));
    });
    cx.on_action(|_: &OpenSettings, cx| {
        with_active_window(cx, |window, cx| crate::open_settings(window, cx));
    });
    cx.on_action(|_: &CloseActiveTab, cx| {
        with_active_window(cx, |window, cx| close_active_tab(window, cx));
    });
    cx.on_action(|_: &SwitchNextTab, cx| {
        with_active_window(cx, |window, cx| {
            cycle_tab(TabCycleDirection::Next, window, cx)
        });
    });
    cx.on_action(|_: &SwitchPreviousTab, cx| {
        with_active_window(cx, |window, cx| {
            cycle_tab(TabCycleDirection::Previous, window, cx)
        });
    });
    cx.on_action(|_: &QuitApp, cx| {
        let Some(handle) = cx.active_window() else {
            return;
        };
        // 与主窗口的关闭按钮走同一条链路：先问用户是否真的退出。
        cx.defer(move |cx| one_core::window_close::request_close_window(handle, cx));
    });
}

/// 在活动窗口上执行动作。窗口句柄必须现在取：`cx.defer` 之后活动窗口可能已经变了。
fn with_active_window(cx: &mut App, run: impl FnOnce(&mut Window, &mut App) + 'static) {
    let Some(handle) = cx.active_window() else {
        return;
    };
    cx.defer(move |cx| {
        let _ = handle.update(cx, |_, window, cx| run(window, cx));
    });
}

/// 关闭当前普通标签。固定（pinned）标签关不掉，与全量产品保持一致。
fn close_active_tab(window: &mut Window, cx: &mut App) {
    let Some(global) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = global.tab_container.clone();

    container.update(cx, |container, cx| {
        if container.is_pinned_tab_active() {
            return;
        }
        let index = container.active_index();
        container.close_tab(index, window, cx).detach();
    });
}

#[derive(Clone, Copy)]
enum TabCycleDirection {
    Next,
    Previous,
}

/// 在「固定标签 + 普通标签」组成的槽位序列上切换，语义与全量产品一致。
fn cycle_tab(direction: TabCycleDirection, window: &mut Window, cx: &mut App) {
    let Some(global) = cx.try_global::<GlobalTabContainer>() else {
        return;
    };
    let container = global.tab_container.clone();

    container.update(cx, |container, cx| {
        let pinned = container.pinned_tab_count();
        let total = pinned + container.tabs().len();
        if total == 0 {
            return;
        }

        let current = match container.active_pinned_index() {
            Some(index) => index,
            None => pinned + container.active_index(),
        };
        let next = match direction {
            TabCycleDirection::Next => (current + 1) % total,
            TabCycleDirection::Previous => (current + total - 1) % total,
        };

        if next < pinned {
            container.activate_pinned_tab_at(next, window, cx);
        } else {
            container.set_active_index(next - pinned, window, cx);
        }
    });
}

#[cfg(test)]
mod tests;
