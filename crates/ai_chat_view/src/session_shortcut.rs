//! 会话级快捷键：后退 / 前进导航、最近会话切换器。
//!
//! 形状照 [`crate::find_shortcut`]：上下文常量 + 动作 + `init` / `refresh_keybindings`，
//! 由宿主（`main`）在启动与「设置变更后」各调一次；`init` 已随 `ai_chat_view::init`
//! 注册，漏调宿主也不会静默失效。
//!
//! 分工：
//! - 后退 / 前进 / 打开切换器：挂在会话视图**根节点**（复用 findbar 的上下文常量），
//!   composer 聚焦时靠 `AiChatTranscript > Input` 复合上下文压过 `gpui-component`
//!   自己的同键绑定（与 findbar 同一套坑，测试同样钉死）。
//! - 切换器**内部**的循环 / 确认 / 取消：挂在 [`AI_CHAT_SESSION_SWITCHER_CONTEXT`]
//!   上——只有切换器打开且持有焦点时才生效。这些键（tab / 方向键 / enter / escape）
//!   不提供自定义：`tab` / `escape` 承担通用语义，可自定义会造成「切换器关不掉」。

use gpui::{App, KeyBinding};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

/// 复用 findbar 的根上下文（同一个视图根节点，不另立一套）。
use crate::find_shortcut::{AI_CHAT_COMPOSER_CONTEXT, AI_CHAT_SEARCH_CONTEXT};

/// 会话切换器 overlay 自身的按键上下文。
pub const AI_CHAT_SESSION_SWITCHER_CONTEXT: &str = "AgentSessionSwitcher";

const NAVIGATION_CONTEXTS: [&str; 2] = [AI_CHAT_SEARCH_CONTEXT, AI_CHAT_COMPOSER_CONTEXT];

const MACOS_BACK_SHORTCUT: &str = "cmd-[";
const OTHER_BACK_SHORTCUT: &str = "ctrl-[";
const MACOS_FORWARD_SHORTCUT: &str = "cmd-]";
const OTHER_FORWARD_SHORTCUT: &str = "ctrl-]";
const MACOS_SWITCHER_SHORTCUT: &str = "cmd-shift-j";
const OTHER_SWITCHER_SHORTCUT: &str = "ctrl-shift-j";

/// 默认快捷键的**唯一真相源**：绑定层与设置页都从这里取。
pub const SESSION_BACK_MACOS: &[&str] = &[MACOS_BACK_SHORTCUT];
pub const SESSION_BACK_OTHER: &[&str] = &[OTHER_BACK_SHORTCUT];
pub const SESSION_FORWARD_MACOS: &[&str] = &[MACOS_FORWARD_SHORTCUT];
pub const SESSION_FORWARD_OTHER: &[&str] = &[OTHER_FORWARD_SHORTCUT];
pub const SESSION_SWITCHER_MACOS: &[&str] = &[MACOS_SWITCHER_SHORTCUT];
pub const SESSION_SWITCHER_OTHER: &[&str] = &[OTHER_SWITCHER_SHORTCUT];

gpui::actions!(
    ai_chat_session,
    [
        NavigateSessionBack,
        NavigateSessionForward,
        ToggleSessionSwitcher,
        CycleSessionSwitcherForward,
        CycleSessionSwitcherBackward,
        SelectFirstSessionInSwitcher,
        SelectLastSessionInSwitcher,
        ConfirmSessionSwitch,
        CancelSessionSwitch
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn default_back_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        SESSION_BACK_MACOS
    } else {
        SESSION_BACK_OTHER
    }
}

fn default_forward_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        SESSION_FORWARD_MACOS
    } else {
        SESSION_FORWARD_OTHER
    }
}

fn default_switcher_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        SESSION_SWITCHER_MACOS
    } else {
        SESSION_SWITCHER_OTHER
    }
}

