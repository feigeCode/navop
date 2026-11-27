//! 「设置 → SSH / SFTP → 链接协议」：控制 Navop 是否接管 `jms://` / `ssh://` / `sftp://`。
//!
//! `ssh://`、`sftp://` 是系统级的公共 scheme，终端与文件管理器通常已经注册了
//! 默认处理程序。注册会改变双击/点击链接时的默认行为，所以默认关闭，只在用户
//! 显式勾选后才写入系统关联（macOS 的 scheme 声明是静态的，由应用层再按此设置
//! 放行）。
//!
//! `jms://` 是 Navop 私有的 JumpServer 深链接，默认开启；关闭后同样不再注册
//! 系统关联，并拒绝打开这类链接。
//!
//! 每个开关下面都显示系统侧的实际情况（是否已被 Navop 接管）；打开开关属于
//! 覆写本机配置，所以先弹确认框说明后果，用户确认后才落盘并写系统关联。

use gpui::{
    AnyElement, App, Hsla, IntoElement as _, ParentElement as _, Styled as _, Window, div, px,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    dialog::DialogButtonProps,
    setting::{RenderOptions, SettingField, SettingGroup, SettingItem},
    switch::Switch,
    v_flex,
};
use one_core::settings::AppSettings;
use rust_i18n::t;

use crate::settings::deeplink_status::{self, SchemeTakeover};

/// 含 `ssh://` 与 `jms://` 的链接协议组，放在「设置 → SSH」。
pub fn ssh_deeplink_setting_group() -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.Deeplinks.group_title"))
        .description(t!("Settings.General.Deeplinks.ssh_group_description"))
        .items(vec![ssh_scheme_item(), jms_scheme_item()])
}

/// 只含 `sftp://` 的链接协议组，放在「设置 → SFTP」。
pub fn sftp_deeplink_setting_group() -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.Deeplinks.group_title"))
        .description(t!("Settings.General.Deeplinks.sftp_group_description"))
        .items(vec![sftp_scheme_item()])
}

fn ssh_scheme_item() -> SettingItem {
    scheme_item(
        "ssh",
        t!("Settings.General.Deeplinks.ssh").to_string(),
        t!("Settings.General.Deeplinks.ssh_desc").to_string(),
    )
}

fn sftp_scheme_item() -> SettingItem {
    scheme_item(
        "sftp",
        t!("Settings.General.Deeplinks.sftp").to_string(),
        t!("Settings.General.Deeplinks.sftp_desc").to_string(),
    )
}

fn jms_scheme_item() -> SettingItem {
    scheme_item(
        AppSettings::PRIVATE_DEEPLINK_SCHEME,
        t!("Settings.General.Deeplinks.jms").to_string(),
        t!("Settings.General.Deeplinks.jms_desc").to_string(),
    )
}

fn scheme_item(scheme: &'static str, title: String, description: String) -> SettingItem {
    let default_enabled = AppSettings::default().deeplink_scheme_enabled(scheme);
    SettingItem::new(
        title,
        SettingField::render(move |options, _window, cx| {
            render_scheme_control(scheme, options, cx)
        })
        .on_reset(
            move |cx| AppSettings::global(cx).deeplink_scheme_enabled(scheme) != default_enabled,
            move |_window, cx| apply_scheme_toggle(scheme, default_enabled, cx),
        ),
    )
    .description(description)
}

/// 开关 + 系统侧接管状态。状态来自 [`deeplink_status`] 的后台快照，这里只读。
fn render_scheme_control(
    scheme: &'static str,
    options: &RenderOptions,
    cx: &mut App,
) -> AnyElement {
    let enabled = deeplink_status::switch_enabled(cx, scheme);
    if enabled {
        // 第一次打开这一页时才去探测系统状态，当前帧仍然只读空快照。
        deeplink_status::ensure_probed(cx);
    }
    let status = status_line(scheme, enabled, cx);
    v_flex()
        .items_end()
        .gap_1()
        .child(
            Switch::new(scheme)
                .checked(enabled)
                .with_size(options.size())
                .on_click(move |checked: &bool, window, cx| {
                    request_scheme_toggle(scheme, *checked, window, cx);
                }),
        )
        .child(status)
        .into_any_element()
}

fn status_line(scheme: &'static str, enabled: bool, cx: &App) -> AnyElement {
    let (text, color) = status_text(scheme, enabled, cx);
    div()
        .max_w(px(360.0))
        .text_xs()
        .text_right()
        .text_color(color)
        .child(text)
        .into_any_element()
}

/// 状态行的语义（不含文案与颜色），便于单测覆盖每一条分支。
#[derive(Clone, Debug, PartialEq, Eq)]
enum StatusLine {
    Disabled { scheme: String },
    DisabledButTaken { scheme: String },
    Checking,
    Taken { scheme: String },
    Other { scheme: String, app: String },
    Unset { scheme: String },
    Unknown { scheme: String, error: String },
    NotApplicable,
}

fn status_line_kind(scheme: &str, enabled: bool, takeover: Option<SchemeTakeover>) -> StatusLine {
    let label = deeplink_status::scheme_label(scheme);
    if !enabled {
        return match takeover {
            Some(SchemeTakeover::Taken) => StatusLine::DisabledButTaken { scheme: label },
            _ => StatusLine::Disabled { scheme: label },
        };
    }
    match takeover {
        None => StatusLine::Checking,
        Some(SchemeTakeover::Taken) => StatusLine::Taken { scheme: label },
        Some(SchemeTakeover::Other(app)) => StatusLine::Other { scheme: label, app },
        Some(SchemeTakeover::Unset) => StatusLine::Unset { scheme: label },
        Some(SchemeTakeover::Unknown(error)) => StatusLine::Unknown {
            scheme: label,
            error,
        },
        Some(SchemeTakeover::NotApplicable) => StatusLine::NotApplicable,
    }
}

