//! 「设置 → 通用 → 链接协议」：控制 Navop 是否接管 `ssh://` / `sftp://`。
//!
//! 这些是系统级的公共 scheme，终端与文件管理器通常已经注册了默认处理程序。
//! 注册会改变双击/点击链接时的默认行为，所以默认全部关闭，只在用户显式勾选后
//! 才写入系统关联（macOS 的 scheme 声明是静态的，由应用层再按此设置放行）。

use gpui::{App, IntoElement, ParentElement, SharedString, Styled};
use gpui_component::setting::{SettingField, SettingGroup, SettingItem};
use one_core::settings::AppSettings;
use rust_i18n::t;

pub fn deeplink_setting_group() -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.Deeplinks.group_title"))
        .description(t!("Settings.General.Deeplinks.group_description"))
        .items(vec![
            scheme_item(
                "ssh",
                t!("Settings.General.Deeplinks.ssh").to_string(),
                t!("Settings.General.Deeplinks.ssh_desc").to_string(),
            ),
            scheme_item(
                "sftp",
                t!("Settings.General.Deeplinks.sftp").to_string(),
                t!("Settings.General.Deeplinks.sftp_desc").to_string(),
            ),
        ])
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
