use connection_form::team::{
    connection_sync_controls_visible_in, refresh_teams_tooltip, team_label, team_management_enabled,
};
use gpui::prelude::FluentBuilder;
use gpui::{
    App, Context, FocusHandle, Focusable, IntoElement, ParentElement, Render, Styled, Window, div,
    px,
};
#[cfg(windows)]
use gpui_component::radio::RadioGroup;
use gpui_component::{
    ActiveTheme,
    button::{Button, ButtonVariants as _},
    checkbox::Checkbox,
    h_flex,
    input::Input,
    radio::Radio,
    scroll::ScrollableElement,
    select::Select,
    v_flex,
};
use one_assets::IconName;
use rust_i18n::t;

use super::RemoteDesktopFormWindow;
use one_core::storage::{
    RdpAudioMode, RdpEgfxMode, RdpKeyboardHookMode, RdpSettings, RemoteDesktopProtocol,
};

impl RemoteDesktopFormWindow {
    fn render_form_row(&self, label: String, child: impl IntoElement) -> impl IntoElement {
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(100.0)).text_sm().text_right().child(label))
            .child(div().flex_1().child(child))
    }

    fn render_body(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let credential_is_manual = self
            .credential_picker
            .read(cx)
            .selected_reference()
            .is_none();

        v_flex()
            .gap_2()
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_name").to_string(),
                Input::new(&self.name_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_host").to_string(),
                Input::new(&self.host_input),
            ))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_port").to_string(),
                Input::new(&self.port_input),
            ))
            .child(self.render_form_row("钥匙串".to_string(), self.credential_picker.clone()))
            .when(credential_is_manual, |form| {
                form.child(self.render_form_row(
                    t!("RemoteDesktopForm.label_username").to_string(),
                    Input::new(&self.username_input),
                ))
                .child(self.render_form_row(
                    t!("RemoteDesktopForm.label_password").to_string(),
                    Input::new(&self.password_input).mask_toggle(),
                ))
            })
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_domain").to_string(),
                Input::new(&self.domain_input),
            ))
            .child(self.render_proxy_section(cx))
            .child(self.render_form_row(
                t!("RemoteDesktopForm.label_workspace").to_string(),
                Select::new(&self.workspace_select).w_full(),
            ))
            .when(
                connection_sync_controls_visible_in(cx) && team_management_enabled(cx),
                |form| {
                    form.child(
                        self.render_form_row(
                            team_label(),
                            h_flex()
                                .gap_2()
                                .child(Select::new(&self.team_select).w_full())
                                .child(
                                    Button::new("sync-remote-desktop-teams")
                                        .icon(IconName::Refresh)
                                        .ghost()
                                        .tooltip(refresh_teams_tooltip())
                                        .on_click(cx.listener(|this, _, window, cx| {
                                            this.request_team_sync(window, cx);
                                        })),
                                ),
                        ),
                    )
                },
            )
            .child(self.render_read_only_row(cx))
            .when(self.protocol == RemoteDesktopProtocol::Rdp, |form| {
                let form = form.child(self.render_audio_playback_row(cx));
                let form = form.child(self.render_keyboard_hook_row(cx));
                let form = form.child(self.render_legacy_security_row(cx));
                let form = form.when(self.egfx_controls_visible(), |form| {
                    form.child(self.render_egfx_row(cx))
                });
                #[cfg(windows)]
                let form = form.child(self.render_backend_preference_row(cx));
                form
            })
            .when(connection_sync_controls_visible_in(cx), |form| {
                form.child(self.render_sync_row(cx))
            })
    }

    fn render_proxy_section(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let proxy_credential_is_manual = self
            .proxy_credential_picker
            .read(cx)
            .selected_reference()
            .is_none();

        v_flex()
            .gap_2()
            .child(
                self.render_form_row(
                    t!("RemoteDesktopForm.label_proxy").to_string(),
                    Checkbox::new("remote-desktop-proxy-enabled")
                        .checked(self.proxy_enabled)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_enabled = !this.proxy_enabled;
                            cx.notify();
                        })),
                ),
            )
            .when(self.proxy_enabled, |form| {
                form.child(self.render_proxy_type_row(cx))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_host").to_string(),
                        Input::new(&self.proxy_host_input),
                    ))
                    .child(self.render_form_row(
                        t!("RemoteDesktopForm.label_proxy_port").to_string(),
                        Input::new(&self.proxy_port_input),
                    ))
                    .child(self.render_form_row(
                        "钥匙串".to_string(),
                        self.proxy_credential_picker.clone(),
                    ))
                    .when(proxy_credential_is_manual, |form| {
                        form.child(self.render_form_row(
                            t!("RemoteDesktopForm.label_proxy_username").to_string(),
                            Input::new(&self.proxy_username_input),
                        ))
                        .child(self.render_form_row(
                            t!("RemoteDesktopForm.label_proxy_password").to_string(),
                            Input::new(&self.proxy_password_input).mask_toggle(),
                        ))
                    })
            })
    }

    fn render_proxy_type_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_form_row(
            t!("RemoteDesktopForm.label_proxy_type").to_string(),
            h_flex()
                .gap_4()
                .child(
                    Radio::new("remote-desktop-proxy-socks5")
                        .label("SOCKS5")
                        .checked(self.proxy_type == one_core::storage::ProxyType::Socks5)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_type = one_core::storage::ProxyType::Socks5;
                            cx.notify();
                        })),
                )
                .child(
                    Radio::new("remote-desktop-proxy-http")
                        .label("HTTP CONNECT")
                        .checked(self.proxy_type == one_core::storage::ProxyType::Http)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.proxy_type = one_core::storage::ProxyType::Http;
                            cx.notify();
                        })),
                ),
        )
    }

    fn render_read_only_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_form_row(
            t!("RemoteDesktopForm.label_read_only").to_string(),
            Checkbox::new("remote-desktop-read-only")
                .checked(self.read_only)
                .on_click(cx.listener(|this, _, _, cx| {
                    this.read_only = !this.read_only;
                    cx.notify();
                })),
        )
    }

    fn render_audio_playback_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        self.render_form_row(
            t!("RemoteDesktopForm.label_audio_playback").to_string(),
            Checkbox::new("remote-desktop-audio-playback")
                .checked(self.audio_playback)
                .on_click(cx.listener(|this, _, _, cx| {
                    let enabled = !this.audio_playback;
                    this.audio_playback = enabled;
                    if this.protocol == RemoteDesktopProtocol::Rdp
                        && let Some(settings) = this.rdp_settings.as_mut()
                    {
                        settings.audio.mode = if enabled {
                            RdpAudioMode::Local
                        } else {
                            RdpAudioMode::Disabled
                        };
                    }
                    cx.notify();
                })),
        )
    }

    /// 老式 RDP 服务端（如未装 NLA 的 xrdp / 旧版 Windows Server）常常拒绝
    /// CredSSP 认证；勾选后用「兼容」安全策略再试一次，避免连接在协商阶段就被拒。
    fn legacy_security_compatible(&self) -> bool {
        legacy_security_compatible_for(self.rdp_settings.as_ref())
    }

    fn set_legacy_security_compatible(&mut self, enabled: bool) {
        apply_legacy_security_compatible(&mut self.rdp_settings, self.audio_playback, enabled);
    }

    fn render_legacy_security_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_1()
            .child(
                self.render_form_row(
                    t!("RemoteDesktopForm.label_legacy_security").to_string(),
                    Checkbox::new("remote-desktop-legacy-security")
                        .checked(self.legacy_security_compatible())
                        .on_click(cx.listener(|this, _, _, cx| {
                            let enabled = !this.legacy_security_compatible();
                            this.set_legacy_security_compatible(enabled);
                            cx.notify();
                        })),
                ),
            )
            .child(
                div()
                    .pl(px(112.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("RemoteDesktopForm.legacy_security_hint").to_string()),
            )
    }

    /// 键盘接管策略：Alt+Tab / Win 这类系统组合键是否发给远端。
    ///
    /// 默认 `Focused`，即在远端窗口获得焦点时接管；用户想用 Alt+Tab 切回本机时可以选
    /// 「仅本地」。该设置目前只由 Windows 原生 RDP 后端（`windows_rdp_host`）消费。
    fn keyboard_hook_mode(&self) -> RdpKeyboardHookMode {
        keyboard_hook_mode_for(self.rdp_settings.as_ref())
    }

    fn set_keyboard_hook_mode(&mut self, mode: RdpKeyboardHookMode) {
        apply_keyboard_hook_mode(&mut self.rdp_settings, self.audio_playback, mode);
    }

    fn render_keyboard_hook_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.keyboard_hook_mode();
        let options = [
            (
                "remote-desktop-keyboard-hook-local",
                RdpKeyboardHookMode::Local,
                t!("RemoteDesktopForm.keyboard_hook_local").to_string(),
            ),
            (
                "remote-desktop-keyboard-hook-focused",
                RdpKeyboardHookMode::Focused,
                t!("RemoteDesktopForm.keyboard_hook_focused").to_string(),
            ),
            (
                "remote-desktop-keyboard-hook-fullscreen",
                RdpKeyboardHookMode::Fullscreen,
                t!("RemoteDesktopForm.keyboard_hook_fullscreen").to_string(),
            ),
        ];

        let mut row = h_flex().gap_4();
        for (id, mode, label) in options {
            row = row.child(
                Radio::new(id)
                    .label(label)
                    .checked(selected == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_keyboard_hook_mode(mode);
                        cx.notify();
                    })),
            );
        }

        v_flex()
            .gap_1()
            .child(
                self.render_form_row(t!("RemoteDesktopForm.label_keyboard_hook").to_string(), row),
            )
            .child(
                div()
                    .pl(px(112.0))
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(t!("RemoteDesktopForm.keyboard_hook_hint").to_string()),
            )
    }

    /// The graphics pipeline policy only applies to the IronRDP backend: the Windows native
    /// backend never launches the helper, so the setting would do nothing there.
    fn egfx_controls_visible(&self) -> bool {
        if self.protocol != RemoteDesktopProtocol::Rdp {
            return false;
        }
        #[cfg(windows)]
        let visible = !super::backend_preference::windows_native_rdp_available()
            || super::backend_preference::effective_backend_preference(self.backend_preference)
                == one_core::storage::RemoteDesktopBackendPreference::Canvas;
        #[cfg(not(windows))]
        let visible = true;
        visible
    }

    /// The policy lives in the RDP settings, which stay absent until the user picks something.
    fn egfx_mode(&self) -> RdpEgfxMode {
        self.rdp_settings
            .as_ref()
            .map(|settings| settings.graphics.egfx)
            .unwrap_or_default()
    }

    fn set_egfx_mode(&mut self, mode: RdpEgfxMode) {
        let audio_playback = self.audio_playback;
        let settings = self
            .rdp_settings
            .get_or_insert_with(|| RdpSettings::from_legacy_audio_playback(audio_playback));
        settings.graphics.egfx = mode;
    }

    fn render_egfx_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let selected = self.egfx_mode();
        let options = [
            (
                "remote-desktop-egfx-auto",
                RdpEgfxMode::Auto,
                t!("RemoteDesktopForm.egfx_auto").to_string(),
            ),
            (
                "remote-desktop-egfx-always",
                RdpEgfxMode::Always,
                t!("RemoteDesktopForm.egfx_always").to_string(),
            ),
            (
                "remote-desktop-egfx-never",
                RdpEgfxMode::Never,
                t!("RemoteDesktopForm.egfx_never").to_string(),
            ),
        ];

        let mut row = h_flex().gap_4();
        for (id, mode, label) in options {
            row = row.child(
                Radio::new(id)
                    .label(label)
                    .checked(selected == mode)
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.set_egfx_mode(mode);
                        cx.notify();
                    })),
            );
        }

        self.render_form_row(t!("RemoteDesktopForm.label_egfx").to_string(), row)
    }

    #[cfg(windows)]
    fn render_backend_preference_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let native_available = super::backend_preference::windows_native_rdp_available();
        // Legacy connections that stored Auto map to the effective backend for display:
        // Windows native when available, otherwise IronRDP (Canvas).
        let effective_preference =
            super::backend_preference::effective_backend_preference(self.backend_preference);
        let selected_index = super::backend_preference::backend_preferences()
            .iter()
            .position(|preference| *preference == effective_preference);

        self.render_form_row(
            t!("RemoteDesktopForm.label_backend_preference").to_string(),
            RadioGroup::horizontal("remote-desktop-backend-preference")
                .selected_index(selected_index)
                .on_click(cx.listener(|this, index, _, cx| {
                    if let Some(preference) =
                        super::backend_preference::backend_preferences().get(*index)
                    {
                        this.backend_preference = *preference;
                        cx.notify();
                    }
                }))
                .children([
                    Radio::new("remote-desktop-backend-windows-native")
                        .label(if native_available {
                            t!("RemoteDesktopForm.backend_windows_native").to_string()
                        } else {
                            format!(
                                "{} ({})",
                                t!("RemoteDesktopForm.backend_windows_native"),
                                t!("RemoteDesktopForm.backend_windows_native_status")
                            )
                        })
                        .disabled(!native_available),
                    Radio::new("remote-desktop-backend-ironrdp")
                        .label(t!("RemoteDesktopForm.backend_ironrdp").to_string()),
                ]),
        )
    }

    fn render_sync_row(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let label = t!("ConnectionForm.cloud_sync").to_string();
        let desc = t!("ConnectionForm.cloud_sync_desc").to_string();
        h_flex()
            .gap_3()
            .items_center()
            .child(div().w(px(100.0)).text_sm().text_right().child(label))
            .child(
                div().flex_1().child(
                    h_flex()
                        .gap_2()
                        .child(
                            Checkbox::new("remote-desktop-sync-enabled")
                                .checked(self.sync_enabled)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.sync_enabled = !this.sync_enabled;
                                    cx.notify();
                                })),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(desc),
                        ),
                ),
            )
    }
}

