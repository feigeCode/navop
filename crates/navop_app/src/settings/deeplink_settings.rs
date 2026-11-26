//! 「设置 → SSH / SFTP → 链接协议」：控制 Navop 是否接管 `jms://` / `ssh://` / `sftp://`。
//!
//! `ssh://`、`sftp://` 是系统级的公共 scheme，终端与文件管理器通常已经注册了
//! 默认处理程序。注册会改变双击/点击链接时的默认行为，所以默认关闭，只在用户
//! 显式勾选后才写入系统关联（macOS 的 scheme 声明是静态的，由应用层再按此设置
//! 放行）。
//!
//! `jms://` 是 Navop 私有的 JumpServer 深链接，默认开启；关闭后同样不再注册
//! 系统关联，并拒绝打开这类链接。

use gpui::App;
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::AppSettings;
use rust_i18n::t;

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
    SettingItem::new(
        title,
        SettingField::switch(
            move |cx: &App| AppSettings::global(cx).deeplink_scheme_enabled(scheme),
            move |enabled: bool, cx: &mut App| {
                AppSettings::update_and_save(cx, |settings| {
                    settings.set_deeplink_scheme_enabled(scheme, enabled);
                });
                // 开关变化后立即重写系统关联：启用会注册 scheme，停用会摘掉
                // 之前由 Navop 写过的键。
                crate::file_association::schedule_registration(cx);
                cx.refresh_windows();
            },
        )
        .default_value(AppSettings::default().deeplink_scheme_enabled(scheme)),
    )
    .description(description)
}
