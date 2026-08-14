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
fn both_home_styles_expose_credential_vault_in_their_native_navigation() {
    let toolbar = include_str!("../toolbar.rs");
    let legacy_sidebar = include_str!("../sidebar.rs");
    let persistent_sidebar = include_str!("../../persistent_connection_sidebar/rail.rs");
    let modern_home = include_str!("../modern_home.rs");
    let workspace_tools = modern_home
        .split("fn render_workspace_tools")
        .nth(1)
        .and_then(|source| source.split("fn render_status_panel").next())
        .expect("modern workspace tools section");

    assert!(!toolbar.contains("\"credential-vault-button\""));
    assert!(!toolbar.contains("add_credential_vault_tab"));

    assert!(legacy_sidebar.contains("\"legacy-open-credential-vault\""));
    assert!(legacy_sidebar.contains("t!(\"Home.credential_vault\")"));
    assert!(legacy_sidebar.contains("home.add_credential_vault_tab(window, cx)"));

    assert!(!persistent_sidebar.contains("\"persistent-open-credential-vault\""));

    assert!(workspace_tools.contains("\"modern-home-credential-vault\""));
    assert!(workspace_tools.contains("t!(\"Home.credential_vault\")"));
    assert!(workspace_tools.contains("home.add_credential_vault_tab(window, cx)"));
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
    let sidebar_rows = include_str!("../../persistent_connection_sidebar/rows.rs");
    let row_parts = include_str!("../../persistent_connection_sidebar/row_parts.rs");

    assert!(list_item.contains("connection_team_badge"));
    assert!(list_item.contains("conn-list-team-"));
    assert!(card.contains("connection_team_badge"));
    assert!(card.contains("conn-team-"));
    assert!(sidebar_rows.contains("connection_team_indicator"));
    assert!(row_parts.contains("persistent-team-"));
}

#[test]
fn connection_hover_actions_have_stable_ids() {
    let list_actions = include_str!("../connection_list_actions.rs");
    let card_actions = include_str!("../connection_card_actions.rs");

    assert!(list_actions.contains("conn-list-actions-{}"));
    assert!(card_actions.contains("conn-card-actions-{}"));
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
    let toolbar = include_str!("../toolbar.rs");

    assert!(toolbar.contains("is_feature_enabled(Feature::TeamManagement, cx)"));
}

#[test]
fn personal_and_team_keys_share_one_toolbar_menu() {
    let toolbar = include_str!("../toolbar.rs");

    assert!(toolbar.contains("Button::new(\"key-menu-button\")"));
    assert!(toolbar.contains(".dropdown_caret(true)"));
    assert!(toolbar.contains("Encryption.personal_key_unlocked"));
    assert!(toolbar.contains("Encryption.personal_key_locked"));
    assert!(toolbar.contains("Encryption.team_key"));
    assert!(!toolbar.contains("Button::new(\"team-key-button\")"));
}

#[test]
fn home_overview_is_compact_and_avoids_duplicate_search() {
    let toolbar = include_str!("../toolbar.rs");
    let content = include_str!("../content.rs");
    let card = include_str!("../connection_card.rs");
    let render = include_str!("../render.rs");
    let modern_home = include_str!("../modern_home.rs");

    assert!(toolbar.contains("Input::new(&self.search_input)"));
    assert!(content.contains("max_w(px(1160.0))"));
    assert!(content.contains("MODERN_HOME_CARD_MIN_WIDTH"));
    assert!(content.contains("MODERN_HOME_CARD_MAX_WIDTH"));
    assert!(content.contains(".flex_grow()"));
    assert!(card.contains("px(76.0)"));
    assert!(!card.contains(".shadow_sm()\n            .group"));
    assert!(render.contains("self.render_modern_home(window, cx)"));
    assert!(modern_home.contains("modern-home-start-center"));
    assert!(modern_home.contains("START_CENTER_MAX_WIDTH: gpui::Pixels = px(1040.0)"));
    assert!(modern_home.contains(".max_w(START_CENTER_MAX_WIDTH)"));
    assert!(modern_home.contains("modern-home-hero"));
    assert!(modern_home.contains("modern-home-recent-column"));
    assert!(modern_home.contains("modern-home-side-column"));
    assert!(!modern_home.contains("render_connection_card"));
    assert!(!modern_home.contains("view.update(cx, |home"));
    assert!(modern_home.contains("modern-home-sync"));
    assert!(modern_home.contains("modern-home-keys"));
    assert!(modern_home.contains(
        ".size_full()\n            .overflow_y_scroll()\n            .scrollbar_width(px(0.0))"
    ));
    assert!(!modern_home.contains(".min_h_full()"));
    assert!(modern_home.contains("self.render_local_terminal_button(window, cx)"));
    assert!(!modern_home.contains("modern-home-local-terminal"));
    assert!(!modern_home.contains("IconName::Terminal).with_size(px(42.0))"));
    assert!(!modern_home.contains(".read(cx)"));
}

