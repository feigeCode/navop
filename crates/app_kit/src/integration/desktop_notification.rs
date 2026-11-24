//! 系统桌面通知桥接（当前实现 Windows 原生 Toast）。
//!
//! 当终端或子视图发出 [`TabAttention`]（例如 CLI 工具通过 OSC 9 / OSC 777
//! 发送的审批请求或任务完成通知）且满足触发条件时，弹出 Windows 系统通知；
//! 点击通知后通过 [`crate::integration::tray::activate_session_from_external`]
//! 安全投递回 GPUI 前台任务，唤醒主窗口并切换到对应标签页。

use one_core::settings::{
    AppSettings, TerminalDesktopNotificationDuration, TerminalDesktopNotificationMode,
};
use one_core::tab_container::{TabAttention, TabAttentionKind};
use rust_i18n::t;

/// 桌面通知发送参数（来自 [`AppSettings`]）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DesktopNotificationOptions {
    pub mode: TerminalDesktopNotificationMode,
    pub duration: TerminalDesktopNotificationDuration,
    pub sound_enabled: bool,
}

impl DesktopNotificationOptions {
    #[must_use]
    pub fn from_settings(settings: &AppSettings) -> Self {
        Self {
            mode: settings.terminal_desktop_notification_mode,
            duration: settings.terminal_desktop_notification_duration,
            sound_enabled: settings.terminal_desktop_notification_sound,
        }
    }
}

/// 根据用户设置的通知策略与当前焦点状态，决定是否应当弹出系统桌面通知。
#[must_use]
pub fn should_dispatch_os_notification(
    mode: TerminalDesktopNotificationMode,
    tab_is_active: bool,
    window_is_active: bool,
) -> bool {
    match mode {
        TerminalDesktopNotificationMode::UnfocusedOrInactiveTab => {
            !window_is_active || !tab_is_active
        }
        TerminalDesktopNotificationMode::UnfocusedWindowOnly => !window_is_active,
        TerminalDesktopNotificationMode::Always => true,
        TerminalDesktopNotificationMode::Off => false,
    }
}

/// 构造系统通知的标题与正文。
#[must_use]
pub fn format_notification_text(
    tab_title: &str,
    attention: &TabAttention,
) -> (String, String) {
    let tab_title = tab_title.trim();
    let prefix = match attention.kind {
        TabAttentionKind::Approval => t!("DesktopNotification.prefix_approval"),
        TabAttentionKind::Completed => t!("DesktopNotification.prefix_completed"),
        TabAttentionKind::Info => t!("DesktopNotification.prefix_info"),
    };

    let heading = match (attention.title.as_deref(), tab_title.is_empty()) {
        (Some(title), false) if !title.trim().is_empty() => {
            format!("{prefix} · {tab_title} ({})", title.trim())
        }
        (Some(title), true) if !title.trim().is_empty() => format!("{prefix} · {}", title.trim()),
        (_, false) => format!("{prefix} · {tab_title}"),
        (_, true) => prefix.to_string(),
    };

    let body = if attention.message.trim().is_empty() {
        prefix.to_string()
    } else {
        attention.message.trim().to_string()
    };

    (heading, body)
}

/// 组装 Windows Toast XML，内置 XML 特殊字符转义。
///
/// 支持按配置切换 `duration="long"`（约 25 秒）或 `duration="short"`（约 5 秒），
/// 并在关闭提示音时追加 `<audio silent="true"/>`。
#[must_use]
pub fn build_toast_xml(
    title: &str,
    body: &str,
    tab_id: &str,
    duration: TerminalDesktopNotificationDuration,
    sound_enabled: bool,
) -> String {
    let title = xml_escape(title);
    let body = xml_escape(body);
    let launch = xml_escape(tab_id);
    let duration_attr = duration.as_str();
    let audio = if sound_enabled {
        ""
    } else {
        r#"<audio silent="true"/>"#
    };
    format!(
        r#"<toast duration="{duration_attr}" launch="{launch}"><visual><binding template="ToastGeneric"><text>{title}</text><text>{body}</text></binding></visual>{audio}</toast>"#
    )
}

fn xml_escape(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for ch in input.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            '\0' => {}
            other => out.push(other),
        }
    }
    out
}

