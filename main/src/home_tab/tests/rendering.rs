use super::*;

#[test]
fn open_local_terminal_shortcut_defaults_are_conflict_free() {
    assert_eq!("cmd-alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_MACOS);
    assert_eq!("alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER);
    assert_ne!("ctrl-alt-t", OPEN_LOCAL_TERMINAL_SHORTCUT_OTHER);
}

#[test]
fn local_terminal_launcher_is_visible_in_home_toolbar() {
    let toolbar = include_str!("../toolbar.rs");
    let launcher = include_str!("../local_terminal.rs");

    assert!(toolbar.contains("render_local_terminal_button(window, cx)"));
    assert!(launcher.contains("DropdownButton::new(\"local-terminal-dropdown\")"));
    assert!(launcher.contains("IconName::SquareTerminalColor.color()"));
    assert!(launcher.contains("launch_target_is_default"));
    assert!(launcher.contains("LocalTerminalLaunchTarget::Custom"));
}

#[test]
fn connection_team_badge_uses_cached_team_name() {
    let teams = vec![TeamOption {
        id: "team-1".to_string(),
        name: "Platform".to_string(),
        key_status: one_core::cloud_sync::TeamKeyCacheStatus::Cached,
        key_version: 1,
        key_verification: None,
        last_verified_at: None,
        role: Some("member".to_string()),
        membership_state: one_core::storage::TeamMembershipState::Active,
    }];

    assert_eq!(
        Some("Platform".to_string()),
        connection_team_badge(Some("team-1"), &teams).map(|badge| badge.name)
    );
    assert!(connection_team_badge(Some("missing"), &teams).is_none());
    assert!(connection_team_badge(None, &teams).is_none());
}

#[test]
fn list_and_card_layouts_render_cached_team_badges() {
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card.rs");
    let card_content = include_str!("../connection_card_content.rs");
    let sidebar_rows = include_str!("../../persistent_connection_sidebar/rows.rs");
    let row_parts = include_str!("../../persistent_connection_sidebar/row_parts.rs");

    assert!(list_item.contains("connection_team_badge"));
    assert!(list_item.contains("render_team_badge"));
    assert!(card.contains("connection_team_badge"));
    // 卡片团队在名称行内渲染，与连接名同基线对齐
    assert!(card_content.contains("render_team_badge"));
    assert!(sidebar_rows.contains("connection_team_indicator"));
    assert!(row_parts.contains("persistent-team-"));
}

#[test]
fn connection_hover_actions_have_stable_ids() {
    let list_actions = include_str!("../connection_list_actions.rs");
    let card_actions = include_str!("../connection_card_actions.rs");

    assert!(list_actions.contains("conn-list-actions-{}"));
    assert!(card_actions.contains("{card_id}-actions"));
}

#[test]
fn home_redesign_layout_contracts() {
    let content = include_str!("../content.rs");
    let card = include_str!("../connection_card.rs");

    // 统一网格：卡片固定共享列宽，不再按组 grow/basis 自适应拉宽
    assert!(content.contains(".w(card_width)"));
    assert!(!content.contains("flex_grow_1()"));
    assert!(!content.contains("flex_basis"));
    // 共享几何由内容区统一计算一次
    assert!(content.contains("grid::card_grid_metrics"));
    // 最近区与普通区分组使用命名空间 ID，同一连接重复展示不冲突
    assert!(card.contains("conn-card-recent"));
    assert!(card.contains("conn-card\""));
    // 卡片不使用常驻或悬停阴影
    assert!(!card.contains("shadow_md"));
    assert!(!card.contains("shadow_sm"));
    // 最近区使用历史语义图标而非收藏星标，逐项「最近」角标已删除
    assert!(content.contains("NAVOP_HISTORY_ICON"));
    assert!(!content.contains("IconName::StarFill"));
    assert!(!card.contains("recent_badge"));
}

#[test]
fn group_expand_commands_live_in_a_menu() {
    let content = include_str!("../content.rs");
    // 展开全部/折叠全部归入「分组」菜单，不再各占一个标题栏按钮（redesign §4.3）
    assert!(content.contains("home-group-menu"));
    assert!(!content.contains("Button::new(\"home-expand-all\")"));
    assert!(!content.contains("Button::new(\"home-collapse-all\")"));
}

#[test]
fn home_blocking_work_is_dispatched_off_the_gpui_foreground() {
    let data = include_str!("../data.rs");
    let cloud_sync = include_str!("../cloud_sync.rs");

    assert!(data.contains("cx.background_spawn"));
    assert!(cloud_sync.contains("Tokio::spawn"));
    assert!(!cloud_sync.contains("self.log_sync_decrypt_health"));
}

#[test]
fn connection_render_uses_cached_team_permissions() {
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card.rs");

    assert!(list_item.contains("team_permissions"));
    assert!(list_item.contains("can_edit_connection"));
    assert!(card.contains("team_permissions"));
    assert!(card.contains("can_edit_connection"));
    assert!(!list_item.contains("can_edit_connection(&conn, cx)"));
    assert!(!card.contains("can_edit_connection(&conn, cx)"));
}

#[test]
fn team_key_entry_uses_team_management_feature_gate() {
    assert!(
        include_str!("../account_menu.rs")
            .contains("is_feature_enabled(Feature::TeamManagement, cx)")
    );
}

#[test]
fn account_menu_groups_sync_keys_and_authentication() {
    let account = include_str!("../account_menu.rs");
    for key in [
        "Encryption.personal_key",
        "Home.unlock_state_unlocked",
        "Home.unlock_state_locked",
        "Encryption.team_key",
        "Auth.logout",
        "Auth.login",
    ] {
        assert!(account.contains(key));
    }
    assert!(account.contains("Anchor::TopLeft"));
}

#[test]
fn home_shortcuts_are_attached_to_their_actions() {
    let toolbar = include_str!("../toolbar.rs");
    let shortcuts = include_str!("../home_shortcuts.rs");
    assert!(toolbar.contains("new_connection_tooltip(cx)"));
    assert!(include_str!("../local_terminal.rs").contains("home_shortcuts::terminal_tooltip(cx)"));
    assert!(shortcuts.contains("shortcuts_for(cx, action, &[fallback])"));
}

#[test]
fn sidebar_search_aligns_with_home_toolbar_height() {
    let tree = include_str!("../../persistent_connection_sidebar/tree.rs");
    assert!(tree.contains("fn render_tree_search"));
    assert!(tree.contains(".h_10()"));
}

#[test]
fn persistent_sidebar_supports_connection_group_drag_and_drop() {
    let rows = include_str!("../../persistent_connection_sidebar/rows.rs");
    let grouping = include_str!("../connection_grouping.rs");

    assert!(rows.contains(".on_drag("));
    assert!(rows.contains(".drag_over::<DragConnection>"));
    assert!(rows.contains("move_connection_to_workspace"));
    assert!(
        rows.contains("home.move_connection_to_workspace(drag.connection_id, workspace_id, cx);")
    );
    assert!(rows.contains("Some(id)"));
    assert!(grouping.contains("repo.update_workspace("));
    assert!(grouping.contains("ConnectionDataEvent::ConnectionUpdated"));
}

#[test]
fn persistent_sidebar_groups_expose_a_rename_interaction() {
    let rows = include_str!("../../persistent_connection_sidebar/rows.rs");
    let row_parts = include_str!("../../persistent_connection_sidebar/row_parts.rs");

    assert!(row_parts.contains("Workspace.rename"));
    assert!(rows.contains(".on_double_click("));
    assert!(rows.contains("show_workspace_dialog"));
}

#[test]
fn both_settings_entries_use_the_existing_tab_opener() {
    assert!(include_str!("../sidebar_navigation.rs").contains("home.add_settings_tab(window, cx)"));
    assert!(include_str!("../../onetcli_app.rs").contains("home.add_settings_tab(window, cx)"));
}

#[test]
fn team_key_settings_tab_has_feature_guard() {
    let source = include_str!("../../home/home_tabs.rs");
    let entry = source
        .split("pub(crate) fn add_team_key_settings_tab(")
        .nth(1)
        .expect("team key settings entry exists")
        .split("pub(crate) fn add_extensions_tab(")
        .next()
        .expect("team key settings entry has an end marker");

    assert!(entry.contains("is_feature_enabled(Feature::TeamManagement, cx)"));
}

#[test]
fn home_render_uses_cached_external_driver_registry() {
    let home = include_str!("../../home_tab.rs");
    let icon = include_str!("../connection_icon.rs");
    let visuals = include_str!("../../connection_visuals.rs");
    let list_item = include_str!("../connection_list.rs");
    let card = include_str!("../connection_card_content.rs");
    let quick_open = include_str!("../../home/home_connection_quick_open.rs");

    assert!(home.contains("external_driver_registry: IpcDriverRegistry"));
    assert!(icon.contains("stored_connection_icon"));
    assert!(visuals.contains("external_driver_icon_for_config_with_registry"));
    assert!(visuals.contains("external_driver_icon_from_sources"));
    assert!(list_item.contains("connection_icon"));
    assert!(card.contains("connection_icon"));
    assert!(quick_open.contains("stored_connection_icon"));
    assert!(quick_open.contains("external_driver_registry: IpcDriverRegistry"));
    assert!(!icon.contains("IpcDriverRegistry::load_default()"));
    assert!(!quick_open.contains("IpcDriverRegistry::load_default()"));
}
