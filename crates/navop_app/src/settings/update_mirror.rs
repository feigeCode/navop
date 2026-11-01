//! 更新镜像源设置项：自动 / Cloudflare / CNB / GitHub。
//!
//! 三个镜像提供的是同一份更新清单，客户端只按偏好顺序依次尝试：选中的镜像排第一，
//! 失败就换下一个（顺序由 `app_kit::update::mirror` 统一维护）。所以这个下拉不会
//! 让用户「换到一个查不到更新的源」——最坏情况只是多花一次请求。
//!
//! 注意 GitHub 只发布正式版本，测试通道不会回退到 GitHub（见 `update_attempts`）。
//!
//! 同一个偏好也作用于扩展下载：扩展安装包与各自的 `extension-manifest.json`
//! 在三处都有副本（R2 / GitHub / CNB），扩展侧按同一个偏好排序候选地址
//! （见 `extension_runtime::mirror`）。CNB 没有 marketplace 清单，所以扩展列表
//! 仍走 R2 / GitHub，只有安装包与扩展清单会优先走 CNB。

use gpui::{App, AppContext as _, SharedString};
use gpui_component::{
    WindowExt as _,
    notification::Notification,
    setting::{SettingField, SettingItem},
};
use one_core::settings::{AppSettings, UpdateMirror};
use rust_i18n::t;

use crate::update;

/// 当前生效的镜像偏好；未选择过时是「自动」。
fn displayed_mirror(cx: &App) -> UpdateMirror {
    AppSettings::global(cx).update_mirror.unwrap_or_default()
}

fn mirror_label(mirror: UpdateMirror) -> SharedString {
    // 用字面量 key，语言包契约测试才能扫到这些标签。
    let label = match mirror {
        UpdateMirror::Auto => t!("Settings.About.Update.mirror_auto").to_string(),
        UpdateMirror::Cloudflare => t!("Settings.About.Update.mirror_cloudflare").to_string(),
        UpdateMirror::Github => t!("Settings.About.Update.mirror_github").to_string(),
        UpdateMirror::Cnb => t!("Settings.About.Update.mirror_cnb").to_string(),
    };
    SharedString::from(label)
}

/// 关于页「更新」分组里的镜像源下拉框。
pub fn update_mirror_item() -> SettingItem {
    let options = UpdateMirror::ALL
        .iter()
        .map(|mirror| (SharedString::from(mirror.as_str()), mirror_label(*mirror)))
        .collect();

    SettingItem::new(
        t!("Settings.About.Update.mirror"),
        SettingField::dropdown(
            options,
            |cx: &App| SharedString::from(displayed_mirror(cx).as_str()),
            |value: SharedString, cx: &mut App| {
                apply_mirror(cx, UpdateMirror::from_str(&value));
            },
        )
        .default_value(SharedString::from(UpdateMirror::Auto.as_str())),
    )
    .description(t!("Settings.About.Update.mirror_desc").to_string())
}

/// 落盘镜像偏好并立刻按新源检查一次更新。
fn apply_mirror(cx: &mut App, mirror: UpdateMirror) {
    if displayed_mirror(cx) == mirror {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.update_mirror = Some(mirror);
    });
    // 扩展下载复用同一个偏好，切换后立刻同步给扩展运行时。
    extension_runtime::set_extension_mirror(mirror);
    notify(cx, t!("Settings.About.Update.switched_mirror").to_string());
    // 换源之后马上重查，否则用户要等下一次启动才能看出这个源通不通。
    update::check_for_updates_manually(cx);
}

fn notify(cx: &mut App, message: String) {
    if let Some(window_id) = cx.active_window() {
        let _ = cx.update_window(window_id, |_, window, cx| {
            window.push_notification(Notification::info(message).autohide(true), cx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mirror_values_match_the_persisted_snake_case_values() {
        assert_eq!("auto", UpdateMirror::Auto.as_str());
        assert_eq!("cloudflare", UpdateMirror::Cloudflare.as_str());
        assert_eq!("github", UpdateMirror::Github.as_str());
        assert_eq!("cnb", UpdateMirror::Cnb.as_str());

        assert_eq!(UpdateMirror::Auto, UpdateMirror::from_str("auto"));
        assert_eq!(
            UpdateMirror::Cloudflare,
            UpdateMirror::from_str("cloudflare")
        );
        assert_eq!(UpdateMirror::Github, UpdateMirror::from_str("github"));
        assert_eq!(UpdateMirror::Cnb, UpdateMirror::from_str("cnb"));
        // 未知取值退回自动，别让配置文件里拼错的值把更新检查卡死。
        assert_eq!(
            UpdateMirror::Auto,
            UpdateMirror::from_str("mirror.example.com")
        );
    }

    #[test]
    fn dropdown_lists_every_mirror_with_the_auto_option_first() {
        assert_eq!(UpdateMirror::Auto, UpdateMirror::ALL[0]);
        assert_eq!(
            UpdateMirror::ALL.len(),
            UpdateMirror::ALL
                .iter()
                .map(|mirror| mirror.as_str())
                .collect::<std::collections::HashSet<_>>()
                .len(),
            "镜像取值必须唯一"
        );
        // 下拉里的每一项都必须能映射回枚举本身，否则选完会被悄悄重置。
        for mirror in UpdateMirror::ALL {
            assert_eq!(mirror, UpdateMirror::from_str(mirror.as_str()));
        }
    }
}
