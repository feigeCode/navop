use super::*;
use crate::ffi::{
    AUDIO_FLAG_CAPTURE, CONNECTION_FLAG_ADMIN_SESSION, CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED,
    CONNECTION_FLAG_AUTO_RECONNECT, DISPLAY_FLAG_SMART_SIZING, DISPLAY_FLAG_SPAN_MONITORS,
    DISPLAY_FLAG_USE_MULTIMON, INPUT_FLAG_ENABLE_WINDOWS_KEY, INPUT_FLAG_GRAB_FOCUS_ON_CONNECT,
    PERFORMANCE_FLAG_BITMAP_CACHE, PERFORMANCE_FLAG_DESKTOP_COMPOSITION,
    PERFORMANCE_FLAG_FONT_SMOOTHING, RESOURCE_FLAG_CAMERAS, RESOURCE_FLAG_CLIPBOARD,
    RESOURCE_FLAG_DRIVES, RESOURCE_FLAG_MICROPHONES, SECURITY_FLAG_ENABLE_CREDSSP,
    SECURITY_FLAG_ENCRYPTION_ENABLED,
};
use crate::policy::{
    WindowsRdpAudioMode, WindowsRdpAudioPolicy, WindowsRdpAudioQuality, WindowsRdpConnectionPolicy,
    WindowsRdpDisplayMode, WindowsRdpDisplayPolicy, WindowsRdpGatewayCredentialSource,
    WindowsRdpGatewayMode, WindowsRdpGatewayPolicy, WindowsRdpInputPolicy,
    WindowsRdpKeyboardHookMode, WindowsRdpNetworkConnectionType, WindowsRdpPerformancePolicy,
    WindowsRdpPerformancePreset, WindowsRdpReconnectPolicy, WindowsRdpResourcePolicy,
    WindowsRdpSecurityPolicy,
};

fn valid_options(
    host: impl Into<String>,
) -> Result<WindowsRdpConnectionOptions, WindowsRdpHostError> {
    WindowsRdpConnectionOptions::new(host, 3389, 1920, 1080, WindowsRdpColorDepth::Bpp32)
}

#[test]
fn connection_options_reject_invalid_host_and_port() {
    assert_eq!(valid_options(""), Err(WindowsRdpHostError::InvalidArgument));
    assert_eq!(
        valid_options("rdp\0host"),
        Err(WindowsRdpHostError::InvalidArgument)
    );
    assert_eq!(
        WindowsRdpConnectionOptions::new("rdp.example", 0, 1920, 1080, WindowsRdpColorDepth::Bpp32,),
        Err(WindowsRdpHostError::InvalidArgument)
    );
}

#[test]
fn connection_options_measure_host_length_in_utf16_code_units() {
    let exact = "a".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS);
    assert!(valid_options(exact).is_ok());

    let over = "a".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS + 1);
    assert_eq!(
        valid_options(over),
        Err(WindowsRdpHostError::InvalidArgument)
    );

    let supplementary = "😀".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS / 2);
    assert!(valid_options(supplementary).is_ok());
    let supplementary_over = "😀".repeat(WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS / 2 + 1);
    assert_eq!(
        valid_options(supplementary_over),
        Err(WindowsRdpHostError::InvalidArgument)
    );
}

#[test]
fn connection_options_reject_invalid_dimensions() {
    for (width, height) in [
        (0, 1080),
        (1920, 0),
        (i32::MAX as u32 + 1, 1080),
        (1920, i32::MAX as u32 + 1),
    ] {
        assert_eq!(
            WindowsRdpConnectionOptions::new(
                "rdp.example",
                3389,
                width,
                height,
                WindowsRdpColorDepth::Bpp32,
            ),
            Err(WindowsRdpHostError::InvalidArgument)
        );
    }
}

#[test]
fn connection_options_preserve_valid_values() {
    let options = WindowsRdpConnectionOptions::new(
        "rdp.example",
        3390,
        1600,
        900,
        WindowsRdpColorDepth::Bpp24,
    )
    .expect("valid connection options");

    assert_eq!(options.host(), "rdp.example");
    assert_eq!(options.port(), 3390);
    assert_eq!(options.desktop_width(), 1600);
    assert_eq!(options.desktop_height(), 900);
    assert_eq!(options.color_depth(), WindowsRdpColorDepth::Bpp24);
}

