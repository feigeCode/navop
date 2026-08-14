use super::*;
use crate::ffi::CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED;

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
fn connection_options_debug_redacts_the_complete_endpoint() {
    let options = WindowsRdpConnectionOptions::new(
        "alice@example.com:endpoint-sentinel@[2001:db8::1]",
        3390,
        1600,
        900,
        WindowsRdpColorDepth::Bpp24,
    )
    .expect("valid connection options")
    .with_audio_playback(false);
    let debug = format!("{options:?}");

    assert!(debug.contains("WindowsRdpConnectionOptions"));
    assert!(debug.contains("port: 3390"));
    assert!(debug.contains("audio_playback: false"));
    assert!(debug.contains("<redacted"));
    assert!(!debug.contains("alice@example.com"));
    assert!(!debug.contains("endpoint-sentinel"));
    assert!(!debug.contains("2001:db8"));
}
