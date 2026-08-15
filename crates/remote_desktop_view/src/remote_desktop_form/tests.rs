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
fn form_apply_and_build_keep_rdp_settings_verbatim() {
    let source = include_str!("../remote_desktop_form.rs").replace("\r\n", "\n");
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

    // The loaded RDP settings must enter form state…
    assert!(apply.contains("self.rdp_settings = params.rdp;"));
    // …and be written back verbatim (never effective defaults)…
    assert!(
        build.contains("rdp: rdp_settings_for_protocol(self.protocol, self.rdp_settings.clone())")
    );
    // …while the field starts empty for new connections.
    assert!(source.contains("rdp_settings: None,"));
    // No code may write `rdp: None` unconditionally any more.
    assert!(!source.contains("rdp: None,"));
}