/// 键盘接管策略的读取逻辑独立出来，方便单测覆盖「未设置时取默认值」。
pub(super) fn keyboard_hook_mode_for(settings: Option<&RdpSettings>) -> RdpKeyboardHookMode {
    settings
        .map(|settings| settings.input.keyboard_hook)
        .unwrap_or_default()
}

/// 写入键盘接管策略；旧连接还没有 `RdpSettings` 时按当前音频开关补一份默认设置，
/// 避免为了改一个枚举就把整份 RDP 设置丢掉。
pub(super) fn apply_keyboard_hook_mode(
    settings: &mut Option<RdpSettings>,
    audio_playback: bool,
    mode: RdpKeyboardHookMode,
) {
    settings
        .get_or_insert_with(|| RdpSettings::from_legacy_audio_playback(audio_playback))
        .input
        .keyboard_hook = mode;
}

/// 是否已经切到「兼容旧服务端」的安全策略：关闭 CredSSP 并把认证级别放宽到 2。
pub(super) fn legacy_security_compatible_for(settings: Option<&RdpSettings>) -> bool {
    settings
        .map(|settings| {
            !settings.security.enable_credssp && settings.security.authentication_level >= 2
        })
        .unwrap_or(false)
}

/// 写入兼容安全策略；关闭时恢复默认的 CredSSP + 强制认证。
pub(super) fn apply_legacy_security_compatible(
    settings: &mut Option<RdpSettings>,
    audio_playback: bool,
    enabled: bool,
) {
    let security = &mut settings
        .get_or_insert_with(|| RdpSettings::from_legacy_audio_playback(audio_playback))
        .security;
    security.enable_credssp = !enabled;
    security.authentication_level = if enabled { 2 } else { 0 };
    security.public_mode = enabled;
}

