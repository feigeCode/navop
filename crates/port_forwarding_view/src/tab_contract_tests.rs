const TAB_SOURCE: &str = include_str!("tab.rs");
const CLOSE_SOURCE: &str = include_str!("tab_close.rs");
const RENDER_SOURCE: &str = include_str!("tab_render.rs");

#[test]
fn port_forwarding_tab_uses_application_tokio_runtime() {
    assert!(TAB_SOURCE.contains("Tokio::spawn_result"));
    assert!(!TAB_SOURCE.contains("background_spawn"));
}

#[test]
fn port_forwarding_tab_implements_close_confirmation() {
    assert!(TAB_SOURCE.contains("fn try_close"));
    assert!(TAB_SOURCE.contains("stop_forwarding"));
    assert!(CLOSE_SOURCE.contains("oneshot::channel"));
}

#[test]
fn failed_port_forwarding_can_rebuild_and_retry_request() {
    assert!(TAB_SOURCE.contains("fn retry_forwarding"));
    assert!(TAB_SOURCE.contains("start_in_flight"));
    assert!(TAB_SOURCE.contains("pending_close"));
}

#[test]
fn malformed_saved_connections_do_not_panic_the_ui() {
    assert!(!TAB_SOURCE.contains(".unwrap()"));
}

#[test]
fn activity_history_scrolls_inside_a_bounded_panel() {
    assert!(RENDER_SOURCE.contains(".max_h(px(ACTIVITY_MAX_HEIGHT))"));
    assert!(RENDER_SOURCE.contains(".overflow_y_scrollbar()"));
}
