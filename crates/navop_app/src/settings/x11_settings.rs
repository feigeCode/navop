//! 通用设置中的 X11 服务选择、状态与内置组件维护。

use gpui::{
    AnyElement, App, AppContext as _, AsyncApp, Context, Entity, Global, IntoElement,
    ParentElement as _, Render, SharedString, Styled as _, WeakEntity, Window, div,
    prelude::FluentBuilder as _,
};
use gpui_component::{
    ActiveTheme as _,
    button::Button,
    h_flex,
    setting::{SettingField, SettingGroup, SettingItem},
    v_flex,
};
use one_assets::IconName;
use one_core::gpui_tokio::Tokio;
use one_core::settings::{
    AppSettings, X11ForwardingStatus, X11ServerMode, current_status, installed_version, redownload,
    take_restart_notification_for,
};
use rust_i18n::t;
use std::time::Duration;

use super::x11_state::{
    X11DownloadState, x11_disabled_guidance_key, x11_download_error_key, x11_status_label,
};

const X11_RESTART_POLL_INTERVAL: Duration = Duration::from_secs(1);

#[derive(Clone)]
struct GlobalX11SettingsState(Entity<X11SettingsView>);

impl Global for GlobalX11SettingsState {}

#[derive(Default)]
pub(crate) struct X11SettingsView {
    download: X11DownloadState,
}

pub(crate) fn x11_setting_group(default_mode: X11ServerMode) -> SettingGroup {
    SettingGroup::new()
        .title(t!("Settings.General.X11.group_title"))
        .item(mode_item(default_mode))
        .item(status_item())
}

fn mode_item(default_mode: X11ServerMode) -> SettingItem {
    SettingItem::new(
        t!("Settings.General.X11.mode"),
        SettingField::dropdown(
            vec![
                ("auto".into(), mode_label(X11ServerMode::Auto)),
                ("external".into(), mode_label(X11ServerMode::External)),
                ("embedded".into(), mode_label(X11ServerMode::Embedded)),
            ],
            |cx: &App| SharedString::from(mode_value(AppSettings::global(cx).x11_server_mode)),
            |value: SharedString, cx: &mut App| {
                let mode = mode_from_value(&value);
                AppSettings::update_and_save(cx, |settings| settings.x11_server_mode = mode);
                x11_settings_state(cx).update(cx, |_, cx| cx.notify());
            },
        )
        .default_value(SharedString::from(mode_value(default_mode))),
    )
    .description(t!("Settings.General.X11.mode_desc").to_string())
}

fn status_item() -> SettingItem {
    SettingItem::new(
        t!("Settings.General.X11.status"),
        SettingField::render(|_, _, cx| x11_settings_state(cx).into_any_element()),
    )
    .description(t!("Settings.General.X11.status_desc").to_string())
}

fn x11_settings_state(cx: &mut App) -> Entity<X11SettingsView> {
    if let Some(global) = cx.try_global::<GlobalX11SettingsState>() {
        return global.0.clone();
    }
    let state = cx.new(X11SettingsView::new);
    cx.set_global(GlobalX11SettingsState(state.clone()));
    state
}

fn mode_label(mode: X11ServerMode) -> SharedString {
    let label = match mode {
        X11ServerMode::Auto => t!("Settings.General.X11.mode_auto"),
        X11ServerMode::External => t!("Settings.General.X11.mode_external"),
        X11ServerMode::Embedded => t!("Settings.General.X11.mode_embedded"),
    };
    SharedString::from(label.to_string())
}

fn mode_value(mode: X11ServerMode) -> &'static str {
    match mode {
        X11ServerMode::Auto => "auto",
        X11ServerMode::External => "external",
        X11ServerMode::Embedded => "embedded",
    }
}

fn mode_from_value(value: &str) -> X11ServerMode {
    match value {
        "external" => X11ServerMode::External,
        "embedded" => X11ServerMode::Embedded,
        _ => X11ServerMode::Auto,
    }
}

impl X11SettingsView {
    fn new(cx: &mut Context<Self>) -> Self {
        let view = Self::default();
        view.poll_restart_notifications(cx);
        view
    }

