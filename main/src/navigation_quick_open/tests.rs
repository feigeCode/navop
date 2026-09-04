use super::*;

#[test]
fn connection_navigation_partition_is_complete_and_stable() {
    let visible = visible_connection_types();
    let overflow = overflow_connection_types();
    let combined = visible
        .iter()
        .chain(overflow.iter())
        .copied()
        .collect::<Vec<_>>();

    assert_eq!(
        visible,
        vec![
            ConnectionType::All,
            ConnectionType::SshSftp,
            ConnectionType::Database,
            ConnectionType::Redis,
            ConnectionType::MongoDB,
            ConnectionType::Mqtt,
            ConnectionType::Serial,
            ConnectionType::Telnet,
        ]
    );
    assert_eq!(
        overflow,
        vec![
            ConnectionType::PortForwarding,
            ConnectionType::Rdp,
            ConnectionType::Vnc,
            ConnectionType::Extension,
        ]
    );
    assert_eq!(combined, ConnectionType::all());
    for connection_type in ConnectionType::all() {
        assert_eq!(
            is_overflow_connection_type(connection_type),
            overflow.contains(&connection_type)
        );
    }
}

#[test]
fn application_navigation_partition_preserves_optional_entries() {
    for (show_ai, show_team) in [(false, false), (false, true), (true, false), (true, true)] {
        let availability = NavigationAvailability {
            show_ai_workbench: show_ai,
            show_team,
        };
        let leading = leading_navigation_applications(availability);
        let overflow = overflow_navigation_applications();
        let combined = leading
            .iter()
            .chain(overflow.iter())
            .copied()
            .collect::<Vec<_>>();
        let mut expected = Vec::new();
        if show_ai {
            expected.push(NavigationApplication::AiWorkbench);
        }
        if show_team {
            expected.push(NavigationApplication::Team);
        }
        expected.push(NavigationApplication::Notes);
        #[cfg(feature = "api-testing")]
        expected.push(NavigationApplication::ApiTesting);
        expected.extend([
            NavigationApplication::Extensions,
            NavigationApplication::SessionLogs,
            NavigationApplication::CredentialVault,
            NavigationApplication::JsonFormatter,
        ]);

        assert_eq!(combined, expected);
    }
}

#[test]
fn extensions_stays_visible_while_json_formatter_moves_to_more_applications() {
    let availability = NavigationAvailability {
        show_ai_workbench: false,
        show_team: false,
    };

    assert_eq!(
        leading_navigation_applications(availability),
        vec![
            NavigationApplication::Notes,
            #[cfg(feature = "api-testing")]
            NavigationApplication::ApiTesting,
            NavigationApplication::Extensions,
        ]
    );
    assert_eq!(
        overflow_navigation_applications(),
        vec![
            NavigationApplication::SessionLogs,
            NavigationApplication::CredentialVault,
            NavigationApplication::JsonFormatter,
        ]
    );
}

#[test]
fn navigation_item_search_is_case_insensitive() {
    let item = NavigationQuickOpenItem::application(NavigationApplication::SessionLogs);

    assert!(delegate::item_matches_query(&item, "session"));
    assert!(delegate::item_matches_query(&item, "LOGS"));
    assert!(!delegate::item_matches_query(&item, "notes"));
}

#[test]
fn connection_quick_open_marks_only_the_current_overflow_filter() {
    let request =
        NavigationQuickOpenRequest::connections(ConnectionType::Rdp, Rc::new(|_, _, _| {}));

    assert_eq!(
        request
            .items
            .iter()
            .filter(|item| item.selected)
            .map(|item| item.target)
            .collect::<Vec<_>>(),
        vec![NavigationTarget::Connection(ConnectionType::Rdp)]
    );
}

#[test]
fn quick_open_renders_business_selection_separately_from_keyboard_selection() {
    let delegate = include_str!("delegate.rs");

    assert!(delegate.contains(".confirmed(item.selected)"));
    assert!(delegate.contains(".check_icon(IconName::Check)"));
}

#[test]
fn quick_open_uses_monochrome_navigation_icons_without_recoloring_color_assets() {
    let delegate = include_str!("delegate.rs");
    let legacy_sidebar = include_str!("../home_tab/sidebar_navigation.rs");
    let filter_bar = include_str!("../persistent_connection_sidebar/filter_bar.rs");
    let modern_home = include_str!("../home_tab/modern_home.rs");

    assert!(delegate.contains("connection_type_navigation_icon("));
    assert!(delegate.contains("ConnectionVisualSize::Inline"));
    assert!(!delegate.contains("connection_type.icon()"));
    assert!(delegate.contains("Icon::new(application_icon(application))"));
    assert!(delegate.contains(".mono()"));
    assert!(!delegate.contains(".color()"));
    assert!(legacy_sidebar.contains("Icon::new(icon).mono()"));
    assert!(!legacy_sidebar.contains("Icon::new(icon).color()"));
    assert!(filter_bar.contains("Icon::new(IconName::Filter)"));
    assert!(filter_bar.contains("connection_type_rail_icon(filter)"));
    assert!(modern_home.contains("Icon::new(application.icon())\n                        .mono()"));
    for color_icon in [
        "IconName::AI,",
        "IconName::TeamColor",
        "IconName::NotesColor",
        "IconName::ExtensionsColor",
        "IconName::SettingColor",
    ] {
        assert!(!legacy_sidebar.contains(color_icon));
    }
}

#[test]
fn quick_open_layout_is_compact_and_gives_icons_a_consistent_container() {
    let dialog = include_str!("../navigation_quick_open.rs");
    let delegate = include_str!("delegate.rs");

    assert!(dialog.contains("const DIALOG_WIDTH: gpui::Pixels = px(440.0);"));
    assert!(dialog.contains(".id(\"navigation-quick-open-dialog\")"));
    assert!(dialog.contains(".pb_2()"));
    assert!(delegate.contains("const ICON_CONTAINER_SIZE: gpui::Pixels = px(32.0);"));
    assert!(delegate.contains(".size(ICON_CONTAINER_SIZE)"));
    assert!(delegate.contains(".justify_center()"));
    assert!(delegate.contains(".bg(cx.theme().muted.opacity(0.45))"));
}
