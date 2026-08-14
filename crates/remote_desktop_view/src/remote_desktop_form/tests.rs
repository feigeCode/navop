use one_core::storage::RemoteDesktopProtocol;

use super::audio_playback_for_protocol;

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