#[test]
fn modern_start_center_separates_primary_work_from_supporting_tools() {
    let modern_home = include_str!("../modern_home.rs");

    for stable_id in [
        "modern-home-hero",
        "modern-home-recent-panel",
        "modern-home-create-panel",
        "modern-home-tools-panel",
        "modern-home-status-panel",
        "modern-home-sync",
        "modern-home-keys",
    ] {
        assert!(modern_home.contains(stable_id));
    }
    assert!(modern_home.contains(".flex_basis(START_CENTER_MAIN_COLUMN_WIDTH)"));
    assert!(modern_home.contains(".flex_basis(START_CENTER_SIDE_COLUMN_WIDTH)"));
    assert!(modern_home.contains(".items_stretch()"));
    assert!(modern_home.contains(".flex_grow_factor(2.0)"));
    assert!(modern_home.contains(".flex_grow()"));
    assert!(
        modern_home
            .contains("surface_panel(\"modern-home-status-panel\", cx)\n        .flex_grow()")
    );
    assert!(modern_home.contains("render_recent_connections_panel"));
    assert!(modern_home.contains("render_create_panel"));
    assert!(modern_home.contains("render_workspace_tools"));
    assert!(modern_home.contains("render_status_panel"));
    assert!(!modern_home.contains("start_center_card_slot"));
    assert!(modern_home.contains(".filter(|conn| conn.last_used_at.is_some())"));
    assert!(
        modern_home.contains("recent.sort_by_key(|conn| std::cmp::Reverse(conn.last_used_at))")
    );
    assert!(modern_home.contains("recent.truncate(8)"));
    assert!(modern_home.contains(".min_h(px(50.0))"));
    assert!(modern_home.contains(".min_h(px(140.0))"));
    assert!(!modern_home.contains(".min_h(px(210.0))"));
    assert!(modern_home.contains("home.open_connection_from_quick(&open_connection, window, cx)"));
    assert!(!modern_home.contains(".on_double_click("));
}

#[test]
fn modern_start_center_shortcuts_are_attached_to_their_actions() {
    let modern_home = include_str!("../modern_home.rs");
    let shortcuts = include_str!("../modern_home_shortcuts.rs");

    assert!(modern_home.contains("new_connection_shortcut(cx)"));
    assert!(modern_home.contains("terminal_shortcut(cx)"));
    assert!(modern_home.contains("quick_open_shortcut(cx)"));
    assert!(!modern_home.contains("render_shortcuts(cx)"));
    assert!(shortcuts.contains("fn shortcut_badge_for"));
    assert!(shortcuts.contains("action_id::HOME_QUICK_OPEN"));
    assert!(shortcuts.contains("action_id::HOME_NEW_CONNECTION"));
    assert!(shortcuts.contains("action_id::HOME_OPEN_LOCAL_TERMINAL"));
    assert!(shortcuts.contains("shortcuts_for(cx, action, &[fallback])"));
    assert!(shortcuts.contains("unwrap_or_else(|| fallback.to_string())"));
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
fn legacy_and_modern_home_layouts_are_both_kept() {
    let render = include_str!("../render.rs");
    let legacy_home = include_str!("../legacy_home.rs");
    let content = include_str!("../content.rs");
    let card = include_str!("../connection_card.rs");
    let sidebar = include_str!("../sidebar.rs");

    assert!(render.contains("self.render_legacy_home(window, cx)"));
    assert!(render.contains("self.render_modern_home(window, cx)"));
    assert!(legacy_home.contains("self.render_sidebar(window, cx)"));
    assert!(content.contains("slot.w(px(320.0)).flex_shrink_0()"));
    assert!(content.contains("slot.min_w(MODERN_HOME_CARD_MIN_WIDTH)"));
    assert!(card.contains("if legacy { px(90.0) } else { px(76.0) }"));
    assert!(!sidebar.contains("\"legacy-open-home\""));
    assert!(sidebar.contains("ConnectionType::all()"));
    assert!(sidebar.contains("this.set_selected_filter(filter, cx);"));
    assert!(sidebar.contains("legacy-home-sidebar-toggle"));
    assert!(sidebar.contains("FunctionalIcon::new(IconName::User)"));
    assert!(!sidebar.contains("ObjectIcon::new(IconName::User)"));
}

#[test]
fn legacy_ai_workbench_uses_the_original_color_icon() {
    let sidebar = include_str!("../sidebar.rs");
    let ai_entry = sidebar
        .split(".when(show_ai_workbench")
        .nth(1)
        .and_then(|source| source.split(".when(show_team").next())
        .expect("legacy AI workbench sidebar entry");

    assert!(ai_entry.contains("\"legacy-open-ai-workbench\""));
    assert!(ai_entry.contains("IconName::AI,"));
    assert!(!ai_entry.contains("IconName::AILine"));
}

#[test]
fn modern_home_cards_are_small_and_fill_each_row() {
    let home = include_str!("../../home_tab.rs");
    let content = include_str!("../content.rs");

    assert!(home.contains("MODERN_HOME_CARD_MIN_WIDTH: gpui::Pixels = px(220.0)"));
    assert!(home.contains("MODERN_HOME_CARD_MAX_WIDTH: gpui::Pixels = px(260.0)"));
    assert!(
        content
            .matches(".flex_basis(MODERN_HOME_CARD_MIN_WIDTH)")
            .count()
            >= 3
    );
    assert!(content.matches(".flex_grow()").count() >= 3);
}

#[test]
fn collapsed_modern_sidebar_lets_home_content_use_the_full_width() {
    let content = include_str!("../content.rs");
    let app = include_str!("../../onetcli_app.rs");

    assert!(content.contains("center_modern_content"));
    assert!(content.contains("!legacy && self.persistent_sidebar_expanded"));
    assert!(content.contains(".when(center_modern_content"));
    assert!(app.contains("home.set_persistent_sidebar_expanded(expanded, cx)"));
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
