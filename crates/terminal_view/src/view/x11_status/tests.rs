use super::take_session_notice;
use ssh::X11ForwardingStatus;

#[test]
fn missing_disabled_and_failed_sessions_have_no_notice() {
    let mut notified = false;
    let ready = X11ForwardingStatus::External {
        server: "127.0.0.1:0".into(),
    };
    assert!(take_session_notice(&mut notified, false, Some(&ready)).is_none());
    assert!(take_session_notice(&mut notified, true, None).is_none());
    assert!(
        take_session_notice(
            &mut notified,
            true,
            Some(&X11ForwardingStatus::Disabled {
                reason: "Vulkan 支持不足".into(),
            }),
        )
        .is_none()
    );
}

#[test]
fn successful_session_is_notified_once_per_ready_transition() {
    let mut notified = false;
    let ready = X11ForwardingStatus::External {
        server: "127.0.0.1:0".into(),
    };
    let notice = take_session_notice(&mut notified, true, Some(&ready))
        .expect("成功连接必须给出一次轻提示");
    assert!(notice.contains("127.0.0.1:0"));
    assert!(take_session_notice(&mut notified, true, Some(&ready)).is_none());

    assert!(take_session_notice(&mut notified, true, None).is_none());
    assert!(take_session_notice(&mut notified, true, Some(&ready)).is_some());
}

#[test]
fn session_notice_reads_only_the_terminal_backend_result() {
    let source = include_str!("../x11_status.rs");
    assert!(source.contains(".x11_status()"));
    assert!(!source.contains("current_status()"));
    assert!(source.contains(".autohide(true)"));

    let terminal_source = include_str!("../../../../terminal/src/terminal.rs");
    let getter = terminal_source
        .split("pub fn x11_status(&self)")
        .nth(1)
        .and_then(|tail| tail.split("\n    }").next())
        .expect("Terminal::x11_status 应存在");
    assert!(getter.contains("self.backend"));
    assert!(
        !getter.contains("ssh_session_manager"),
        "当前终端的 X11 请求完成前不得复用共享 transport 的旧状态"
    );
}

#[test]
fn x11_never_renders_a_terminal_session_footer() {
    let source = include_str!("../recording_playback_render.rs");
    assert!(!source.contains("render_x11_session_footer"));
}

#[test]
fn session_notice_copy_exists_in_all_three_languages() {
    for language in ["en", "zh-CN", "zh-HK"] {
        rust_i18n::set_locale(language);
        for key in ["TerminalX11.external"] {
            assert_ne!(key, rust_i18n::t!(key), "{language} 缺少文案: {key}");
        }
    }
    rust_i18n::set_locale("en");
}
