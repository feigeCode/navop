//! 更新通道设置项：稳定版 / 测试版（Beta）。
//!
//! 服务端把测试版清单单独写在 `updates/beta/latest.json`（见
//! `docs/design/beta-channel.md`），客户端只在这里选择跟随哪一份清单：
//! 稳定通道永远只接受正式版本号，测试通道才允许 `0.23.0-beta.N` 这类预发布版本。

use gpui::{
    AnyElement, App, AppContext as _, IntoElement as _, ParentElement as _, SharedString,
    Styled as _, div,
};
use gpui_component::{
    ActiveTheme as _, Sizable as _, WindowExt as _,
    dialog::DialogButtonProps,
    h_flex,
    notification::Notification,
    setting::{SettingField, SettingItem},
    tag::Tag,
};
use one_core::settings::{AppSettings, UpdateChannel};
use rust_i18n::t;

use crate::update;

/// 当前实际生效的通道：用户显式选择优先，否则跟随构建版本（预发布构建默认 Beta）。
fn displayed_channel(cx: &App) -> UpdateChannel {
    UpdateChannel::resolve(
        AppSettings::global(cx).update_channel,
        env!("CARGO_PKG_VERSION"),
    )
}

/// 当前构建本身是不是预发布版本。
pub fn current_build_is_prerelease() -> bool {
    UpdateChannel::from_version(env!("CARGO_PKG_VERSION")) == UpdateChannel::Beta
}

fn channel_label(channel: UpdateChannel) -> SharedString {
    match channel {
        UpdateChannel::Stable => {
            SharedString::from(t!("Settings.About.Update.channel_stable").to_string())
        }
        UpdateChannel::Beta => {
            SharedString::from(t!("Settings.About.Update.channel_beta").to_string())
        }
    }
}

/// 关于页「更新」分组里的通道下拉框。
pub fn update_channel_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.About.Update.channel"),
        SettingField::dropdown(
            vec![
                (
                    UpdateChannel::Stable.as_str().into(),
                    channel_label(UpdateChannel::Stable),
                ),
                (
                    UpdateChannel::Beta.as_str().into(),
                    channel_label(UpdateChannel::Beta),
                ),
            ],
            |cx: &App| SharedString::from(displayed_channel(cx).as_str()),
            |value: SharedString, cx: &mut App| {
                let channel = UpdateChannel::from_str(&value);
                // 切到测试通道要先确认：装上测试版后没有一键回退，得说清楚。
                if channel == UpdateChannel::Beta && displayed_channel(cx) != UpdateChannel::Beta {
                    confirm_switch_to_beta(cx);
                } else {
                    apply_channel(cx, channel);
                }
            },
        )
        .default_value(SharedString::from(
            UpdateChannel::from_version(env!("CARGO_PKG_VERSION")).as_str(),
        )),
    )
    .description(t!("Settings.About.Update.channel_desc").to_string())
}

/// 正在用测试版却已切回稳定通道时的说明条：告诉他们怎么回去。
pub fn beta_exit_notice_item() -> SettingItem {
    SettingItem::render(|_, _, cx| render_beta_exit_notice(cx))
}

fn render_beta_exit_notice(cx: &App) -> AnyElement {
    if displayed_channel(cx) != UpdateChannel::Stable {
        return div().into_any_element();
    }
    h_flex()
        .w_full()
        .items_start()
        .gap_2()
        .rounded_md()
        .bg(cx.theme().warning.opacity(0.12))
        .p_3()
        .child(
            Tag::warning()
                .small()
                .rounded_full()
                .child(channel_label(UpdateChannel::Beta)),
        )
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().foreground)
                .child(t!("Settings.About.Update.beta_exit_notice").to_string()),
        )
        .into_any_element()
}

/// 落盘通道选择并立刻按新通道检查一次更新。
fn apply_channel(cx: &mut App, channel: UpdateChannel) {
    if AppSettings::global(cx).update_channel == Some(channel) {
        return;
    }
    AppSettings::update_and_save(cx, |settings| {
        settings.update_channel = Some(channel);
    });
    let message = match channel {
        UpdateChannel::Stable => t!("Settings.About.Update.switched_stable").to_string(),
        UpdateChannel::Beta => t!("Settings.About.Update.switched_beta").to_string(),
    };
    notify(cx, message);
    // 切完立刻按新通道查一次，否则用户要等到下一次启动才知道有没有新版本。
    update::check_for_updates_manually(cx);
}

fn notify(cx: &mut App, message: String) {
    if let Some(window_id) = cx.active_window() {
        let _ = cx.update_window(window_id, |_, window, cx| {
            window.push_notification(Notification::info(message).autohide(true), cx);
        });
    }
}

fn confirm_switch_to_beta(cx: &mut App) {
    let Some(window_id) = cx.active_window() else {
        // 没有活动窗口时直接生效，避免设置项和落盘值不一致。
        apply_channel(cx, UpdateChannel::Beta);
        return;
    };
    let _ = cx.update_window(window_id, |_, window, cx| {
        window.open_alert_dialog(cx, |alert, _, _| {
            alert
                .title(t!("Settings.About.Update.switch_beta_title").to_string())
                .description(t!("Settings.About.Update.switch_beta_desc").to_string())
                .confirm()
                .button_props(
                    DialogButtonProps::default()
                        .ok_text(t!("Settings.About.Update.switch_beta_confirm").to_string())
                        .cancel_text(t!("Settings.About.Update.switch_beta_cancel").to_string())
                        .show_cancel(true),
                )
                .on_ok(|_, _, cx: &mut App| {
                    apply_channel(cx, UpdateChannel::Beta);
                    true
                })
        });
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn channel_values_match_the_persisted_snake_case_values() {
        assert_eq!("stable", UpdateChannel::Stable.as_str());
        assert_eq!("beta", UpdateChannel::Beta.as_str());
        assert_eq!(UpdateChannel::Stable, UpdateChannel::from_str("stable"));
        assert_eq!(UpdateChannel::Beta, UpdateChannel::from_str("beta"));
        assert_eq!(UpdateChannel::Stable, UpdateChannel::from_str("nightly"));
    }

    #[test]
    fn prerelease_builds_default_to_the_beta_channel() {
        assert_eq!(
            UpdateChannel::Beta,
            UpdateChannel::from_version("0.23.0-beta.2")
        );
        assert_eq!(UpdateChannel::Stable, UpdateChannel::from_version("0.22.0"));
        // 用户显式选择永远优先于构建版本。
        assert_eq!(
            UpdateChannel::Stable,
            UpdateChannel::resolve(Some(UpdateChannel::Stable), "0.23.0-beta.2")
        );
    }
}
