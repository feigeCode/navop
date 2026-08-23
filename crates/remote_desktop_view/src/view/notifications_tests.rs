use remote_desktop::{
    RemoteDesktopFailure, RemoteDesktopProtocol, RemoteDesktopReconnect,
    RemoteDesktopReconnectReason,
};
#[cfg(feature = "windows-native-rdp")]
use windows_rdp_host::{WindowsRdpDiagnosticCategory, WindowsRdpLogonErrorKind};

#[cfg(feature = "windows-native-rdp")]
use super::super::native_events::NativeRdpNotificationRequest;
#[cfg(feature = "windows-native-rdp")]
use super::localized_windows_native_rdp_notification_for_locale;
use super::{
    localized_clipboard_files_received_for_locale, localized_clipboard_install_failed_for_locale,
    localized_failure_message_for_locale, localized_reconnect_notification_for_locale,
    localized_session_taken_over_for_locale, localized_vnc_clipboard_ascii_warning_for_locale,
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
    assert_eq!(
        "Could not place the received files on the system clipboard",
        localized_clipboard_install_failed_for_locale("en")
    );
    assert_eq!(
        "无法将接收的文件写入系统剪贴板",
        localized_clipboard_install_failed_for_locale("zh-CN")
    );
    assert_eq!(
        "無法將接收的檔案寫入系統剪貼簿",
        localized_clipboard_install_failed_for_locale("zh-HK")
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

#[cfg(feature = "windows-native-rdp")]
#[test]
fn native_rdp_notifications_are_localized_by_owned_classification() {
    let cases = [
        (
            NativeRdpNotificationRequest::FatalError { generation: 7 },
            [
                "Windows native RDP encountered a fatal error and disconnected.",
                "Windows 原生 RDP 遇到严重错误并已断开连接。",
                "Windows 原生 RDP 遇到嚴重錯誤並已中斷連線。",
            ],
        ),
        (
            NativeRdpNotificationRequest::LogonError {
                generation: 7,
                kind: WindowsRdpLogonErrorKind::BadCredentials,
            },
            [
                "Windows native RDP sign-in failed. Check the username, password, and domain.",
                "Windows 原生 RDP 登录失败，请检查用户名、密码和域。",
                "Windows 原生 RDP 登入失敗，請檢查使用者名稱、密碼和網域。",
            ],
        ),
        (
            NativeRdpNotificationRequest::LogonError {
                generation: 7,
                kind: WindowsRdpLogonErrorKind::PasswordChangeRequired,
            },
            [
                "Windows native RDP requires the account password to be changed before sign-in.",
                "Windows 原生 RDP 要求先更改账户密码，然后才能登录。",
                "Windows 原生 RDP 要求先變更帳戶密碼，然後才能登入。",
            ],
        ),
        (
            NativeRdpNotificationRequest::LogonError {
                generation: 7,
                kind: WindowsRdpLogonErrorKind::Unknown,
            },
            [
                "The remote server rejected the Windows native RDP sign-in.",
                "远程服务器拒绝了 Windows 原生 RDP 登录。",
                "遠端伺服器拒絕了 Windows 原生 RDP 登入。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::Authentication,
            },
            [
                "Windows native RDP disconnected because authentication failed.",
                "Windows 原生 RDP 因身份验证失败而断开连接。",
                "Windows 原生 RDP 因身份驗證失敗而中斷連線。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::CertificateOrSecurity,
            },
            [
                "Windows native RDP disconnected because the certificate or security negotiation failed.",
                "Windows 原生 RDP 因证书或安全协商失败而断开连接。",
                "Windows 原生 RDP 因憑證或安全性協商失敗而中斷連線。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::Gateway,
            },
            [
                "Windows native RDP disconnected because the Remote Desktop Gateway failed.",
                "Windows 原生 RDP 因远程桌面网关失败而断开连接。",
                "Windows 原生 RDP 因遠端桌面閘道失敗而中斷連線。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::ServerPolicy,
            },
            [
                "Windows native RDP was disconnected by remote server policy.",
                "Windows 原生 RDP 已被远程服务器策略断开。",
                "Windows 原生 RDP 已被遠端伺服器原則中斷。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::Network,
            },
            [
                "Windows native RDP disconnected because the network connection was lost.",
                "Windows 原生 RDP 因网络连接丢失而断开。",
                "Windows 原生 RDP 因網路連線遺失而中斷。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::NativeUnavailable,
            },
            [
                "Windows native RDP disconnected because the Windows RDP component became unavailable.",
                "Windows 原生 RDP 因 Windows RDP 组件不可用而断开连接。",
                "Windows 原生 RDP 因 Windows RDP 元件不可用而中斷連線。",
            ],
        ),
        (
            NativeRdpNotificationRequest::Disconnected {
                generation: 7,
                category: WindowsRdpDiagnosticCategory::Unknown,
            },
            [
                "Windows native RDP disconnected unexpectedly.",
                "Windows 原生 RDP 意外断开连接。",
                "Windows 原生 RDP 意外中斷連線。",
            ],
        ),
    ];

    for (request, expected) in cases {
        assert_eq!(
            expected[0],
            localized_windows_native_rdp_notification_for_locale("en", request)
        );
        assert_eq!(
            expected[1],
            localized_windows_native_rdp_notification_for_locale("zh-CN", request)
        );
        assert_eq!(
            expected[2],
            localized_windows_native_rdp_notification_for_locale("zh-HK", request)
        );
    }
}

#[cfg(feature = "windows-native-rdp")]
#[test]
fn native_rdp_notifications_do_not_expose_native_diagnostics() {
    let requests = [
        NativeRdpNotificationRequest::FatalError { generation: 1 },
        NativeRdpNotificationRequest::LogonError {
            generation: 1,
            kind: WindowsRdpLogonErrorKind::BadCredentials,
        },
        NativeRdpNotificationRequest::Disconnected {
            generation: 1,
            category: WindowsRdpDiagnosticCategory::Unknown,
        },
    ];

    for request in requests {
        for locale in ["en", "zh-CN", "zh-HK"] {
            let message = localized_windows_native_rdp_notification_for_locale(locale, request);
            for forbidden in [
                "HRESULT",
                "IMsRdpClient",
                "code=",
                "0x",
                "/Users/",
                ".cargo/git/checkouts",
                "password=",
                "username=",
            ] {
                assert!(
                    !message.contains(forbidden),
                    "notification {message:?} contains forbidden detail {forbidden:?}"
                );
            }
        }
    }
}

#[cfg(feature = "windows-native-rdp")]
#[test]
fn native_rdp_notification_source_is_deferred_and_generation_checked() {
    let source = include_str!("notifications.rs");
    let view_source = include_str!("../view.rs");

    assert!(source.contains("RemoteDesktopNativeRdpNotification"));
    assert!(source.contains("window.defer(cx"));
    assert!(source.contains("Notification::error(message)"));
    assert!(source.contains(".autohide(true)"));
    assert!(view_source.contains("pending_windows_native_notifications"));
    assert!(view_source.contains("request.generation()"));
}