/// 发送一条指向特定标签页的系统桌面通知。
///
/// 失败只打 warn 日志，不阻塞也不中断 UI 主线程。
pub fn notify_tab_attention(
    options: DesktopNotificationOptions,
    tab_id: &str,
    tab_title: &str,
    tab_is_active: bool,
    window_is_active: bool,
    attention: &TabAttention,
) {
    if !should_dispatch_os_notification(options.mode, tab_is_active, window_is_active) {
        return;
    }

    let (title, body) = format_notification_text(tab_title, attention);
    platform::send_tab_notification(
        Some(tab_id.to_string()),
        title,
        body,
        options.duration,
        options.sound_enabled,
    );
}

/// 发送一条测试系统通知，用于用户在设置页验证系统通知权限、停留时长与提示音。
pub fn send_test_notification(
    duration: TerminalDesktopNotificationDuration,
    sound_enabled: bool,
) {
    let title = t!("DesktopNotification.test_title").to_string();
    let body = t!("DesktopNotification.test_body").to_string();
    platform::send_tab_notification(None, title, body, duration, sound_enabled);
}

#[cfg(target_os = "windows")]
mod platform {
    use super::build_toast_xml;
    use one_core::settings::TerminalDesktopNotificationDuration;
    use std::sync::OnceLock;
    use windows::Data::Xml::Dom::XmlDocument;
    use windows::Foundation::TypedEventHandler;
    use windows::UI::Notifications::{ToastNotification, ToastNotificationManager};
    use windows::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_WRITE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
        RegCreateKeyExW, RegSetValueExW,
    };
    use windows::Win32::UI::Shell::SetCurrentProcessExplicitAppUserModelID;
    use windows::core::{HSTRING, IInspectable, PCWSTR};

    static AUMID_READY: OnceLock<String> = OnceLock::new();

    fn ensure_aumid() -> &'static str {
        AUMID_READY.get_or_init(|| {
            let identity = crate::identity();
            let aumid = identity
                .map(|id| id.app_id)
                .unwrap_or("com.navop.desktop")
                .to_string();
            let display_name = crate::display_name();

            unsafe {
                let _ = SetCurrentProcessExplicitAppUserModelID(&HSTRING::from(&aumid));
            }
            register_aumid_in_hkcu(&aumid, &display_name);
            aumid
        })
    }

    /// 在 `HKCU\Software\Classes\AppUserModelId\<aumid>` 注册展示名。
    ///
    /// 未打包（unpackaged / portable / cargo run）的 Win32 进程只要有这条 HKCU 记录，
    /// Windows ToastNotificationManager 就能正常弹出带应用名的原生 Toast，且回调
    /// `Activated` 事件在进程存活期间直接回到本进程。
    fn register_aumid_in_hkcu(aumid: &str, display_name: &str) {
        let subkey = HSTRING::from(format!(r"Software\Classes\AppUserModelId\{aumid}"));
        let value_name = HSTRING::from("DisplayName");
        let wide_name: Vec<u16> = display_name
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        let bytes = unsafe {
            std::slice::from_raw_parts(wide_name.as_ptr().cast::<u8>(), wide_name.len() * 2)
        };

        let mut hkey = HKEY::default();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                PCWSTR(subkey.as_ptr()),
                Some(0),
                PCWSTR::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_WRITE,
                None,
                &mut hkey,
                None,
            )
        };
        if status.is_err() {
            tracing::debug!(?status, %aumid, "注册 Windows AppUserModelId 失败");
            return;
        }
        let _ = unsafe {
            RegSetValueExW(
                hkey,
                PCWSTR(value_name.as_ptr()),
                Some(0),
                REG_SZ,
                Some(bytes),
            )
        };
        let _ = unsafe { RegCloseKey(hkey) };
    }

    pub(super) fn send_tab_notification(
        tab_id: Option<String>,
        title: String,
        body: String,
        duration: TerminalDesktopNotificationDuration,
        sound_enabled: bool,
    ) {
        let aumid = ensure_aumid().to_string();
        let xml_payload = build_toast_xml(
            &title,
            &body,
            tab_id.as_deref().unwrap_or(""),
            duration,
            sound_enabled,
        );

        // WinRT Toast 创建与展示放到后台线程，避免阻塞 GPUI UI 线程。
        std::thread::spawn(move || {
            if let Err(error) = show_toast(&aumid, &xml_payload, tab_id) {
                tracing::warn!(%error, "发送 Windows 系统通知失败");
            }
        });
    }

    fn show_toast(
        aumid: &str,
        xml_payload: &str,
        tab_id: Option<String>,
    ) -> windows::core::Result<()> {
        let doc = XmlDocument::new()?;
        doc.LoadXml(&HSTRING::from(xml_payload))?;

        let toast = ToastNotification::CreateToastNotification(&doc)?;
        if let Some(target_tab_id) = tab_id {
            toast.Activated(&TypedEventHandler::<ToastNotification, IInspectable>::new(
                move |_sender, _args| {
                    crate::integration::tray::activate_session_from_external(target_tab_id.clone());
                    Ok(())
                },
            ))?;
        }

        let notifier =
            ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))?;
        notifier.Show(&toast)?;
        Ok(())
    }
}

