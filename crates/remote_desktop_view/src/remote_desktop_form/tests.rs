use one_core::storage::{RdpAudioMode, RdpKeyboardHookMode, RdpSettings, RemoteDesktopProtocol};

use super::view::{
    apply_keyboard_hook_mode, apply_legacy_security_compatible, keyboard_hook_mode_for,
    legacy_security_compatible_for,
};
use super::{audio_playback_for_protocol, rdp_settings_for_protocol};

#[test]
fn audio_playback_is_only_enabled_for_rdp() {
    assert!(audio_playback_for_protocol(
        RemoteDesktopProtocol::Rdp,
        true
    ));
    assert!(!audio_playback_for_protocol(
        RemoteDesktopProtocol::Vnc,
        true
    ));
    assert!(!audio_playback_for_protocol(
        RemoteDesktopProtocol::Rdp,
        false
    ));
}

#[test]
fn rdp_settings_are_preserved_verbatim_for_rdp_connections() {
    let custom = RdpSettings::default();
    assert_eq!(
        Some(custom.clone()),
        rdp_settings_for_protocol(RemoteDesktopProtocol::Rdp, Some(custom))
    );
}

#[test]
fn legacy_rdp_without_settings_stays_none() {
    assert_eq!(
        None,
        rdp_settings_for_protocol(RemoteDesktopProtocol::Rdp, None)
    );
}

#[test]
fn switching_to_vnc_clears_rdp_settings() {
    let custom = RdpSettings::default();
    assert_eq!(
        None,
        rdp_settings_for_protocol(RemoteDesktopProtocol::Vnc, Some(custom))
    );
    assert_eq!(
        None,
        rdp_settings_for_protocol(RemoteDesktopProtocol::Vnc, None)
    );
}

#[test]
fn form_audio_checkbox_and_full_rdp_settings_stay_synchronized() {
    let source = include_str!("../remote_desktop_form.rs").replace("\r\n", "\n");
    let view = include_str!("view.rs").replace("\r\n", "\n");
    let apply = source
        .split("fn apply_params(")
        .nth(1)
        .expect("apply_params body")
        .split("\n    fn build_params(")
        .next()
        .expect("apply_params end");
    let build = source
        .split("fn build_params(")
        .nth(1)
        .expect("build_params body")
        .split("\n    fn on_save(")
        .next()
        .expect("build_params end");
    let audio_click = view
        .split("fn render_audio_playback_row(")
        .nth(1)
        .expect("audio row body")
        .split("\n    /// The graphics pipeline policy")
        .next()
        .expect("audio row end");

    // Loaded full settings are the source of truth for the binary checkbox.
    assert!(apply.contains("self.rdp_settings = params.rdp;"));
    assert!(apply.contains("settings.audio.mode == RdpAudioMode::Local"));
    assert!(
        apply.contains("audio_playback_for_protocol(self.protocol, params.audio_playback)"),
        "legacy connections without full RDP settings must keep using the legacy bool"
    );

    // Clicking the binary control is the only point where the full audio mode
    // is intentionally collapsed to Local/Disabled. An untouched Remote mode
    // therefore remains verbatim in self.rdp_settings.
    assert!(audio_click.contains("this.rdp_settings.as_mut()"));
    assert!(audio_click.contains("RdpAudioMode::Local"));
    assert!(audio_click.contains("RdpAudioMode::Disabled"));
    assert!(!audio_click.contains("RdpAudioMode::Remote"));

    // The legacy bool written alongside full settings must describe the same
    // effective local-playback state.
    assert!(build.contains("settings.audio.mode == RdpAudioMode::Local"));
    assert!(build.contains(".unwrap_or(self.audio_playback)"));
    assert!(
        build.contains("rdp: rdp_settings_for_protocol(self.protocol, self.rdp_settings.clone())")
    );

    // New/legacy connections remain None rather than materializing defaults,
    // and switching protocols still clears all RDP-only settings.
    assert!(source.contains("rdp_settings: None,"));
    assert!(!source.contains("rdp: None,"));
}