impl Focusable for RemoteDesktopFormWindow {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for RemoteDesktopFormWindow {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .justify_center()
            .size_full()
            .child(
                div()
                    .flex_1()
                    .h_full()
                    .min_h_0()
                    .min_w_0()
                    .overflow_hidden()
                    .child(
                        div()
                            .size_full()
                            .p_3()
                            .overflow_y_scrollbar()
                            .child(self.render_body(cx)),
                    ),
            )
            .when_some(self.error.clone(), |this, error| {
                this.child(
                    h_flex()
                        .justify_center()
                        .pb_2()
                        .child(div().text_sm().text_color(cx.theme().danger).child(error)),
                )
            })
            .when_some(self.connection_test.result().cloned(), |this, result| {
                this.child(self.render_connection_test_result(result, cx))
            })
            .child(self.render_footer(cx))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn remote_desktop_form_keeps_scrollbar_inside_explicit_flex_clip_boundary() {
        let source = include_str!("view.rs");
        let render_source = source.split("#[cfg(test)]").next().unwrap();

        assert!(render_source.contains(".min_h_0()"));
        assert!(render_source.contains(".min_w_0()"));
        assert!(render_source.contains(".overflow_hidden()"));
        assert!(render_source.matches(".size_full()").count() >= 2);
        assert!(render_source.contains(".overflow_y_scrollbar()"));
    }

