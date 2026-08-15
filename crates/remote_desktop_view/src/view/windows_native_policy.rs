use one_core::storage::{
    RdpAudioMode, RdpAudioQuality, RdpDisplayMode, RdpGatewayCredentialSource, RdpGatewayMode,
    RdpKeyboardHookMode, RdpNetworkConnectionType, RdpPerformancePreset, RdpSettings,
};
use windows_rdp_host::{
    WindowsRdpAudioMode, WindowsRdpAudioPolicy, WindowsRdpAudioQuality,
    WindowsRdpConnectionPolicy, WindowsRdpCredentialBundle, WindowsRdpDisplayMode,
    WindowsRdpDisplayPolicy, WindowsRdpGatewayCredentialSource, WindowsRdpGatewayMode,
    WindowsRdpGatewayPolicy, WindowsRdpInputPolicy, WindowsRdpKeyboardHookMode,
    WindowsRdpNetworkConnectionType, WindowsRdpPerformancePolicy, WindowsRdpPerformancePreset,
    WindowsRdpReconnectPolicy, WindowsRdpResourcePolicy, WindowsRdpSecurityPolicy,
};

pub(super) fn connection_policy(settings: &RdpSettings) -> WindowsRdpConnectionPolicy {
    WindowsRdpConnectionPolicy {
        admin_session: settings.admin_session,
        display: display_policy(settings),
        resources: resource_policy(settings),
        audio: audio_policy(settings),
        input: input_policy(settings),
        performance: performance_policy(settings),
        security: security_policy(settings),
        gateway: gateway_policy(settings),
        reconnect: reconnect_policy(settings),
    }
}

pub(super) fn initial_desktop_size(
    settings: &RdpSettings,
    viewport: (u16, u16),
) -> (u32, u32) {
    match settings.display.mode {
        RdpDisplayMode::Dynamic => (u32::from(viewport.0), u32::from(viewport.1)),
        RdpDisplayMode::Fixed => (settings.display.width, settings.display.height),
    }
}

pub(super) fn uses_dynamic_display_updates(settings: &RdpSettings) -> bool {
    settings.display.mode == RdpDisplayMode::Dynamic
}

pub(super) fn apply_gateway_credentials(
    credentials: &mut WindowsRdpCredentialBundle,
    settings: &RdpSettings,
) {
    set_optional_gateway_username(credentials, settings.gateway.username.as_ref());
    set_optional_gateway_domain(credentials, settings.gateway.domain.as_ref());
    set_optional_gateway_password(credentials, settings.gateway.password.as_ref());
}

fn display_policy(settings: &RdpSettings) -> WindowsRdpDisplayPolicy {
    WindowsRdpDisplayPolicy {
        mode: match settings.display.mode {
            RdpDisplayMode::Dynamic => WindowsRdpDisplayMode::Dynamic,
            RdpDisplayMode::Fixed => WindowsRdpDisplayMode::Fixed,
        },
        smart_sizing: settings.display.smart_sizing,
        use_multimon: settings.display.use_multimon,
        span_monitors: settings.display.span_monitors,
        desktop_scale_factor: settings.display.desktop_scale_factor,
        device_scale_factor: settings.display.device_scale_factor,
    }
}

fn resource_policy(settings: &RdpSettings) -> WindowsRdpResourcePolicy {
    let resources = &settings.resources;
    WindowsRdpResourcePolicy {
        clipboard: resources.clipboard,
        drives: resources.drives,
        dynamic_drives: resources.dynamic_drives,
        dynamic_devices: resources.dynamic_devices,
        printers: resources.printers,
        serial_ports: resources.serial_ports,
        smart_cards: resources.smart_cards,
        cameras: resources.cameras,
        microphones: resources.microphones,
        pos_devices: resources.pos_devices,
    }
}

fn audio_policy(settings: &RdpSettings) -> WindowsRdpAudioPolicy {
    WindowsRdpAudioPolicy {
        mode: match settings.audio.mode {
            RdpAudioMode::Local => WindowsRdpAudioMode::Local,
            RdpAudioMode::Remote => WindowsRdpAudioMode::Remote,
            RdpAudioMode::Disabled => WindowsRdpAudioMode::Disabled,
        },
        quality: match settings.audio.quality {
            RdpAudioQuality::Dynamic => WindowsRdpAudioQuality::Dynamic,
            RdpAudioQuality::Medium => WindowsRdpAudioQuality::Medium,
            RdpAudioQuality::High => WindowsRdpAudioQuality::High,
        },
        capture: settings.audio.capture,
    }
}

fn input_policy(settings: &RdpSettings) -> WindowsRdpInputPolicy {
    WindowsRdpInputPolicy {
        keyboard_hook: match settings.input.keyboard_hook {
            RdpKeyboardHookMode::Local => WindowsRdpKeyboardHookMode::Local,
            RdpKeyboardHookMode::Focused => WindowsRdpKeyboardHookMode::Focused,
            RdpKeyboardHookMode::Fullscreen => WindowsRdpKeyboardHookMode::Fullscreen,
        },
        enable_windows_key: settings.input.enable_windows_key,
        grab_focus_on_connect: settings.input.grab_focus_on_connect,
    }
}