fn default_back_shortcuts() -> &'static [&'static str] {
    default_back_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_forward_shortcuts() -> &'static [&'static str] {
    default_forward_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_switcher_shortcuts() -> &'static [&'static str] {
    default_switcher_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();

    for key in shortcuts_for(
        cx,
        action_id::AI_CHAT_SESSION_BACK,
        &default_back_shortcuts(),
    ) {
        for context in NAVIGATION_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, NavigateSessionBack, Some(context)));
        }
    }
    for key in shortcuts_for(
        cx,
        action_id::AI_CHAT_SESSION_FORWARD,
        &default_forward_shortcuts(),
    ) {
        for context in NAVIGATION_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, NavigateSessionForward, Some(context)));
        }
    }
    for key in shortcuts_for(
        cx,
        action_id::AI_CHAT_SESSION_SWITCHER,
        &default_switcher_shortcuts(),
    ) {
        for context in NAVIGATION_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, ToggleSessionSwitcher, Some(context)));
        }
    }

    // 切换器内部键：不可自定义（tab / escape 语义太通用，改了会关不掉或打不了字）。
    let switcher = AI_CHAT_SESSION_SWITCHER_CONTEXT;
    for key in ["tab", "down", "right"] {
        keybindings.push(KeyBinding::new(
            key,
            CycleSessionSwitcherForward,
            Some(switcher),
        ));
    }
    for key in ["shift-tab", "up", "left"] {
        keybindings.push(KeyBinding::new(
            key,
            CycleSessionSwitcherBackward,
            Some(switcher),
        ));
    }
    keybindings.push(KeyBinding::new(
        "home",
        SelectFirstSessionInSwitcher,
        Some(switcher),
    ));
    keybindings.push(KeyBinding::new(
        "end",
        SelectLastSessionInSwitcher,
        Some(switcher),
    ));
    keybindings.push(KeyBinding::new(
        "enter",
        ConfirmSessionSwitch,
        Some(switcher),
    ));
    keybindings.push(KeyBinding::new(
        "escape",
        CancelSessionSwitch,
        Some(switcher),
    ));
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();
    for context in NAVIGATION_CONTEXTS {
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_SESSION_BACK,
            &default_back_shortcuts(),
            Some(context),
            NavigateSessionBack,
        ));
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_SESSION_FORWARD,
            &default_forward_shortcuts(),
            Some(context),
            NavigateSessionForward,
        ));
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_SESSION_SWITCHER,
            &default_switcher_shortcuts(),
            Some(context),
            ToggleSessionSwitcher,
        ));
    }
    keybindings
}

/// 设置页按平台取默认值；必须与绑定层同源（见 [`crate::find_shortcut`] 的同款测试）。
pub fn session_shortcut_defaults_for_platform(
    is_macos: bool,
) -> (
    &'static [&'static str],
    &'static [&'static str],
    &'static [&'static str],
) {
    (
        default_back_shortcuts_for_platform(is_macos),
        default_forward_shortcuts_for_platform(is_macos),
        default_switcher_shortcuts_for_platform(is_macos),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_uses_brackets_and_switcher_uses_secondary_shift_j() {
        assert_eq!(["cmd-["], default_back_shortcuts_for_platform(true));
        assert_eq!(["ctrl-["], default_back_shortcuts_for_platform(false));
        assert_eq!(["cmd-]"], default_forward_shortcuts_for_platform(true));
        assert_eq!(["ctrl-]"], default_forward_shortcuts_for_platform(false));
        assert_eq!(
            ["cmd-shift-j"],
            default_switcher_shortcuts_for_platform(true)
        );
        assert_eq!(
            ["ctrl-shift-j"],
            default_switcher_shortcuts_for_platform(false)
        );
    }

    /// 三条可自定义的快捷键互不冲突，也不能是空串（`Keystroke::parse` 判非法）。
    #[test]
    fn customizable_shortcuts_are_valid_and_distinct() {
        for is_macos in [true, false] {
            let (back, forward, switcher) = session_shortcut_defaults_for_platform(is_macos);
            let all: Vec<&str> = back
                .iter()
                .chain(forward)
                .chain(switcher)
                .copied()
                .collect();
            assert!(!all.iter().any(|spec| spec.is_empty()));
            let unique: std::collections::HashSet<&&str> = all.iter().collect();
            assert_eq!(all.len(), unique.len(), "duplicate shortcuts in {all:?}");
        }
    }

    /// 设置页展示的默认值必须与绑定层一致（同 findbar 的纪律）。
    #[test]
    fn settings_defaults_match_the_binding_layer() {
        for is_macos in [true, false] {
            let (back, forward, switcher) = session_shortcut_defaults_for_platform(is_macos);
            assert_eq!(back, default_back_shortcuts_for_platform(is_macos));
            assert_eq!(forward, default_forward_shortcuts_for_platform(is_macos));
            assert_eq!(switcher, default_switcher_shortcuts_for_platform(is_macos));
        }
    }

    /// `ctrl-tab` 在本应用是「工作台 Tab 切换」的全局键，会话切换器**不能**抢它。
    /// 这条把「为什么默认键是 cmd-shift-j 而不是 ctrl-tab」钉进测试：谁想改成
    /// ctrl-tab，先想起这条再掂量。
    #[test]
    fn the_switcher_default_does_not_shadow_the_app_tab_switcher() {
        for is_macos in [true, false] {
            let (_, _, switcher) = session_shortcut_defaults_for_platform(is_macos);
            assert!(
                !switcher.contains(&"ctrl-tab") && !switcher.contains(&"ctrl-shift-tab"),
                "ctrl-tab / ctrl-shift-tab 属于应用级 Tab 切换，会话切换器不得占用"
            );
        }
    }
}
