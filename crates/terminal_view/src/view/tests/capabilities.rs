fn function_region<'a>(source: &'a str, start: &str, end: &str) -> &'a str {
    source
        .split(start)
        .nth(1)
        .and_then(|source| source.split(end).next())
        .unwrap_or_else(|| panic!("missing function region between `{start}` and `{end}`"))
}

#[test]
fn playback_never_registers_or_uses_ssh_broadcast_input() {
    let source = include_str!("../initialization.rs");
    let registration = function_region(
        source,
        "fn register_broadcast_input",
        "fn unregister_broadcast_input",
    );
    assert!(
        registration.contains("live_ssh_feature_supported(terminal.live_connection_kind())"),
        "broadcast registration must authorize the live SSH capability, not source metadata"
    );

    let broadcast = function_region(
        source,
        "fn broadcast_user_input",
        "fn refresh_public_mcp_session",
    );
    let guard = broadcast
        .find("self.is_live_ssh_terminal(cx)")
        .expect("broadcast sender must have a live SSH guard");
    let delivery = broadcast
        .find("deliveries_from")
        .expect("broadcast delivery lookup should remain");
    assert!(guard < delivery, "the live SSH guard must precede delivery");
}

#[test]
fn playback_reconnect_is_rejected_before_connection_storage_access() {
    let source = include_str!("../preferences.rs");
    let reconnect = function_region(source, "pub fn reconnect", "pub fn sync_terminal_path");
    let guard = reconnect
        .find("if !self.accepts_live_terminal_input(cx)")
        .expect("reconnect must reject playback");
    let load = reconnect
        .find("resolve_ssh_reconnect_source")
        .expect("SSH reconnect source resolution should remain");
    assert!(
        guard < load,
        "playback must be rejected before reading connection storage"
    );
}

#[test]
fn playback_duplicate_is_rejected_at_view_and_workspace_execution_boundaries() {
    let view_tab = include_str!("../tab_content.rs");
    let workspace_tab = include_str!("../../workspace/tab_content.rs");
    let support = include_str!("../workspace_support.rs");

    assert!(
        function_region(view_tab, "fn duplicate(", "fn try_close")
            .contains("if !self.duplicate_supported(cx)")
    );
    assert!(
        function_region(workspace_tab, "fn duplicate(", "fn try_close")
            .contains("if !self.active_pane().read(cx).duplicate_supported(cx)")
    );
    assert!(support.contains("pub(crate) fn duplicate_supported(&self, cx: &App) -> bool"));
    assert!(support.contains("self.terminal.read(cx).live_connection_kind()"));
}