#[test]
fn connection_options_encode_audio_playback() {
    let default = valid_options("rdp.example").expect("valid connection options");
    assert!(default.audio_playback());
    assert_eq!(default.as_native().expect("native options").native.flags, 0);

    let disabled = default.with_audio_playback(false);
    assert!(!disabled.audio_playback());
    assert_eq!(
        disabled
            .as_native()
            .expect("native options with disabled audio")
            .native
            .flags,
        CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED
    );
}

#[test]
fn default_policy_preserves_legacy_behavior() {
    let options = valid_options("rdp.example").expect("valid connection options");
    let native = options.as_native().expect("native options");

    assert_eq!(options.policy(), &WindowsRdpConnectionPolicy::default());
    assert_eq!(native.native.audio_mode, WindowsRdpAudioMode::Local as u32);
    assert_eq!(
        native.native.security_flags,
        SECURITY_FLAG_ENABLE_CREDSSP | SECURITY_FLAG_ENCRYPTION_ENABLED
    );
    assert_eq!(native.native.resource_flags, RESOURCE_FLAG_CLIPBOARD);
    assert_eq!(
        native.native.input_flags,
        INPUT_FLAG_ENABLE_WINDOWS_KEY | INPUT_FLAG_GRAB_FOCUS_ON_CONNECT
    );
    assert_eq!(
        native.native.connection_flags,
        CONNECTION_FLAG_AUTO_RECONNECT
    );
}

#[test]
fn complete_policy_maps_to_native_fields() {
    let policy = WindowsRdpConnectionPolicy {
        admin_session: true,
        display: WindowsRdpDisplayPolicy {
            mode: WindowsRdpDisplayMode::Fixed,
            smart_sizing: true,
            use_multimon: true,
            span_monitors: true,
            desktop_scale_factor: 140,
            device_scale_factor: 140,
        },
        resources: WindowsRdpResourcePolicy {
            clipboard: true,
            drives: true,
            cameras: true,
            microphones: true,
            ..Default::default()
        },
        audio: WindowsRdpAudioPolicy {
            mode: WindowsRdpAudioMode::Remote,
            quality: WindowsRdpAudioQuality::High,
            capture: true,
        },
        input: WindowsRdpInputPolicy {
            keyboard_hook: WindowsRdpKeyboardHookMode::Fullscreen,
            enable_windows_key: true,
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
            font_smoothing: true,
            desktop_composition: true,
            bitmap_cache: true,
            network_connection_type: WindowsRdpNetworkConnectionType::Lan,
        },
        security: WindowsRdpSecurityPolicy {
            enable_credssp: true,
            authentication_level: 2,
            public_mode: false,
            encryption_enabled: true,
        },
        gateway: WindowsRdpGatewayPolicy {
            mode: WindowsRdpGatewayMode::Explicit,
            bypass_local: true,
            credential_source: WindowsRdpGatewayCredentialSource::Any,
            hostname: Some("gateway.example".to_owned()),
        },
        reconnect: WindowsRdpReconnectPolicy {
            keep_alive_seconds: 30,
            timeout_seconds: 90,
            auto_reconnect: true,
            max_reconnect_attempts: 12,
        },
    };
    let options = valid_options("rdp.example")
        .expect("valid connection options")
        .with_policy(policy.clone());
    let native = options
        .as_native()
        .expect("complete policy should be valid");

    assert_eq!(options.policy(), &policy);
    assert_eq!(
        native.native.display_mode,
        WindowsRdpDisplayMode::Fixed as u32
    );
    assert_eq!(
        native.native.display_flags,
        DISPLAY_FLAG_SMART_SIZING | DISPLAY_FLAG_USE_MULTIMON | DISPLAY_FLAG_SPAN_MONITORS
    );
    assert_eq!(
        native.native.resource_flags,
        RESOURCE_FLAG_CLIPBOARD
            | RESOURCE_FLAG_DRIVES
            | RESOURCE_FLAG_CAMERAS
            | RESOURCE_FLAG_MICROPHONES
    );
    assert_eq!(native.native.audio_mode, WindowsRdpAudioMode::Remote as u32);
    assert_eq!(
        native.native.audio_quality,
        WindowsRdpAudioQuality::High as u32
    );
    assert_eq!(native.native.audio_flags, AUDIO_FLAG_CAPTURE);
    assert_eq!(
        native.native.keyboard_hook_mode,
        WindowsRdpKeyboardHookMode::Fullscreen as u32
    );
    assert_eq!(native.native.input_flags, INPUT_FLAG_ENABLE_WINDOWS_KEY);
    assert_eq!(
        native.native.performance_flags,
        PERFORMANCE_FLAG_FONT_SMOOTHING
            | PERFORMANCE_FLAG_DESKTOP_COMPOSITION
            | PERFORMANCE_FLAG_BITMAP_CACHE
    );
    assert_eq!(native.native.authentication_level, 2);
    assert_eq!(
        native.native.gateway_credential_source,
        WindowsRdpGatewayCredentialSource::Any as u32
    );
    assert_eq!(
        native.native.connection_flags,
        CONNECTION_FLAG_ADMIN_SESSION | CONNECTION_FLAG_AUTO_RECONNECT
    );
    assert_eq!(native.native.keep_alive_seconds, 30);
    assert_eq!(native.native.timeout_seconds, 90);
    assert_eq!(native.native.max_reconnect_attempts, 12);
}

