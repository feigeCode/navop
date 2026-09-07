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
    // 常驻侧栏团队标识与主页卡片徽标同一中性样式（muted 底），且不带
    // 独立 hitbox（独立元素会截获行 hover）。
    assert!(row_parts.contains("cx.theme().muted"));
    assert!(!row_parts.contains("persistent-team-"));
    assert!(!row_parts.contains("cx.theme().primary"));
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
fn home_sidebar_refinement_contracts() {
    let nav = include_str!("../sidebar_navigation.rs");
    let sidebar = include_str!("../sidebar.rs");
    let card = include_str!("../connection_card.rs");
    let account = include_str!("../account_menu.rs");
    let applications = include_str!("../../navigation_applications.rs");

    // 导航行本地 helper：hover 与选中分离，selected+hover 不被普通 hover 覆盖
    assert!(nav.contains("fn home_nav_row"));
    assert!(nav.contains("hover_bg"));
    assert!(nav.contains("active_bg"));
    assert!(nav.contains(".focus_visible("));
    assert!(nav.contains(".on_key_down("));
    assert!(!nav.contains("SidebarMenuItem::new"));
    // 功能图标统一线性单色；Home 用自有线稿资源；会话日志用 SquareTerminal 线性资源
    assert!(nav.contains("NAVOP_HOME_LINE_ICON"));
    assert!(nav.contains(".mono()"));
    assert!(applications.contains("IconName::SquareTerminal"));
    assert!(!applications.contains("IconName::Terminal,"));
    // 侧栏右边线读 sidebar_border，非通用 border
    assert!(sidebar.contains("sidebar_border"));
    assert!(!sidebar.contains("cx.theme().border"));
    // 卡片：非选中 hover 才生效，selected+hover 保留选中组合
    assert!(card.contains(".when(!is_selected, |this|"));
    assert!(!card.contains("hover_border"));
    // 账户 fallback 中性化：不用 Avatar hash 自动色
    assert!(account.contains("neutral_avatar_for_url"));
    assert!(account.contains("IconName::CircleUser"));
    assert!(!account.contains("Avatar::new()\n                .name("));
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
fn home_batch_mode_is_shared_across_card_list_and_tree_layouts() {
    let home = include_str!("../../home_tab.rs");
    let toolbar = include_str!("../toolbar.rs");
    let batch_bar = include_str!("../batch_bar.rs");
    let home_layout = include_str!("../home_layout.rs");
    let card = include_str!("../connection_card.rs");
    let list = include_str!("../connection_list.rs");
    let selection = include_str!("../connection_selection.rs");

    // 选择状态由 HomePage 持有，三布局共享；Tree 布局不再重复渲染主页批量条
    assert!(home.contains("connection_selection"));
    assert!(selection.contains("fn set_batch_mode"));
    assert!(toolbar.contains("home-batch-toggle"));
    assert!(toolbar.contains("IconName::ListChecks"));
    assert!(home_layout.contains("render_batch_bar"));
    assert!(home_layout.contains("ConnectionLayout::Tree"));
    // 卡片与列表在批量模式下渲染勾选框并按修饰键做范围/多选
    for source in [card, list] {
        assert!(source.contains("connection_selection_checkbox"));
        assert!(source.contains("ConnectionSelectionMode::Range"));
        assert!(source.contains("ConnectionSelectionMode::Toggle"));
    }
    // 批量条提供全选可见/移动/删除/退出
    assert!(batch_bar.contains("home-select-visible-connections"));
    assert!(batch_bar.contains("home-move-selected-connections"));
    assert!(batch_bar.contains("home-delete-selected-connections"));
    assert!(batch_bar.contains("home-exit-batch-connections"));
}

#[test]
fn embedded_tree_reuses_home_search_and_filter_without_own_search_box() {
    let tree = include_str!("../../persistent_connection_sidebar/tree.rs");
    let implementation = tree.split("#[cfg(test)]").next().unwrap();

    // 树内搜索框与树头部都仅在非嵌入（停靠/浮动）时渲染
    for marker in [
        "tree.child(self.render_tree_search(palette, cx))",
        "tree.child(self.render_tree_header(palette, macos_titlebar_inset, cx))",
    ] {
        let render_at = implementation.find(marker).expect("渲染点存在");
        let mut window_start = render_at.saturating_sub(160);
        while !implementation.is_char_boundary(window_start) {
            window_start -= 1;
        }
        assert!(
            implementation[window_start..render_at].contains("!self.home_embedded"),
            "{marker} 应由 !home_embedded 门控"
        );
    }
    // 嵌入时搜索词与类型筛选直接来自主页工具栏
    assert!(implementation.contains("home.search_query.read(cx)"));
    assert!(implementation.contains("home.selected_filter"));
}

#[test]
fn home_toolbar_groups_utility_controls_into_one_container() {
    let toolbar = include_str!("../toolbar.rs");

    // 筛选/排序/布局/刷新/批量收纳进同一 muted 分组容器
    let group = toolbar
        .find(".bg(cx.theme().muted)")
        .map(|_| toolbar.contains("render_batch_toggle(cx)"));
    assert!(group.unwrap_or(false));
    assert!(toolbar.contains("render_home_type_filter(window, cx)"));
    // 「全部类型」不再使用星号图标
    assert!(toolbar.contains("IconName::Apps"));
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
