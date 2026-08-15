use super::{
    RdpAudioMode, RdpAudioQuality, RdpDisplayMode, RdpGatewayCredentialSource, RdpGatewayMode,
    RdpKeyboardHookMode, RdpNetworkConnectionType, RdpPerformancePreset, RdpSettings,
    RemoteDesktopParams,
};

#[test]
fn legacy_remote_desktop_json_preserves_disabled_audio() {
    let params: RemoteDesktopParams = serde_json::from_str(
        r#"{
            "protocol":"Rdp",
            "host":"legacy.example",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "audio_playback":false
        }"#,
    )
    .unwrap();

    assert!(params.rdp.is_none());
    assert_eq!(
        RdpAudioMode::Disabled,
        params.effective_rdp_settings().audio.mode
    );
}

#[test]
fn legacy_remote_desktop_json_preserves_enabled_audio() {
    let params: RemoteDesktopParams = serde_json::from_str(
        r#"{
            "protocol":"Rdp",
            "host":"legacy.example",
            "port":3389,
            "username":null,
            "password":null,
            "domain":null,
            "audio_playback":true
        }"#,
    )
    .unwrap();

    assert_eq!(
        RdpAudioMode::Local,
        params.effective_rdp_settings().audio.mode
    );
}

#[test]
fn rdp_settings_defaults_match_native_product_policy() {
    let settings = RdpSettings::default();

    assert!(!settings.admin_session);
    assert_eq!(RdpDisplayMode::Dynamic, settings.display.mode);
    assert!(!settings.display.smart_sizing);
    assert!(settings.resources.clipboard);
    assert!(!settings.resources.drives);
    assert!(settings.resources.shared_folders.is_empty());
    assert_eq!(RdpAudioMode::Local, settings.audio.mode);
    assert_eq!(RdpAudioQuality::Dynamic, settings.audio.quality);
    assert!(!settings.audio.capture);
    assert_eq!(RdpKeyboardHookMode::Focused, settings.input.keyboard_hook);
    assert!(settings.security.enable_credssp);
    assert_eq!(0, settings.security.authentication_level);
    assert_eq!(RdpGatewayMode::Disabled, settings.gateway.mode);
    assert_eq!(60, settings.connection.keep_alive_seconds);
    assert_eq!(600, settings.connection.timeout_seconds);
}

#[test]
fn complete_rdp_settings_round_trip_without_losing_credentials_or_policy() {
    let mut settings = RdpSettings::default();
    settings.admin_session = true;
    settings.display.mode = RdpDisplayMode::Fixed;
    settings.display.width = 2560;
    settings.display.height = 1440;
    settings.display.smart_sizing = true;
    settings.display.use_multimon = true;
    settings.display.desktop_scale_factor = 150;
    settings.display.device_scale_factor = 140;
    settings.resources.clipboard = false;
    settings.resources.drives = true;
    settings.resources.dynamic_drives = true;
    settings.resources.printers = true;
    settings.resources.serial_ports = true;
    settings.resources.smart_cards = true;
    settings.resources.cameras = true;
    settings.resources.microphones = true;
    settings
        .resources
        .shared_folders
        .push(super::RdpSharedFolder {
            name: "workspace".to_string(),
            path: "D:\\workspace".to_string(),
            read_only: true,
        });
    settings.audio.mode = RdpAudioMode::Remote;
    settings.audio.quality = RdpAudioQuality::High;
    settings.audio.capture = true;
    settings.performance.preset = RdpPerformancePreset::Custom;
    settings.performance.wallpaper = false;
    settings.performance.font_smoothing = true;
    settings.performance.network_connection_type = RdpNetworkConnectionType::Wan;
    settings.input.keyboard_hook = RdpKeyboardHookMode::Fullscreen;
    settings.security.authentication_level = 2;
    settings.gateway.mode = RdpGatewayMode::Explicit;
    settings.gateway.hostname = Some("gateway.example".to_string());
    settings.gateway.username = Some("gateway-user".to_string());
    settings.gateway.password = Some("gateway-secret".to_string());
    settings.gateway.domain = Some("CORP".to_string());
    settings.gateway.credential_source = RdpGatewayCredentialSource::Password;
    settings.connection.keep_alive_seconds = 30;
    settings.connection.timeout_seconds = 90;

    let json = serde_json::to_string(&settings).unwrap();
    let restored: RdpSettings = serde_json::from_str(&json).unwrap();

    assert_eq!(settings, restored);
    assert_eq!(Some("gateway-secret"), restored.gateway.password.as_deref());
}

#[test]
fn gateway_debug_output_redacts_credentials() {
    let mut settings = RdpSettings::default();
    settings.gateway.mode = RdpGatewayMode::Explicit;
    settings.gateway.hostname = Some("gateway.private.example".to_string());
    settings.gateway.username = Some("gateway-user".to_string());
    settings.gateway.password = Some("gateway-secret".to_string());
    settings.gateway.domain = Some("PRIVATE".to_string());

    let debug = format!("{:?}", settings.gateway);

    assert!(debug.contains("hostname_present: true"));
    assert!(debug.contains("password_present: true"));
    assert!(!debug.contains("gateway.private.example"));
    assert!(!debug.contains("gateway-user"));
    assert!(!debug.contains("gateway-secret"));
    assert!(!debug.contains("PRIVATE"));
}

#[test]
fn stored_connection_round_trips_complete_rdp_settings() {
    use super::{RemoteDesktopProtocol, StoredConnection};

    let mut settings = RdpSettings::default();
    settings.display.smart_sizing = true;
    settings.display.use_multimon = true;
    settings.resources.clipboard = false;
    settings.resources.drives = true;
    settings.security.authentication_level = 2;
    settings.gateway.mode = RdpGatewayMode::Explicit;
    settings.gateway.hostname = Some("gateway.example".to_string());
    settings.connection.max_reconnect_attempts = 200;
    settings.connection.keep_alive_seconds = 30;

    let params = RemoteDesktopParams {
        protocol: RemoteDesktopProtocol::Rdp,
        host: "rdp.example".to_string(),
        port: 3389,
        username: Some("alice".to_string()),
        password: Some("secret".to_string()),
        domain: Some("CORP".to_string()),
        read_only: false,
        audio_playback: true,
        proxy: None,
        backend_preference: Default::default(),
        rdp: Some(settings.clone()),
    };
    let connection = StoredConnection::new_remote_desktop("RDP".to_string(), params, None);
    let restored = connection
        .to_remote_desktop_params()
        .expect("stored params must round trip");

    assert_eq!(Some(settings), restored.rdp);
    assert_eq!("rdp.example", restored.host);
    assert_eq!(Some("alice".to_string()), restored.username);
    assert_eq!(Some("CORP".to_string()), restored.domain);
}

#[test]
fn stored_connection_round_trip_keeps_legacy_rdp_settings_none() {
    use super::{RemoteDesktopProtocol, StoredConnection};

    let params = RemoteDesktopParams {
        protocol: RemoteDesktopProtocol::Rdp,
        host: "legacy.example".to_string(),
        port: 3389,
        username: None,
        password: None,
        domain: None,
        read_only: false,
        audio_playback: true,
        proxy: None,
        backend_preference: Default::default(),
        rdp: None,
    };
    let connection = StoredConnection::new_remote_desktop("Legacy".to_string(), params, None);
    let restored = connection
        .to_remote_desktop_params()
        .expect("stored params must round trip");

    assert!(restored.rdp.is_none());
    assert_eq!(
        RdpAudioMode::Local,
        restored.effective_rdp_settings().audio.mode
    );
}