#[cfg(not(target_os = "windows"))]
mod platform {
    use one_core::settings::TerminalDesktopNotificationDuration;

    pub(super) fn send_tab_notification(
        _tab_id: Option<String>,
        _title: String,
        _body: String,
        _duration: TerminalDesktopNotificationDuration,
        _sound_enabled: bool,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dispatches_according_to_notification_mode() {
        use TerminalDesktopNotificationMode as Mode;

        assert!(!should_dispatch_os_notification(
            Mode::UnfocusedOrInactiveTab,
            true,
            true
        ));
        assert!(should_dispatch_os_notification(
            Mode::UnfocusedOrInactiveTab,
            false,
            true
        ));
        assert!(should_dispatch_os_notification(
            Mode::UnfocusedOrInactiveTab,
            true,
            false
        ));
        assert!(should_dispatch_os_notification(
            Mode::UnfocusedOrInactiveTab,
            false,
            false
        ));

        assert!(!should_dispatch_os_notification(
            Mode::UnfocusedWindowOnly,
            true,
            true
        ));
        assert!(!should_dispatch_os_notification(
            Mode::UnfocusedWindowOnly,
            false,
            true
        ));
        assert!(should_dispatch_os_notification(
            Mode::UnfocusedWindowOnly,
            true,
            false
        ));
        assert!(should_dispatch_os_notification(
            Mode::UnfocusedWindowOnly,
            false,
            false
        ));

        assert!(should_dispatch_os_notification(Mode::Always, true, true));
        assert!(should_dispatch_os_notification(Mode::Always, false, false));

        assert!(!should_dispatch_os_notification(Mode::Off, true, true));
        assert!(!should_dispatch_os_notification(Mode::Off, false, false));
    }

    #[test]
    fn formats_approval_and_completed_notifications() {
        let approval_prefix = t!("DesktopNotification.prefix_approval");
        let completed_prefix = t!("DesktopNotification.prefix_completed");

        let approval = TabAttention::from_notification(None, "Needs approval".to_string());
        assert_eq!(TabAttentionKind::Approval, approval.kind);
        let (title, body) = format_notification_text("dev-shell", &approval);
        assert_eq!(format!("{approval_prefix} · dev-shell"), title);
        assert_eq!("Needs approval", body);

        let done = TabAttention::from_notification(
            Some("Antigravity".to_string()),
            "Agent finished".to_string(),
        );
        assert_eq!(TabAttentionKind::Completed, done.kind);
        let (title, body) = format_notification_text("dev-shell", &done);
        assert_eq!(format!("{completed_prefix} · dev-shell (Antigravity)"), title);
        assert_eq!("Agent finished", body);
    }

    #[test]
    fn toast_xml_escapes_special_characters_and_applies_options() {
        let xml = build_toast_xml(
            r#"A & <B>"#,
            r#"Run "cmd" > 'out'"#,
            "tab-1&2",
            TerminalDesktopNotificationDuration::Long,
            true,
        );
        assert!(xml.contains(r#"duration="long""#));
        assert!(xml.contains(r#"launch="tab-1&amp;2""#));
        assert!(xml.contains("<text>A &amp; &lt;B&gt;</text>"));
        assert!(xml.contains("<text>Run &quot;cmd&quot; &gt; &apos;out&apos;</text>"));
        assert!(!xml.contains("<audio"));

        let silent_short = build_toast_xml(
            "Title",
            "Body",
            "tab-3",
            TerminalDesktopNotificationDuration::Short,
            false,
        );
        assert!(silent_short.contains(r#"duration="short""#));
        assert!(silent_short.contains(r#"<audio silent="true"/>"#));
    }
}
