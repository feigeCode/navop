//! 会话内搜索（findbar）的快捷键与按键上下文。
//!
//! 形状照 `db_view::search_shortcut`：上下文常量 + 动作 + `init` / `refresh_keybindings`，
//! 由宿主（`main`）在启动与「设置变更后」各调一次。
//!
//! 分工：
//! - [`AI_CHAT_SEARCH_CONTEXT`] 挂在会话视图**根节点**上，负责「打开 findbar / 下一条 / 上一条」。
//! - [`AI_CHAT_FINDBAR_CONTEXT`] 只挂在 findbar 自身容器上，负责 `escape` 关闭。
//!   之所以能生效，是因为 `InputBaseState::escape` 在**未开启 `clean_on_escape`** 时会
//!   `cx.propagate()`（见 gpui-component `base/src/input/base/state.rs` 的 `escape`），
//!   事件因此继续冒泡到 findbar 容器；一旦打开 `clean_on_escape` 这层就会被吃掉。
//!
//! # 为什么要多绑一条复合上下文
//!
//! `gpui-component` 的 `input::init` 把 `cmd-f` 绑成了 `Input` 上下文里的 `input::Search`。
//! 用户正在 composer 里打字时，上下文栈是 `[AiChatTranscript, Input]`，只绑
//! [`AI_CHAT_SEARCH_CONTEXT`] 的这条（深度 1）会被 `Input` 那条（深度 2）整体压住，
//! 快捷键静默失效。
//!
//! 解法和 `db_view` 的 `SqlEditor > Input` 一致：再绑一条更具体的
//! [`AI_CHAT_COMPOSER_CONTEXT`]。两条绑定在这条栈上的匹配深度都是 2，胜负由注册顺序
//! 决定——本 crate 的 `init` 在 `gpui_component::init` 之后运行
//! （`main/src/navop_app.rs`），更晚注册的那条胜出。`composer_context_wins_over_input_context`
//! 这个测试把这条隐式依赖钉死，顺序一变就会炸。
//!
//! 明确不做：给 findbar 单独绑 `enter`。`Enter` / `Shift+Enter` 走
//! `InputEvent::PressEnter`（输入组件自己的事件），再绑一次属于两套真相源。

use gpui::{App, KeyBinding};
use one_core::keybindings::{action_id, rebind_keybindings, shortcuts_for};

/// 会话视图根节点的按键上下文：打开 / 前后跳转。
pub const AI_CHAT_SEARCH_CONTEXT: &str = "AiChatTranscript";
/// composer（`Input`）聚焦时的复合上下文，用来压过 `input::Search`。
pub const AI_CHAT_COMPOSER_CONTEXT: &str = "AiChatTranscript > Input";
/// findbar 自身的按键上下文：`escape` 关闭。
pub const AI_CHAT_FINDBAR_CONTEXT: &str = "AiChatFindbar";

/// 「打开 / 前后跳转」需要在两种栈形态下都生效：根节点自身，以及 composer 抢到焦点时。
const FIND_CONTEXTS: [&str; 2] = [AI_CHAT_SEARCH_CONTEXT, AI_CHAT_COMPOSER_CONTEXT];

const MACOS_FIND_SHORTCUT: &str = "cmd-f";
const OTHER_FIND_SHORTCUT: &str = "ctrl-f";
const MACOS_FIND_NEXT_SHORTCUT: &str = "cmd-g";
const OTHER_FIND_NEXT_SHORTCUT: &str = "ctrl-g";
const MACOS_FIND_PREVIOUS_SHORTCUT: &str = "cmd-shift-g";
const OTHER_FIND_PREVIOUS_SHORTCUT: &str = "ctrl-shift-g";
const CLOSE_FINDBAR_SHORTCUT: &str = "escape";

/// 默认快捷键的**唯一真相源**：绑定层与设置页都从这里取，不各写一份字面量。
pub const FIND_MACOS: &[&str] = &[MACOS_FIND_SHORTCUT];
pub const FIND_OTHER: &[&str] = &[OTHER_FIND_SHORTCUT];
pub const FIND_NEXT_MACOS: &[&str] = &[MACOS_FIND_NEXT_SHORTCUT];
pub const FIND_NEXT_OTHER: &[&str] = &[OTHER_FIND_NEXT_SHORTCUT];
pub const FIND_PREVIOUS_MACOS: &[&str] = &[MACOS_FIND_PREVIOUS_SHORTCUT];
pub const FIND_PREVIOUS_OTHER: &[&str] = &[OTHER_FIND_PREVIOUS_SHORTCUT];

gpui::actions!(
    ai_chat_search,
    [
        ToggleTranscriptFind,
        FindNextInTranscript,
        FindPreviousInTranscript,
        CloseTranscriptFind
    ]
);