#[test]
fn gateway_hostname_is_borrowed_for_the_native_call_lifetime() {
    let mut policy = WindowsRdpConnectionPolicy::default();
    policy.gateway.mode = WindowsRdpGatewayMode::Explicit;
    policy.gateway.hostname = Some("网关.example".to_owned());
    let options = valid_options("rdp.example")
        .expect("valid connection options")
        .with_policy(policy);
    let native = options.as_native().expect("valid gateway hostname");
    let hostname = unsafe {
        std::slice::from_raw_parts(
            native.native.gateway_hostname.data,
            native.native.gateway_hostname.len as usize,
        )
    };

    assert_eq!(String::from_utf16(hostname).unwrap(), "网关.example");
}

#[test]
fn policy_rejects_invalid_scale_authentication_gateway_and_time_values() {
    let base = valid_options("rdp.example").expect("valid connection options");
    let mutations: [fn(&mut WindowsRdpConnectionPolicy); 5] = [
        |policy| policy.display.desktop_scale_factor = 99,
        |policy| policy.display.device_scale_factor = 120,
        |policy| policy.security.authentication_level = 3,
        |policy| {
            policy.gateway.mode = WindowsRdpGatewayMode::Explicit;
            policy.gateway.hostname = None;
        },
        |policy| policy.reconnect.keep_alive_seconds = u32::MAX,
    ];

    for mutate in mutations {
        let mut policy = WindowsRdpConnectionPolicy::default();
        mutate(&mut policy);
        assert!(matches!(
            base.clone().with_policy(policy).as_native(),
            Err(WindowsRdpHostError::InvalidArgument)
        ));
    }
}

#[test]
fn connection_options_debug_redacts_the_complete_endpoint() {
    let mut policy = WindowsRdpConnectionPolicy::default();
    policy.gateway.mode = WindowsRdpGatewayMode::Explicit;
    policy.gateway.hostname = Some("gateway-debug-sentinel.example".to_owned());
    let options = WindowsRdpConnectionOptions::new(
        "alice@example.com:endpoint-sentinel@[2001:db8::1]",
        3390,
        1600,
        900,
        WindowsRdpColorDepth::Bpp24,
    )
    .expect("valid connection options")
    .with_policy(policy)
    .with_audio_playback(false);
    let debug = format!("{options:?}");

    assert!(debug.contains("WindowsRdpConnectionOptions"));
    assert!(debug.contains("port: 3390"));
    assert!(debug.contains("audio_playback: false"));
    assert!(debug.contains("<redacted"));
    assert!(!debug.contains("alice@example.com"));
    assert!(!debug.contains("endpoint-sentinel"));
    assert!(!debug.contains("2001:db8"));
    assert!(!debug.contains("gateway-debug-sentinel"));
}
