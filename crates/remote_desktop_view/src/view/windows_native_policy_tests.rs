use one_core::storage::{
    RdpAudioMode, RdpAudioQuality, RdpDisplayMode, RdpGatewayCredentialSource, RdpGatewayMode,
    RdpKeyboardHookMode, RdpNetworkConnectionType, RdpPerformancePreset, RdpSettings,
};
use windows_rdp_host::{
    WindowsRdpAudioMode, WindowsRdpAudioPolicy, WindowsRdpAudioQuality, WindowsRdpConnectionPolicy,
    WindowsRdpCredentialBundle, WindowsRdpDisplayMode, WindowsRdpDisplayPolicy,
    WindowsRdpGatewayCredentialSource, WindowsRdpGatewayMode, WindowsRdpGatewayPolicy,
    WindowsRdpInputPolicy, WindowsRdpKeyboardHookMode, WindowsRdpNetworkConnectionType,
    WindowsRdpPerformancePolicy, WindowsRdpPerformancePreset, WindowsRdpReconnectPolicy,
    WindowsRdpResourcePolicy, WindowsRdpSecurityPolicy,
};

use super::{
    apply_gateway_credentials, connection_policy, initial_desktop_size,
    uses_dynamic_display_updates,
};

fn customized_settings() -> RdpSettings {
    let mut settings = RdpSettings::default();
    settings.admin_session = true;
    settings.display.mode = RdpDisplayMode::Fixed;
    settings.display.width = 2560;
    settings.display.height = 1440;
    settings.display.smart_sizing = true;
    settings.display.use_multimon = true;
    settings.display.span_monitors = true;
    settings.display.desktop_scale_factor = 180;
    settings.display.device_scale_factor = 180;
    settings.resources.clipboard = false;
    settings.resources.drives = true;
    settings.resources.dynamic_drives = true;
    settings.resources.dynamic_devices = true;
    settings.resources.printers = true;
    settings.resources.serial_ports = true;
    settings.resources.smart_cards = true;
    settings.resources.cameras = true;
    settings.resources.microphones = true;
    settings.resources.pos_devices = true;
    settings.audio.mode = RdpAudioMode::Remote;
    settings.audio.quality = RdpAudioQuality::High;
    settings.audio.capture = true;
    settings.input.keyboard_hook = RdpKeyboardHookMode::Fullscreen;
    settings.input.enable_windows_key = false;
    settings.input.grab_focus_on_connect = false;
    settings.performance.preset = RdpPerformancePreset::Custom;
    settings.performance.wallpaper = false;
    settings.performance.full_window_drag = false;
    settings.performance.menu_animations = false;
    settings.performance.themes = false;
    settings.performance.cursor_shadow = false;
    settings.performance.cursor_settings = false;
    settings.performance.font_smoothing = false;
    settings.performance.desktop_composition = false;
    settings.performance.bitmap_cache = false;
    settings.performance.network_connection_type = RdpNetworkConnectionType::Lan;
    settings.security.enable_credssp = false;
    settings.security.authentication_level = 2;
    settings.security.public_mode = true;
    settings.security.encryption_enabled = false;
    settings.gateway.mode = RdpGatewayMode::Explicit;
    settings.gateway.bypass_local = false;
    settings.gateway.credential_source = RdpGatewayCredentialSource::Any;
    settings.gateway.hostname = Some("gateway.example.test".to_owned());
    settings.connection.keep_alive_seconds = 45;
    settings.connection.timeout_seconds = 90;
    settings.connection.auto_reconnect = false;
    settings.connection.max_reconnect_attempts = 7;
    settings
}

#[test]
fn maps_complete_rdp_settings_into_native_policy() {
    let settings = customized_settings();

    assert_eq!(
        connection_policy(&settings),
        WindowsRdpConnectionPolicy {
            admin_session: true,
            display: WindowsRdpDisplayPolicy {
                mode: WindowsRdpDisplayMode::Fixed,
                smart_sizing: true,
                use_multimon: true,
                span_monitors: true,
                desktop_scale_factor: 180,
                device_scale_factor: 180,
            },
            resources: WindowsRdpResourcePolicy {
                clipboard: false,
                drives: true,
                dynamic_drives: true,
                dynamic_devices: true,
                printers: true,
                serial_ports: true,
                smart_cards: true,
                cameras: true,
                microphones: true,
                pos_devices: true,
            },
            audio: WindowsRdpAudioPolicy {
                mode: WindowsRdpAudioMode::Remote,
                quality: WindowsRdpAudioQuality::High,
                capture: true,
            },
            input: WindowsRdpInputPolicy {
                keyboard_hook: WindowsRdpKeyboardHookMode::Fullscreen,
                enable_windows_key: false,
                grab_focus_on_connect: false,
            },
            performance: WindowsRdpPerformancePolicy {
                preset: WindowsRdpPerformancePreset::Custom,
                wallpaper: false,
                full_window_drag: false,
                menu_animations: false,
                themes: false,
                cursor_shadow: false,
                cursor_settings: false,
                font_smoothing: false,
                desktop_composition: false,
                bitmap_cache: false,
                network_connection_type: WindowsRdpNetworkConnectionType::Lan,
            },
            security: WindowsRdpSecurityPolicy {
                enable_credssp: false,
                authentication_level: 2,
                public_mode: true,
                encryption_enabled: false,
            },
            gateway: WindowsRdpGatewayPolicy {
                mode: WindowsRdpGatewayMode::Explicit,
                bypass_local: false,
                credential_source: WindowsRdpGatewayCredentialSource::Any,
                hostname: Some("gateway.example.test".to_owned()),
            },
            reconnect: WindowsRdpReconnectPolicy {
                keep_alive_seconds: 45,
                timeout_seconds: 90,
                auto_reconnect: false,
                max_reconnect_attempts: 7,
            },
        }
    );
}

#[test]
fn fixed_display_uses_configured_desktop_size() {
    let settings = customized_settings();

    assert_eq!(initial_desktop_size(&settings, (1280, 720)), (2560, 1440));
    assert!(!uses_dynamic_display_updates(&settings));
}

#[test]
fn dynamic_display_uses_current_viewport_size() {
    let mut settings = customized_settings();
    settings.display.mode = RdpDisplayMode::Dynamic;

    assert_eq!(initial_desktop_size(&settings, (1280, 720)), (1280, 720));
    assert!(uses_dynamic_display_updates(&settings));
}

#[test]
fn gateway_credentials_remain_separate_from_server_credentials() {
    let mut settings = customized_settings();
    settings.gateway.username = Some("gateway-user".to_owned());
    settings.gateway.domain = Some("GATEWAY".to_owned());
    settings.gateway.password = Some("gateway-secret".to_owned());
    let mut credentials = WindowsRdpCredentialBundle::new()
        .with_username("server-user".to_owned())
        .with_domain("SERVER-DOMAIN".to_owned())
        .with_server_password("server-secret-long".to_owned());

    apply_gateway_credentials(&mut credentials, &settings);

    let debug = format!("{credentials:?}");
    assert!(debug.contains("username: \"<present, 11 UTF-16 code units>\""));
    assert!(debug.contains("domain: \"<present, 13 UTF-16 code units>\""));
    assert!(debug.contains("gateway_username: \"<present, 12 UTF-16 code units>\""));
    assert!(debug.contains("gateway_domain: \"<present, 7 UTF-16 code units>\""));
    assert!(debug.contains("server_password: \"<redacted, 18 UTF-16 code units>\""));
    assert!(debug.contains("gateway_password: \"<redacted, 14 UTF-16 code units>\""));
}
