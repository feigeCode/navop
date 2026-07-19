const STRATEGY_SOURCE: &str = include_str!("home_strategy.rs");
const FORWARDING_SOURCE: &str = include_str!("../home_tab/forwarding.rs");

#[test]
fn port_forwarding_strategy_opens_management_tab() {
    assert!(STRATEGY_SOURCE.contains("open_port_forwarding_tab"));
    assert!(!STRATEGY_SOURCE.contains("home.open_port_forwarding("));
}

#[test]
fn home_page_no_longer_starts_port_forwarding_on_gpui_executor() {
    assert!(!FORWARDING_SOURCE.contains("runtime.start_local(connection_id, request).await"));
    assert!(!FORWARDING_SOURCE.contains("runtime.start_dynamic(connection_id, request).await"));
}