    #[test]
    fn rdp_specific_controls_are_only_rendered_for_rdp() {
        let source = include_str!("view.rs").replace("\r\n", "\n");
        let render_source = source.split("#[cfg(test)]").next().unwrap();

        assert!(render_source.contains("self.protocol == RemoteDesktopProtocol::Rdp"));
        assert!(render_source.contains("self.render_audio_playback_row(cx)"));
        assert!(render_source.contains("remote-desktop-audio-playback"));
        assert!(render_source.contains("RemoteDesktopForm.label_audio_playback"));
        assert!(render_source.contains("self.render_backend_preference_row(cx)"));
        assert!(render_source.contains(
            "#[cfg(windows)]\n                let form = form.child(self.render_backend_preference_row(cx));"
        ));
        assert!(render_source.contains("RemoteDesktopForm.label_backend_preference"));
        assert!(render_source.contains("RadioGroup::horizontal"));
        assert!(render_source.contains("remote-desktop-backend-windows-native"));
        assert!(render_source.contains(".disabled(!native_available)"));
        assert!(render_source.contains("windows_native_rdp_available"));
        assert!(render_source.contains("RemoteDesktopForm.backend_windows_native_status"));
        assert!(render_source.contains("remote-desktop-backend-ironrdp"));
        assert!(!render_source.contains("Select::new(&self.backend_preference_select)"));
    }
}
