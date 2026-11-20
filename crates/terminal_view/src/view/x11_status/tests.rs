use super::session_status;
use ssh::X11ForwardingStatus;

#[test]
fn missing_session_snapshot_stays_hidden() {
    assert!(session_status(None).is_none());
}

#[test]
fn session_status_uses_the_supplied_ssh_session_snapshot() {
    let status = session_status(Some(X11ForwardingStatus::External {
        server: "127.0.0.1:0".into(),
    }))
    .expect("已有会话状态必须可见");
    assert!(status.label.contains("127.0.0.1:0"));
}

#[test]
fn local_disabled_does_not_gain_remote_rejection_advice() {
    let status = session_status(Some(X11ForwardingStatus::Disabled {
        reason: "Vulkan 支持不足".into(),
    }))
    .expect("明确的本地错误必须可见");
    assert!(status.label.contains("Vulkan 支持不足"));
    assert!(!status.label.contains("X11Forwarding"));
    assert!(!status.label.contains("x11-req"));
}

#[test]
fn session_footer_reads_the_terminal_scoped_getter() {
    let source = include_str!("../x11_status.rs");
    assert!(source.contains(".x11_status()"));
    assert!(!source.contains("current_status()"));
}

#[test]
fn session_status_copy_exists_in_all_three_languages() {
    for language in ["en", "zh-CN", "zh-HK"] {
        rust_i18n::set_locale(language);
        for key in [
            "TerminalX11.external",
            "TerminalX11.embedded",
            "TerminalX11.disabled",
        ] {
            assert_ne!(key, rust_i18n::t!(key), "{language} 缺少文案: {key}");
        }
    }
    rust_i18n::set_locale("en");
}
