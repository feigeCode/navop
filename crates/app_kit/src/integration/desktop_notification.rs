//! 系统桌面通知桥接（Windows Toast / macOS 通知中心 / Linux freedesktop 通知）。
//!
//! 当终端或子视图发出 [`TabAttention`]（例如 CLI 工具通过 OSC 9 / OSC 777
//! 发送的审批请求或任务完成通知）且满足触发条件时，弹出当前平台的系统通知。
//! Windows 与 Linux 上点击通知会通过
//! [`crate::integration::tray::activate_session_from_external`] 安全投递回 GPUI
//! 前台任务，唤醒主窗口并切换到对应标签页；macOS 的点击由通知中心激活应用本身。

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

/// 当前平台系统通知的能力矩阵。
///
/// 三个平台的系统通知模型差异很大，设置页只应展示真正生效的选项：
///
/// - Windows Toast：停留时长、提示音、点击回跳标签页全都支持。
/// - macOS 通知中心：停留时长由系统（通知中心 / 专注模式）决定，提示音可指定系统音效，
///   点击通知只会激活应用，无法定位到具体标签页。
/// - Linux freedesktop 通知：停留时长映射到 `expire_timeout`，提示音由各通知守护进程
///   自行决定（不暴露开关），点击「打开」动作可以回跳标签页。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NotificationSupport {
    /// 当前平台是否有可用的系统通知后端。
    pub system_notification: bool,
    /// 「停留时长」设置是否生效。
    pub duration: bool,
    /// 「提示音」设置是否生效。
    pub sound: bool,
    /// 点击通知是否能回到触发它的标签页。
    pub click_activation: bool,
}

/// 打开「系统设置 → 通知」面板的深链（仅 macOS 有效）。
///
/// macOS 的通知授权一旦被拒绝，应用自己无法再次弹框，只能引导用户去系统设置里
/// 手动打开；设置页的提示按钮用这个 URL 直达对应面板。
pub const NOTIFICATION_SETTINGS_URL: &str =
    "x-apple.systempreferences:com.apple.Notifications-Settings.extension";

/// 系统通知的授权状态，用于给设置页提供「为什么没有弹」的可见反馈。
///
/// macOS 的通知投递失败不会报错：授权被拒绝时 `UNUserNotificationCenter` 仍然返回
/// 成功，只是以 `effectiveStyle: none` 投递，用户什么也看不到。因此设置页需要先
/// 查一次授权状态，才能在拒绝时告诉用户去系统设置开启。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotificationAuthorization {
    /// 已授权，系统会正常展示通知。
    Authorized,
    /// 尚未询问过，下一次发送会弹出系统授权框。
    NotDetermined,
    /// 用户或系统已拒绝，必须去「系统设置 → 通知」重新允许。
    Denied,
    /// 当前环境无法投递系统通知，或平台没有授权概念。
    ///
    /// macOS 上未打包运行（`cargo run`）时没有 bundle identifier，`UNUserNotificationCenter`
    /// 不可用，也会落到这个状态。
    Unsupported,
}

/// 查询当前系统通知授权状态。
///
/// 这是同步查询（一次系统调用，耗时在毫秒级），且不会弹授权框；需要弹框时用
/// [`send_test_notification`]。
#[must_use]
pub fn authorization() -> NotificationAuthorization {
    platform::authorization()
}

/// 当前平台的系统通知能力。
#[must_use]
pub const fn support() -> NotificationSupport {
    if cfg!(target_os = "windows") {
        NotificationSupport {
            system_notification: true,
            duration: true,
            sound: true,
            click_activation: true,
        }
    } else if cfg!(target_os = "macos") {
        NotificationSupport {
            system_notification: true,
            duration: false,
            sound: true,
            click_activation: false,
        }
    } else if cfg!(target_os = "linux") {
        NotificationSupport {
            system_notification: true,
            duration: true,
            sound: false,
            click_activation: true,
        }
    } else {
        NotificationSupport {
            system_notification: false,
            duration: false,
            sound: false,
            click_activation: false,
        }
    }
}

