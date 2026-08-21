use super::*;

#[test]
fn live_ssh_maps_connection_state_to_tab_badge() {
    use one_core::tab_container::TabConnectionStatus;
    assert_eq!(
        map_connection_status(true, &ConnectionState::Connected),
        Some(TabConnectionStatus::Connected)
    );
    assert_eq!(
        map_connection_status(true, &ConnectionState::Connecting),
        Some(TabConnectionStatus::Connecting)
    );
    assert_eq!(
        map_connection_status(true, &ConnectionState::Disconnected { error: None }),
        Some(TabConnectionStatus::Disconnected)
    );
    assert_eq!(
        map_connection_status(
            true,
            &ConnectionState::Disconnected {
                error: Some("boom".to_string()),
            },
        ),
        Some(TabConnectionStatus::Disconnected)
    );
}

#[test]
fn read_only_terminals_surface_no_status_badge() {
    assert_eq!(
        map_connection_status(false, &ConnectionState::Connected),
        None
    );
    assert_eq!(
        map_connection_status(false, &ConnectionState::Connecting),
        None
    );
    assert_eq!(
        map_connection_status(false, &ConnectionState::Disconnected { error: None }),
        None
    );
}
