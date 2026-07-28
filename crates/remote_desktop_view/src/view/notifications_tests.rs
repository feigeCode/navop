use remote_desktop::{
    RemoteDesktopFailure, RemoteDesktopProtocol, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};

use super::{
    localized_clipboard_files_received_for_locale, localized_failure_message_for_locale,
    localized_reconnect_notification_for_locale, localized_session_taken_over_for_locale,
    localized_vnc_clipboard_ascii_warning_for_locale,
};

#[test]
fn reconnect_notification_is_localized_without_backend_details() {
    let notification = localized_reconnect_notification_for_locale(
        "en",
        RemoteDesktopProtocol::Rdp,
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::DisplayUpdate,
            delay_secs: Some(1),
        },
    );

    assert_eq!(
        "RDP disconnected: display update error. Reconnecting in 1s",
        notification
    );
    assert!(!notification.contains("VNC"));
    assert!(!notification.contains("/Users/"));
    assert!(!notification.contains(".cargo/git/checkouts"));
}

#[test]
fn reconnect_notification_supports_vnc_and_traditional_chinese() {
    let notification = localized_reconnect_notification_for_locale(
        "zh-CN",
        RemoteDesktopProtocol::Vnc,
        RemoteDesktopReconnect {
            reason: RemoteDesktopReconnectReason::ConnectionLost,
            delay_secs: Some(2),
        },
    );
    assert_eq!(
        "VNC 连接已断开：连接丢失。将在 2 秒后重新连接",
        notification
    );

    let manual = RemoteDesktopReconnect {
        reason: RemoteDesktopReconnectReason::Manual,
        delay_secs: None,
    };
    assert_eq!(
        "正在重新連線 RDP 工作階段",
        localized_reconnect_notification_for_locale("zh-HK", RemoteDesktopProtocol::Rdp, manual)
    );
}

#[test]
fn clipboard_notifications_are_localized_for_all_supported_locales() {
    assert_eq!(
        "VNC clipboard currently supports ASCII text only",
        localized_vnc_clipboard_ascii_warning_for_locale("en")
    );
    assert_eq!(
        "VNC 剪贴板当前仅支持 ASCII 文本",
        localized_vnc_clipboard_ascii_warning_for_locale("zh-CN")
    );
    assert_eq!(
        "VNC 剪貼簿目前僅支援 ASCII 文字",
        localized_vnc_clipboard_ascii_warning_for_locale("zh-HK")
    );
    assert_eq!(
        "Received 2 item(s) from the remote clipboard",
        localized_clipboard_files_received_for_locale("en", 2)
    );
    assert_eq!(
        "已从远端剪贴板接收 2 个项目",
        localized_clipboard_files_received_for_locale("zh-CN", 2)
    );
    assert_eq!(
        "已從遠端剪貼簿接收 2 個項目",
        localized_clipboard_files_received_for_locale("zh-HK", 2)
    );
}

#[test]
fn notifications_are_deferred_and_auto_hidden() {
    let source = include_str!("notifications.rs");

    assert!(source.contains("window.defer(cx"));
    assert!(source.contains(".autohide(true)"));
    assert!(source.contains("Notification::info(message)"));
    assert!(source.contains("RemoteDesktopReconnectNotification"));
    assert!(!source.contains("localized_reconnect_status"));
}

#[test]
fn connection_failures_are_localized_without_backend_diagnostics() {
    assert_eq!(
        "身份验证失败，请检查用户名、密码和域。",
        localized_failure_message_for_locale("zh-CN", &RemoteDesktopFailure::AuthenticationFailed,)
    );
    assert_eq!(
        "无法连接远程主机，请检查主机地址、端口和网络。",
        localized_failure_message_for_locale("zh-CN", &RemoteDesktopFailure::HostUnreachable)
    );
    assert_eq!(
        "Remote desktop connection failed. Check the connection settings and try again.",
        localized_failure_message_for_locale("en", &RemoteDesktopFailure::ConnectionFailed)
    );

    for failure in [
        RemoteDesktopFailure::AuthenticationFailed,
        RemoteDesktopFailure::HostUnreachable,
        RemoteDesktopFailure::ConnectionFailed,
    ] {
        let message = localized_failure_message_for_locale("en", &failure);
        assert!(!message.contains("/Users/"));
        assert!(!message.contains(".cargo/git/checkouts"));
        assert!(!message.contains("connector.rs"));
        assert!(!message.contains("CredSSP @"));
    }
}

#[test]
fn session_takeover_notification_tells_the_user_that_the_tab_was_closed() {
    assert_eq!(
        "另一位用户已连接到远程服务器，当前 RDP 会话已断开，页签已自动关闭。",
        localized_session_taken_over_for_locale("zh-CN")
    );
}