/// 把「停留时长」换算成毫秒。
///
/// Linux 通知用 `expire_timeout` 表达停留时长；Windows Toast 的 `duration` 属性
/// 只有 long / short 两档语义，走 [`TerminalDesktopNotificationDuration::as_str`]。
#[must_use]
pub const fn duration_millis(duration: TerminalDesktopNotificationDuration) -> u32 {
    match duration {
        TerminalDesktopNotificationDuration::Long => 25_000,
        TerminalDesktopNotificationDuration::Short => 5_000,
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
pub fn format_notification_text(tab_title: &str, attention: &TabAttention) -> (String, String) {
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
///
/// 返回发送前的授权状态，设置页据此在权限被拒绝时给出可见提示：通知投递失败时
/// 系统不会报错，没有这个返回值用户只能看到「点了没反应」。
pub fn send_test_notification(
    duration: TerminalDesktopNotificationDuration,
    sound_enabled: bool,
) -> NotificationAuthorization {
    let authorization = platform::authorization();
    let title = t!("DesktopNotification.test_title").to_string();
    let body = t!("DesktopNotification.test_body").to_string();
    platform::send_tab_notification(None, title, body, duration, sound_enabled);
    authorization
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

    /// Windows Toast 没有应用级授权步骤，投递是否可见由系统通知设置决定，
    /// 这里始终视为可用。
    pub(super) fn authorization() -> super::NotificationAuthorization {
        super::NotificationAuthorization::Authorized
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

        let notifier = ToastNotificationManager::CreateToastNotifierWithId(&HSTRING::from(aumid))?;
        notifier.Show(&toast)?;
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod platform {
    use super::NotificationAuthorization;
    use mac_usernotifications::AuthorizationStatus;
    use notify_rust::Notification;
    use one_core::settings::TerminalDesktopNotificationDuration;

    /// 系统自带的提示音名称，关闭提示音时不下发 `soundName`，由系统静音弹出。
    const NOTIFICATION_SOUND: &str = "Glass";

    /// 发送一条 macOS 系统通知。
    ///
    /// 走 `UNUserNotificationCenter`（`notify-rust` 的 `preview-macos-un` 后端）而不是
    /// 已废弃的 `NSUserNotification`：后者的发送者只能靠 `mac-notification-sys` 的
    /// AppleScript 兜底拿身份，当同一个 bundle id 曾被现代客户端注册过时，
    /// `usernoted` 会直接把 legacy 连接拒掉（日志里是
    /// `Legacy client ... connecting to modern client. You can't mix ...`），表现为
    /// 通知静默丢失。现代通道还顺带解决了「应用在前台时系统不弹 banner」的问题：
    /// 后端的 delegate 会在 `willPresentNotification` 里返回 banner。
    pub(super) fn send_tab_notification(
        _tab_id: Option<String>,
        title: String,
        body: String,
        _duration: TerminalDesktopNotificationDuration,
        sound_enabled: bool,
    ) {
        // 授权弹窗要等用户点击，投递也要等系统回执，全部放到后台线程。
        std::thread::spawn(move || {
            if !ensure_authorized() {
                return;
            }

            let mut notification = Notification::new();
            notification.summary(&title).body(&body);
            if sound_enabled {
                notification.sound_name(NOTIFICATION_SOUND);
            }
            if let Err(error) = notification.show() {
                tracing::warn!(%error, "发送 macOS 系统通知失败");
            }
        });
    }

    /// 查询当前通知授权状态；不会弹授权框。
    pub(super) fn authorization() -> NotificationAuthorization {
        let Ok(settings) = notify_rust::get_notification_settings_blocking() else {
            // 未打包的 `cargo run` 没有 bundle identifier，现代通道无法工作。
            return NotificationAuthorization::Unsupported;
        };
        match settings.authorization_status {
            AuthorizationStatus::Authorized
            | AuthorizationStatus::Provisional
            | AuthorizationStatus::Ephemeral => NotificationAuthorization::Authorized,
            AuthorizationStatus::NotDetermined => NotificationAuthorization::NotDetermined,
            AuthorizationStatus::Denied => NotificationAuthorization::Denied,
            AuthorizationStatus::Unknown => NotificationAuthorization::Unsupported,
        }
    }

    /// 发送前确认授权：未询问过时弹一次授权框，已拒绝时只记日志。
    ///
    /// 不做缓存：用户可能刚去系统设置里打开开关，缓存会让本次运行一直认为被拒绝。
    fn ensure_authorized() -> bool {
        match authorization() {
            NotificationAuthorization::Authorized => true,
            NotificationAuthorization::NotDetermined => {
                match notify_rust::request_auth_blocking() {
                    Ok(granted) => granted,
                    Err(error) => {
                        tracing::warn!(%error, "申请 macOS 通知权限失败");
                        false
                    }
                }
            }
            NotificationAuthorization::Denied => {
                tracing::warn!("macOS 通知权限已被拒绝，请在系统设置 → 通知中允许 Navop");
                false
            }
            NotificationAuthorization::Unsupported => {
                tracing::debug!("当前环境不支持 macOS 系统通知（需要打包成 .app 运行）");
                false
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod platform {
    use super::duration_millis;
    use notify_rust::{Notification, Timeout};
    use one_core::settings::TerminalDesktopNotificationDuration;
    use rust_i18n::t;

    /// freedesktop 通知里「默认动作」的固定标识，点击通知主体时触发。
    const DEFAULT_ACTION: &str = "default";

    /// freedesktop 通知没有应用级授权步骤，由通知守护进程决定是否展示。
    pub(super) fn authorization() -> super::NotificationAuthorization {
        super::NotificationAuthorization::Authorized
    }

    pub(super) fn send_tab_notification(
        tab_id: Option<String>,
        title: String,
        body: String,
        duration: TerminalDesktopNotificationDuration,
        _sound_enabled: bool,
    ) {
        // D-Bus 往返和 `wait_for_action` 的阻塞等待都必须离开 UI 线程。
        std::thread::spawn(move || {
            let mut notification = Notification::new();
            notification
                .appname(crate::display_name())
                .summary(&title)
                .body(&body)
                .timeout(Timeout::Milliseconds(duration_millis(duration)));
            if tab_id.is_some() {
                notification.action(DEFAULT_ACTION, t!("DesktopNotification.open").as_ref());
            }

            let handle = match notification.show() {
                Ok(handle) => handle,
                Err(error) => {
                    tracing::warn!(%error, "发送 Linux 系统通知失败");
                    return;
                }
            };

            // 通知关闭/超时也会回调 `__closed`，只有默认动作才代表用户点了通知。
            if let Some(target_tab_id) = tab_id {
                handle.wait_for_action(move |action| {
                    if action == DEFAULT_ACTION {
                        crate::integration::tray::activate_session_from_external(target_tab_id);
                    }
                });
            }
        });
    }
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
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
    fn support_matrix_matches_current_platform() {
        let support = support();
        // 三个桌面平台都已经接通系统通知后端，非桌面平台才会落到兜底空实现。
        assert_eq!(
            cfg!(any(
                target_os = "windows",
                target_os = "macos",
                target_os = "linux"
            )),
            support.system_notification
        );
        assert_eq!(
            cfg!(any(target_os = "windows", target_os = "linux")),
            support.duration
        );
        assert_eq!(
            cfg!(any(target_os = "windows", target_os = "macos")),
            support.sound
        );
        assert_eq!(
            cfg!(any(target_os = "windows", target_os = "linux")),
            support.click_activation
        );
    }

    #[test]
    fn duration_millis_maps_to_platform_timeout() {
        assert_eq!(
            25_000,
            duration_millis(TerminalDesktopNotificationDuration::Long)
        );
        assert_eq!(
            5_000,
            duration_millis(TerminalDesktopNotificationDuration::Short)
        );
    }

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
