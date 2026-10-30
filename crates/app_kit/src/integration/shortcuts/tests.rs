//! 快捷键表的守卫：每一条 `actions` 条目要么被绑定，要么被有意跳过。

use one_core::keybindings::action_id;

use super::{OPEN_SETTINGS_ACTION_ID, ShortcutAction, action_for};
use crate::actions;

/// 独立产品会真正绑定的动作 ID。
fn bound_action_ids() -> [&'static str; 7] {
    [
        action_id::HOME_NEW_CONNECTION,
        OPEN_SETTINGS_ACTION_ID,
        action_id::APP_CLOSE_ACTIVE_TAB,
        action_id::APP_SWITCH_NEXT_TAB,
        action_id::APP_SWITCH_PREVIOUS_TAB,
        action_id::APP_QUIT,
        action_id::WINDOW_TOGGLE_ZOOM,
    ]
}

/// 有意跳过的动作 ID（独立产品没有等价实现）。
fn skipped_action_ids() -> [&'static str; 2] {
    [
        action_id::APP_DUPLICATE_TAB,
        action_id::APP_OPEN_TAB_SWITCHER,
    ]
}

#[test]
fn every_shortcut_entry_is_either_bound_or_explicitly_skipped() {
    for entry in actions::entries() {
        let bound = bound_action_ids().contains(&entry.action_id);
        let skipped = skipped_action_ids().contains(&entry.action_id);

        assert!(
            bound ^ skipped,
            "{} 既没有绑定、也没有写明跳过原因：新增动作时要同步这里",
            entry.action_id
        );
        assert_eq!(
            action_for(entry.action_id).is_some(),
            bound,
            "{} 的映射与绑定表不一致",
            entry.action_id
        );
    }
}

#[test]
fn bound_actions_have_defaults_and_skipped_ones_do_not_resolve() {
    for entry in actions::entries() {
        if !bound_action_ids().contains(&entry.action_id) {
            continue;
        }
        assert!(
            !entry.defaults().is_empty(),
            "{} 绑定了却没有默认键位，用户会以为快捷键坏了",
            entry.action_id
        );
    }

    for id in skipped_action_ids() {
        assert_eq!(action_for(id), None, "{id} 应当被有意跳过");
    }
}

#[test]
fn settings_shortcut_keeps_its_shared_action_id() {
    assert_eq!(OPEN_SETTINGS_ACTION_ID, "app.open_settings");
    assert_eq!(
        action_for(OPEN_SETTINGS_ACTION_ID),
        Some(ShortcutAction::OpenSettings)
    );
}
