use one_core::storage::{RdpSettings, RemoteDesktopProtocol};

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
        .split("\n    fn render_backend_preference_row(")
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
