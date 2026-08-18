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
        ]
    );
    assert_eq!(
        overflow,
        vec![
            ConnectionType::Serial,
            ConnectionType::Telnet,
            ConnectionType::PortForwarding,
            ConnectionType::Rdp,
            ConnectionType::Vnc,
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
        let trailing = trailing_navigation_applications();
        let combined = leading
            .iter()
            .chain(overflow.iter())
            .chain(trailing.iter())
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
            NavigationApplication::JsonFormatter,
            NavigationApplication::SessionLogs,
            NavigationApplication::CredentialVault,
            NavigationApplication::Extensions,
            NavigationApplication::Settings,
        ]);

        assert_eq!(combined, expected);
    }
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