pub fn init(cx: &mut App) {
    cx.bind_keys(init_keybindings(cx));
}

pub fn refresh_keybindings(cx: &mut App) {
    cx.bind_keys(refreshable_keybindings(cx));
}

fn default_find_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        FIND_MACOS
    } else {
        FIND_OTHER
    }
}

fn default_find_next_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        FIND_NEXT_MACOS
    } else {
        FIND_NEXT_OTHER
    }
}

fn default_find_previous_shortcuts_for_platform(is_macos: bool) -> &'static [&'static str] {
    if is_macos {
        FIND_PREVIOUS_MACOS
    } else {
        FIND_PREVIOUS_OTHER
    }
}

fn default_find_shortcuts() -> &'static [&'static str] {
    default_find_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_find_next_shortcuts() -> &'static [&'static str] {
    default_find_next_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn default_find_previous_shortcuts() -> &'static [&'static str] {
    default_find_previous_shortcuts_for_platform(cfg!(target_os = "macos"))
}

fn init_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();

    for key in shortcuts_for(cx, action_id::AI_CHAT_FIND, &default_find_shortcuts()) {
        for context in FIND_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, ToggleTranscriptFind, Some(context)));
        }
    }
    for key in shortcuts_for(
        cx,
        action_id::AI_CHAT_FIND_NEXT,
        &default_find_next_shortcuts(),
    ) {
        for context in FIND_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, FindNextInTranscript, Some(context)));
        }
    }
    for key in shortcuts_for(
        cx,
        action_id::AI_CHAT_FIND_PREVIOUS,
        &default_find_previous_shortcuts(),
    ) {
        for context in FIND_CONTEXTS {
            keybindings.push(KeyBinding::new(&key, FindPreviousInTranscript, Some(context)));
        }
    }
    // `escape` 不可自定义：它同时承担「关掉焦点内的浮层」这件通用语义。
    keybindings.push(KeyBinding::new(
        CLOSE_FINDBAR_SHORTCUT,
        CloseTranscriptFind,
        Some(AI_CHAT_FINDBAR_CONTEXT),
    ));
    keybindings
}

fn refreshable_keybindings(cx: &App) -> Vec<KeyBinding> {
    let mut keybindings = Vec::new();

    for context in FIND_CONTEXTS {
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_FIND,
            &default_find_shortcuts(),
            Some(context),
            ToggleTranscriptFind,
        ));
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_FIND_NEXT,
            &default_find_next_shortcuts(),
            Some(context),
            FindNextInTranscript,
        ));
        keybindings.extend(rebind_keybindings(
            cx,
            action_id::AI_CHAT_FIND_PREVIOUS,
            &default_find_previous_shortcuts(),
            Some(context),
            FindPreviousInTranscript,
        ));
    }
    keybindings
}