fn performance_policy(settings: &RdpSettings) -> WindowsRdpPerformancePolicy {
    let performance = &settings.performance;
    WindowsRdpPerformancePolicy {
        preset: performance_preset(performance.preset),
        wallpaper: performance.wallpaper,
        full_window_drag: performance.full_window_drag,
        menu_animations: performance.menu_animations,
        themes: performance.themes,
        cursor_shadow: performance.cursor_shadow,
        cursor_settings: performance.cursor_settings,
        font_smoothing: performance.font_smoothing,
        desktop_composition: performance.desktop_composition,
        bitmap_cache: performance.bitmap_cache,
        network_connection_type: network_connection_type(performance.network_connection_type),
    }
}

fn performance_preset(preset: RdpPerformancePreset) -> WindowsRdpPerformancePreset {
    match preset {
        RdpPerformancePreset::Auto => WindowsRdpPerformancePreset::Auto,
        RdpPerformancePreset::Low => WindowsRdpPerformancePreset::Low,
        RdpPerformancePreset::Medium => WindowsRdpPerformancePreset::Medium,
        RdpPerformancePreset::High => WindowsRdpPerformancePreset::High,
        RdpPerformancePreset::Custom => WindowsRdpPerformancePreset::Custom,
    }
}

fn network_connection_type(
    connection_type: RdpNetworkConnectionType,
) -> WindowsRdpNetworkConnectionType {
    match connection_type {
        RdpNetworkConnectionType::Modem => WindowsRdpNetworkConnectionType::Modem,
        RdpNetworkConnectionType::BroadbandLow => WindowsRdpNetworkConnectionType::BroadbandLow,
        RdpNetworkConnectionType::Satellite => WindowsRdpNetworkConnectionType::Satellite,
        RdpNetworkConnectionType::BroadbandHigh => WindowsRdpNetworkConnectionType::BroadbandHigh,
        RdpNetworkConnectionType::Wan => WindowsRdpNetworkConnectionType::Wan,
        RdpNetworkConnectionType::Lan => WindowsRdpNetworkConnectionType::Lan,
        RdpNetworkConnectionType::Auto => WindowsRdpNetworkConnectionType::Auto,
    }
}

fn security_policy(settings: &RdpSettings) -> WindowsRdpSecurityPolicy {
    WindowsRdpSecurityPolicy {
        enable_credssp: settings.security.enable_credssp,
        authentication_level: settings.security.authentication_level,
        public_mode: settings.security.public_mode,
        encryption_enabled: settings.security.encryption_enabled,
    }
}

fn gateway_policy(settings: &RdpSettings) -> WindowsRdpGatewayPolicy {
    WindowsRdpGatewayPolicy {
        mode: match settings.gateway.mode {
            RdpGatewayMode::Disabled => WindowsRdpGatewayMode::Disabled,
            RdpGatewayMode::Explicit => WindowsRdpGatewayMode::Explicit,
            RdpGatewayMode::AutoDetect => WindowsRdpGatewayMode::AutoDetect,
        },
        bypass_local: settings.gateway.bypass_local,
        credential_source: match settings.gateway.credential_source {
            RdpGatewayCredentialSource::Password => WindowsRdpGatewayCredentialSource::Password,
            RdpGatewayCredentialSource::SmartCard => WindowsRdpGatewayCredentialSource::SmartCard,
            RdpGatewayCredentialSource::Any => WindowsRdpGatewayCredentialSource::Any,
        },
        hostname: settings.gateway.hostname.clone(),
    }
}

fn reconnect_policy(settings: &RdpSettings) -> WindowsRdpReconnectPolicy {
    WindowsRdpReconnectPolicy {
        keep_alive_seconds: settings.connection.keep_alive_seconds,
        timeout_seconds: settings.connection.timeout_seconds,
        auto_reconnect: settings.connection.auto_reconnect,
        max_reconnect_attempts: settings.connection.max_reconnect_attempts,
    }
}

fn set_optional_gateway_username(
    credentials: &mut WindowsRdpCredentialBundle,
    username: Option<&String>,
) {
    match username {
        Some(username) => credentials.set_gateway_username(username.clone()),
        None => credentials.clear_gateway_username(),
    }
}

fn set_optional_gateway_domain(
    credentials: &mut WindowsRdpCredentialBundle,
    domain: Option<&String>,
) {
    match domain {
        Some(domain) => credentials.set_gateway_domain(domain.clone()),
        None => credentials.clear_gateway_domain(),
    }
}

fn set_optional_gateway_password(
    credentials: &mut WindowsRdpCredentialBundle,
    password: Option<&String>,
) {
    match password {
        Some(password) => credentials.set_gateway_password(password.clone()),
        None => credentials.clear_gateway_password(),
    }
}

#[cfg(test)]
#[path = "windows_native_policy_tests.rs"]
mod tests;