/// 把接管状态翻成一句话：接管了要说明已被接管，没接管要说清被谁占着。
fn status_text(scheme: &'static str, enabled: bool, cx: &App) -> (String, Hsla) {
    match status_line_kind(scheme, enabled, deeplink_status::takeover(cx, scheme)) {
        StatusLine::Disabled { scheme } => (
            t!(
                "Settings.General.Deeplinks.status_disabled",
                scheme = scheme
            )
            .to_string(),
            cx.theme().muted_foreground,
        ),
        StatusLine::DisabledButTaken { scheme } => (
            t!(
                "Settings.General.Deeplinks.status_disabled_but_taken",
                scheme = scheme
            )
            .to_string(),
            cx.theme().muted_foreground,
        ),
        StatusLine::Checking => (
            t!("Settings.General.Deeplinks.status_checking").to_string(),
            cx.theme().muted_foreground,
        ),
        StatusLine::Taken { scheme } => (
            t!("Settings.General.Deeplinks.status_taken", scheme = scheme).to_string(),
            cx.theme().success,
        ),
        StatusLine::Other { scheme, app } => (
            t!(
                "Settings.General.Deeplinks.status_other",
                scheme = scheme,
                app = app
            )
            .to_string(),
            cx.theme().warning,
        ),
        StatusLine::Unset { scheme } => (
            t!("Settings.General.Deeplinks.status_unset", scheme = scheme).to_string(),
            cx.theme().warning,
        ),
        StatusLine::Unknown { error, .. } => (
            t!(
                "Settings.General.Deeplinks.status_unknown",
                error = error
            )
            .to_string(),
            cx.theme().danger,
        ),
        StatusLine::NotApplicable => (
            t!("Settings.General.Deeplinks.status_development").to_string(),
            cx.theme().muted_foreground,
        ),
    }
}

/// 打开开关会覆写本机默认处理程序，先说明再让用户确认；关闭开关没有副作用。
fn request_scheme_toggle(scheme: &'static str, enabled: bool, window: &mut Window, cx: &mut App) {
    if !enabled {
        apply_scheme_toggle(scheme, false, cx);
        return;
    }
    window.open_alert_dialog(cx, move |alert, _, _| {
        alert
            .title(t!("Settings.General.Deeplinks.overwrite_title").to_string())
            .description(
                t!(
                    "Settings.General.Deeplinks.overwrite_desc",
                    scheme = deeplink_status::scheme_label(scheme)
                )
                .to_string(),
            )
            .confirm()
            .button_props(
                DialogButtonProps::default()
                    .ok_text(t!("Settings.General.Deeplinks.overwrite_ok").to_string())
                    .cancel_text(t!("Settings.General.Deeplinks.overwrite_cancel").to_string())
                    .show_cancel(true),
            )
            .on_ok(move |_, _, cx: &mut App| {
                apply_scheme_toggle(scheme, true, cx);
                true
            })
            .on_cancel(|_, _, cx: &mut App| {
                // 开关是纯受控组件（状态来自设置值），取消后重绘即可回弹。
                cx.refresh_windows();
                true
            })
    });
}

fn apply_scheme_toggle(scheme: &'static str, enabled: bool, cx: &mut App) {
    AppSettings::update_and_save(cx, |settings| {
        settings.set_deeplink_scheme_enabled(scheme, enabled);
    });
    // 开关变化后立即重写系统关联：启用会认领默认处理程序，停用会摘掉
    // 之前由 Navop 写过的键；写完之后再刷新接管状态。
    crate::file_association::schedule_registration_after_toggle(cx);
    cx.refresh_windows();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kind(scheme: &str, enabled: bool, takeover: Option<SchemeTakeover>) -> StatusLine {
        status_line_kind(scheme, enabled, takeover)
    }

    #[test]
    fn enabled_scheme_reports_every_takeover_branch() {
        assert_eq!(StatusLine::Checking, kind("jms", true, None));
        assert_eq!(
            StatusLine::Taken {
                scheme: "jms://".to_string()
            },
            kind("jms", true, Some(SchemeTakeover::Taken))
        );
        assert_eq!(
            StatusLine::Other {
                scheme: "ssh://".to_string(),
                app: "com.netcatty.app".to_string(),
            },
            kind(
                "ssh",
                true,
                Some(SchemeTakeover::Other("com.netcatty.app".to_string()))
            )
        );
        assert_eq!(
            StatusLine::Unset {
                scheme: "sftp://".to_string()
            },
            kind("sftp", true, Some(SchemeTakeover::Unset))
        );
        assert_eq!(
            StatusLine::Unknown {
                scheme: "jms://".to_string(),
                error: "permission denied".to_string(),
            },
            kind(
                "jms",
                true,
                Some(SchemeTakeover::Unknown("permission denied".to_string()))
            )
        );
        // 开发态/便携版不写系统关联，不能显示成「被别的应用占用」。
        assert_eq!(
            StatusLine::NotApplicable,
            kind("jms", true, Some(SchemeTakeover::NotApplicable))
        );
    }

    #[test]
    fn disabled_scheme_says_whether_the_system_still_points_at_navop() {
        assert_eq!(
            StatusLine::Disabled {
                scheme: "jms://".to_string()
            },
            kind("jms", false, None)
        );
        assert_eq!(
            StatusLine::Disabled {
                scheme: "jms://".to_string()
            },
            kind(
                "jms",
                false,
                Some(SchemeTakeover::Other("jumpserver".to_string()))
            )
        );
        assert_eq!(
            StatusLine::DisabledButTaken {
                scheme: "jms://".to_string()
            },
            kind("jms", false, Some(SchemeTakeover::Taken))
        );
    }
}