/// 设置页按平台取默认值。设置表直接引用这里的常量，不再自己写字面量。
pub fn find_defaults_for_platform(is_macos: bool) -> (&'static [&'static str], &'static [&'static str], &'static [&'static str]) {
    (
        default_find_shortcuts_for_platform(is_macos),
        default_find_next_shortcuts_for_platform(is_macos),
        default_find_previous_shortcuts_for_platform(is_macos),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{KeyContext, Keymap, Keystroke};

    #[test]
    fn find_uses_cmd_f_on_macos_and_ctrl_f_elsewhere() {
        assert_eq!(["cmd-f"], default_find_shortcuts_for_platform(true));
        assert_eq!(["ctrl-f"], default_find_shortcuts_for_platform(false));
    }

    #[test]
    fn find_next_uses_cmd_g_on_macos_and_ctrl_g_elsewhere() {
        assert_eq!(["cmd-g"], default_find_next_shortcuts_for_platform(true));
        assert_eq!(["ctrl-g"], default_find_next_shortcuts_for_platform(false));
    }

    #[test]
    fn find_previous_uses_shift_variants() {
        assert_eq!(
            ["cmd-shift-g"],
            default_find_previous_shortcuts_for_platform(true)
        );
        assert_eq!(
            ["ctrl-shift-g"],
            default_find_previous_shortcuts_for_platform(false)
        );
    }

    /// 设置页展示的默认值必须与绑定的平台默认值一致，否则用户看到的和实际生效的是两回事。
    #[test]
    fn settings_defaults_match_the_binding_layer() {
        for is_macos in [true, false] {
            let (find, next, previous) = find_defaults_for_platform(is_macos);
            assert_eq!(find, default_find_shortcuts_for_platform(is_macos));
            assert_eq!(next, default_find_next_shortcuts_for_platform(is_macos));
            assert_eq!(
                previous,
                default_find_previous_shortcuts_for_platform(is_macos)
            );
        }
    }

    /// 三条跳转快捷键必须互不冲突，且都不能是空串（空串会被 `Keystroke::parse` 判非法）。
    #[test]
    fn default_shortcuts_are_valid_and_distinct() {
        for is_macos in [true, false] {
            let (find, next, previous) = find_defaults_for_platform(is_macos);
            let all: Vec<&str> = find.iter().chain(next).chain(previous).copied().collect();
            assert!(!all.iter().any(|spec| spec.is_empty()));
            let unique: std::collections::HashSet<&&str> = all.iter().collect();
            assert_eq!(all.len(), unique.len(), "duplicate shortcuts in {all:?}");
        }
    }

    // 「别人的」绑定，用来扮演 `gpui_component::input::init` 里的 `input::Search`
    // （它不在公开导出里，测试只能同形替代，但上下文与深度完全一致）。
    gpui::actions!(ai_chat_find_shortcut_tests, [ForeignInputAction]);

    fn resolve(keystroke: &str, contexts: &[&str]) -> Option<Box<dyn gpui::Action>> {
        let keymap = Keymap::new(vec![
            // 先注册：`gpui_component::init` 在 `ai_chat_view::init` 之前跑。
            KeyBinding::new(keystroke, ForeignInputAction, Some("Input")),
            // 后注册：本 crate 自己那两条。
            KeyBinding::new(keystroke, ToggleTranscriptFind, Some(AI_CHAT_SEARCH_CONTEXT)),
            KeyBinding::new(keystroke, ToggleTranscriptFind, Some(AI_CHAT_COMPOSER_CONTEXT)),
        ]);
        let contexts = contexts
            .iter()
            .map(|context| KeyContext::parse(context).expect("valid context"))
            .collect::<Vec<_>>();
        let keystroke = Keystroke::parse(keystroke).expect("valid keystroke");
        let (bindings, _) = keymap.bindings_for_input(&[keystroke], &contexts);
        bindings.first().map(|binding| binding.action().boxed_clone())
    }

    fn resolves_to_find(keystroke: &str, contexts: &[&str]) -> bool {
        resolve(keystroke, contexts)
            .is_some_and(|action| action.partial_eq(&ToggleTranscriptFind))
    }

    /// composer 获得焦点时，`cmd-f` 必须还是「搜会话」，不能被输入组件自己的搜索抢走。
    ///
    /// 两条绑定在 `[AiChatTranscript, Input]` 上的匹配深度都是 2，胜负**只由注册顺序**决定。
    /// 这个测试把那条隐式依赖钉死：谁把 `ai_chat_view::init` 提到 `gpui_component::init` 之前，
    /// 这里会先炸，而不是上线后「快捷键时灵时不灵」。
    #[test]
    fn composer_context_wins_over_input_context() {
        assert!(
            resolves_to_find("cmd-f", &[AI_CHAT_SEARCH_CONTEXT, "Input"]),
            "composer focus must not hand cmd-f to the input's own search action"
        );
    }

    /// 焦点不在 composer 上时（栈里只有根节点），基本绑定同样生效。
    #[test]
    fn root_context_alone_still_resolves_the_find_shortcut() {
        assert!(resolves_to_find("cmd-f", &[AI_CHAT_SEARCH_CONTEXT]));
    }

    /// findbar 的搜索框自己也是 `Input`：此时栈是三层，复合上下文要照样匹配得上。
    #[test]
    fn findbar_input_keeps_the_find_shortcut_reachable() {
        assert!(resolves_to_find(
            "cmd-f",
            &[AI_CHAT_SEARCH_CONTEXT, AI_CHAT_FINDBAR_CONTEXT, "Input"]
        ));
    }

    /// 三层栈下 `escape` 必须落到 findbar 自己的上下文上，否则关不掉。
    #[test]
    fn escape_resolves_on_the_findbar_context() {
        let keymap = Keymap::new(vec![KeyBinding::new(
            CLOSE_FINDBAR_SHORTCUT,
            CloseTranscriptFind,
            Some(AI_CHAT_FINDBAR_CONTEXT),
        )]);
        let contexts = [AI_CHAT_SEARCH_CONTEXT, AI_CHAT_FINDBAR_CONTEXT, "Input"]
            .iter()
            .map(|context| KeyContext::parse(context).expect("valid context"))
            .collect::<Vec<_>>();
        let keystroke = Keystroke::parse(CLOSE_FINDBAR_SHORTCUT).expect("valid keystroke");
        let (bindings, _) = keymap.bindings_for_input(&[keystroke], &contexts);

        assert!(
            bindings
                .first()
                .is_some_and(|binding| binding.action().partial_eq(&CloseTranscriptFind)),
            "escape must close the findbar while its own input is focused"
        );
    }
}