#[test]
fn form_offers_the_graphics_pipeline_policy_per_connection() {
    let view = include_str!("view.rs").replace("\r\n", "\n");
    let click = view
        .split("fn set_egfx_mode(")
        .nth(1)
        .expect("set_egfx_mode body")
        .split("\n    fn render_egfx_row(")
        .next()
        .expect("set_egfx_mode end");
    let row = view
        .split("fn render_egfx_row(")
        .nth(1)
        .expect("egfx row body")
        .split("\n    #[cfg(windows)]")
        .next()
        .expect("egfx row end");

    // The policy lives in the full RDP settings, so writing it must not reset the audio mode of a
    // connection that only ever stored the legacy bool.
    assert!(click.contains("RdpSettings::from_legacy_audio_playback(audio_playback)"));
    assert!(click.contains("settings.graphics.egfx = mode;"));

    // All three states are reachable from the form.
    for id in [
        "remote-desktop-egfx-auto",
        "remote-desktop-egfx-always",
        "remote-desktop-egfx-never",
    ] {
        assert!(row.contains(id), "the form must offer {id}");
    }
    assert!(row.contains("RdpEgfxMode::Auto"));
    assert!(row.contains("RdpEgfxMode::Always"));
    assert!(row.contains("RdpEgfxMode::Never"));

    // RDP connections render it, and only when the helper is the backend that runs.
    assert!(view.contains("self.egfx_controls_visible()"));
    assert!(view.contains("form.child(self.render_egfx_row(cx))"));
}

#[test]
fn keyboard_hook_defaults_to_focused_when_settings_are_missing() {
    assert_eq!(RdpKeyboardHookMode::Focused, keyboard_hook_mode_for(None));
    assert_eq!(
        RdpKeyboardHookMode::Focused,
        keyboard_hook_mode_for(Some(&RdpSettings::default()))
    );
}

#[test]
fn keyboard_hook_choice_creates_settings_and_keeps_other_fields() {
    let mut settings = None;
    apply_keyboard_hook_mode(&mut settings, true, RdpKeyboardHookMode::Local);

    let mut settings = settings.expect("应补出一份设置");
    assert_eq!(RdpKeyboardHookMode::Local, settings.input.keyboard_hook);
    // 旧连接没有设置时按当前音频开关补默认值，音频不能被这次写入改掉。
    assert_eq!(RdpAudioMode::Local, settings.audio.mode);

    let mut settings = Some(settings);
    apply_keyboard_hook_mode(&mut settings, false, RdpKeyboardHookMode::Fullscreen);
    let settings = settings.expect("设置仍在");
    assert_eq!(
        RdpKeyboardHookMode::Fullscreen,
        settings.input.keyboard_hook
    );
}

#[test]
fn legacy_security_is_off_by_default() {
    // 默认策略必须仍然要求 CredSSP，兼容模式只能由用户显式打开。
    assert!(!legacy_security_compatible_for(None));
    assert!(!legacy_security_compatible_for(Some(
        &RdpSettings::default()
    )));
}

#[test]
fn enabling_legacy_security_relaxes_credssp_and_authentication() {
    let mut settings: Option<RdpSettings> = None;

    apply_legacy_security_compatible(&mut settings, true, true);

    let settings = settings.expect("settings created");
    assert!(!settings.security.enable_credssp);
    assert_eq!(2, settings.security.authentication_level);
    assert!(settings.security.public_mode);
    // 其它字段沿用默认值，不能因为改一个开关就丢掉音频等设置。
    assert_eq!(RdpAudioMode::Local, settings.audio.mode);
    assert!(legacy_security_compatible_for(Some(&settings)));
}

#[test]
fn disabling_legacy_security_restores_the_default_policy() {
    let mut settings: Option<RdpSettings> = None;
    apply_legacy_security_compatible(&mut settings, false, true);

    apply_legacy_security_compatible(&mut settings, false, false);

    let settings = settings.expect("settings kept");
    assert!(settings.security.enable_credssp);
    assert_eq!(0, settings.security.authentication_level);
    assert!(!settings.security.public_mode);
    assert!(!legacy_security_compatible_for(Some(&settings)));
}
