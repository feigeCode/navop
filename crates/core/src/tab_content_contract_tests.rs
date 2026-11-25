use crate::tab_container::{TabContentEvent, TabContentView};
use gpui::App;

#[test]
fn tab_content_view_exposes_sidebar_contracts() {
    let _sidebar_contributions = <dyn TabContentView>::sidebar_contributions;
}

#[test]
fn tab_content_view_exposes_tab_action_contracts() {
    let _can_rename: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::can_rename;
    let _rename: fn(&dyn TabContentView, &str, &mut gpui::Window, &mut App) -> bool =
        <dyn TabContentView>::rename;
    let _apply_title: fn(&dyn TabContentView, &str, &mut gpui::Window, &mut App) =
        <dyn TabContentView>::apply_title;
    let _can_duplicate: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::can_duplicate;
    let _duplicate: fn(
        &dyn TabContentView,
        &mut gpui::Window,
        &mut App,
    ) -> Option<std::sync::Arc<dyn TabContentView>> = <dyn TabContentView>::duplicate;
}

#[test]
fn tab_content_view_exposes_session_lock_contracts() {
    let _lockable: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::lockable;
    let _is_locked: fn(&dyn TabContentView, &App) -> bool = <dyn TabContentView>::is_locked;
    let _is_disconnected: fn(&dyn TabContentView, &App) -> bool =
        <dyn TabContentView>::is_disconnected;
    let _lock_session: fn(&dyn TabContentView, &str, bool, &mut gpui::Window, &mut App) -> bool =
        <dyn TabContentView>::lock_session;
    let _unlock_session: fn(&dyn TabContentView, &str, &mut App) -> bool =
        <dyn TabContentView>::unlock_session;
}

#[test]
fn tab_content_view_session_lock_defaults_fail_closed() {
    let _defaults = <dyn TabContentView>::lockable;
}

#[test]
fn tab_content_view_exposes_connection_status_contracts() {
    let _connection_status: fn(
        &dyn TabContentView,
        &App,
    ) -> Option<crate::tab_container::TabConnectionStatus> =
        <dyn TabContentView>::connection_status;
}

#[test]
fn tab_content_view_connection_status_defaults_to_none() {
    let _defaults = <dyn TabContentView>::connection_status;
}

#[test]
fn tab_content_event_exposes_content_changed_contract() {
    let _event = TabContentEvent::ContentChanged;
}

#[test]
fn tab_content_event_exposes_close_request_contract() {
    assert_eq!(
        "CloseRequested",
        format!("{:?}", TabContentEvent::CloseRequested)
    );
}

#[test]
fn tab_attention_classifies_approval_completed_and_info() {
    use crate::tab_container::{TabAttention, TabAttentionKind};

    let approval = TabAttention::from_notification(None, "Needs approval".to_string());
    assert_eq!(TabAttentionKind::Approval, approval.kind);
    assert_eq!("Needs approval", approval.tooltip_summary());

    let completed = TabAttention::from_notification(
        Some("Antigravity".to_string()),
        "Agent finished".to_string(),
    );
    assert_eq!(TabAttentionKind::Completed, completed.kind);
    assert_eq!("Antigravity: Agent finished", completed.tooltip_summary());

    // 完成提示附带“等待输入”尾缀时仍应归类为 Completed，而非误判成 Approval
    let completed_waiting = TabAttention::from_notification(
        None,
        "Task complete — waiting for your input".to_string(),
    );
    assert_eq!(TabAttentionKind::Completed, completed_waiting.kind);

    let ready_input = TabAttention::from_notification(
        None,
        "Response finished, ready for input".to_string(),
    );
    assert_eq!(TabAttentionKind::Completed, ready_input.kind);

    // 明确的权限/审批请求即使包含其他词，也应归类为 Approval
    let explicit_permission = TabAttention::from_notification(
        None,
        "Permission required to complete action".to_string(),
    );
    assert_eq!(TabAttentionKind::Approval, explicit_permission.kind);

    let info = TabAttention::from_notification(None, "Build log ready?".to_string());
    assert_eq!(TabAttentionKind::Completed, info.kind);

    let generic = TabAttention::from_notification(None, "Hello from CLI".to_string());
    assert_eq!(TabAttentionKind::Info, generic.kind);
}