    fn poll_restart_notifications(&self, cx: &mut Context<Self>) {
        let consumer = cx.entity().entity_id().as_u64();
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            loop {
                let notification = take_restart_notification_for(consumer);
                let alive = this
                    .update(cx, |this, cx| {
                        this.download.record_restart(notification);
                        cx.notify();
                    })
                    .is_ok();
                if !alive {
                    break;
                }
                cx.background_executor()
                    .timer(X11_RESTART_POLL_INTERVAL)
                    .await;
            }
        })
        .detach();
    }

    fn start_redownload(&mut self, cx: &mut Context<Self>) {
        if !self.download.begin_download() {
            return;
        }
        cx.notify();
        let task = Tokio::spawn(cx, async move { redownload().await });
        cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            let result = task
                .await
                .map_err(|error| {
                    t!(
                        "Settings.General.X11.download_failed_retry",
                        reason = error.to_string()
                    )
                    .to_string()
                })
                .and_then(|result| {
                    result.map_err(|error| {
                        let key = x11_download_error_key(&error);
                        t!(key, reason = error.to_string()).to_string()
                    })
                });
            let _ = this.update(cx, |this, cx| {
                this.download.finish_download(result);
                cx.notify();
            });
        })
        .detach();
    }

    fn render_summary(&self, status: &X11ForwardingStatus, cx: &App) -> AnyElement {
        let version = installed_version()
            .unwrap_or_else(|| t!("Settings.General.X11.not_installed").to_string());
        v_flex()
            .gap_1()
            .child(div().text_sm().child(x11_status_label(status)))
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        t!("Settings.General.X11.installed_version", version = version).to_string(),
                    ),
            )
            .into_any_element()
    }

    fn render_restart(&self, cx: &App) -> Option<AnyElement> {
        let notification = self.download.restart_notification()?;
        Some(
            div()
                .text_xs()
                .text_color(cx.theme().warning)
                .child(
                    t!(
                        "Settings.General.X11.remote_display_restarted",
                        reason = notification.reason
                    )
                    .to_string(),
                )
                .into_any_element(),
        )
    }

    fn render_download(&self, cx: &mut Context<Self>) -> AnyElement {
        let downloading = self.download.is_downloading();
        let label = if downloading {
            t!("Settings.General.X11.downloading")
        } else {
            t!("Settings.General.X11.redownload")
        };
        h_flex()
            .w_full()
            .items_center()
            .justify_between()
            .gap_3()
            .when_some(
                self.download.message().map(str::to_owned),
                |this, message| this.child(div().flex_1().min_w_0().text_sm().child(message)),
            )
            .child(
                Button::new("settings-x11-redownload")
                    .icon(IconName::Refresh)
                    .label(label)
                    .loading(downloading)
                    .on_click(cx.listener(|this, _, _, cx| this.start_redownload(cx))),
            )
            .into_any_element()
    }
}

impl Render for X11SettingsView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let status = current_status();
        v_flex()
            .w_full()
            .gap_3()
            .child(self.render_summary(&status, cx))
            .child(self.render_download(cx))
            .when_some(self.render_restart(cx), |this, notification| {
                this.child(notification)
            })
            .when_some(disabled_guidance(&status, cx), |this, guidance| {
                this.child(guidance)
            })
            .when(cfg!(target_os = "macos"), |this| {
                this.child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(t!("Settings.General.X11.macos_xquartz_guidance").to_string()),
                )
            })
    }
}

fn disabled_guidance(status: &X11ForwardingStatus, cx: &App) -> Option<AnyElement> {
    let key = x11_disabled_guidance_key(status)?;
    let X11ForwardingStatus::Disabled { reason } = status else {
        return None;
    };
    Some(
        div()
            .text_xs()
            .text_color(cx.theme().muted_foreground)
            .child(t!(key, reason = reason).to_string())
            .into_any_element(),
    )
}

#[cfg(test)]
mod tests {
    #[test]
    fn x11_mode_description_requires_reconnect_for_existing_sessions() {
        for language in ["en", "zh-CN", "zh-HK"] {
            rust_i18n::set_locale(language);
            let description = rust_i18n::t!("Settings.General.X11.mode_desc");
            let expected = match language {
                "en" => "must be reconnected",
                "zh-CN" => "必须重新连接",
                "zh-HK" => "必須重新連線",
                _ => unreachable!(),
            };
            assert!(description.contains(expected), "{language}: {description}");
        }
        rust_i18n::set_locale("en");
    }
}
