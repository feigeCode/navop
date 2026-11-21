use super::session_status;
use ssh::X11ForwardingStatus;

#[test]
fn missing_session_snapshot_stays_hidden() {
    assert!(session_status(true, None).is_none());
}

#[test]
fn disabled_x11_never_occupies_the_terminal_footer() {
    let ready = X11ForwardingStatus::External {
        server: "127.0.0.1:0".into(),
    };
    assert!(session_status(false, Some(ready)).is_none());
    assert!(
        session_status(
            true,
            Some(X11ForwardingStatus::Disabled {
                reason: "Vulkan 支持不足".into(),
            }),
        )
        .is_none()
    );
}

#[test]
fn session_status_uses_the_supplied_ssh_session_snapshot() {
    let status = session_status(
        true,
        Some(X11ForwardingStatus::External {
            server: "127.0.0.1:0".into(),
        }),
    )
    .expect("启用且成功的 X11 会话必须可见");
    assert!(status.label.contains("127.0.0.1:0"));
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
