use std::fs;
use std::path::{Path, PathBuf};

const HOST_CRATE: &str = "crates/windows_rdp_host";
const ABI_VERSION: &str = "NAVOP_RDP_ABI_VERSION UINT32_C(1)";
const HOST_TEST: &str = "cargo test --locked -p windows_rdp_host --target $RustTarget";

fn workspace_root() -> PathBuf {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));

    manifest_dir
        .ancestors()
        .find(|candidate| {
            candidate.join("Cargo.toml").is_file()
                && candidate.join("script/install-window.ps1").is_file()
                && candidate.join(".github/workflows/ci.yml").is_file()
        })
        .map(Path::to_path_buf)
        .expect("unable to locate Navop workspace root")
}

fn read(relative_path: &str) -> String {
    let path = workspace_root().join(relative_path);
    let contents = fs::read_to_string(&path).unwrap_or_else(|error| {
        panic!("failed to read {}: {error}", path.display());
    });

    contents.replace("\r\n", "\n")
}

fn assert_contains_all(path: &str, required: &[&str]) {
    let contents = read(path);

    for needle in required {
        assert!(contents.contains(needle), "{path} must contain `{needle}`");
    }
}

fn assert_excludes_all(path: &str, forbidden: &[&str]) {
    let contents = read(path);

    for needle in forbidden {
        assert!(
            !contents.contains(needle),
            "{path} must not contain `{needle}`"
        );
    }
}

fn assert_tokens_in_scope(path: &str, scope_start: &str, scope_end: &str, ordered_tokens: &[&str]) {
    let contents = read(path);
    let (_, after_start) = contents
        .split_once(scope_start)
        .unwrap_or_else(|| panic!("{path} must contain scope start `{scope_start}`"));
    let (scope, _) = after_start
        .split_once(scope_end)
        .unwrap_or_else(|| panic!("{path} must contain scope end `{scope_end}`"));

    let mut remaining = scope;
    for token in ordered_tokens {
        let position = remaining
            .find(token)
            .unwrap_or_else(|| panic!("{path} scope must contain `{token}` in order"));
        remaining = &remaining[position + token.len()..];
    }
}

#[test]
fn workspace_declares_the_decoupled_host_crate() {
    assert_contains_all(
        "Cargo.toml",
        &["\"crates/windows_rdp_host\"", "\"tools/windows-rdp-probe\""],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &[
            "name = \"windows_rdp_host\"",
            "[target.'cfg(windows)'.build-dependencies]",
            "cc = \"1.2.65\"",
            "[lints]",
            "workspace = true",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &["gpui", "remote_desktop", "remote_desktop_view", "windows ="],
    );
}

#[test]
fn c_abi_is_versioned_fixed_width_and_opaque() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "#include <stdint.h>",
            "#include <stddef.h>",
            ABI_VERSION,
            "typedef struct NativeRdpHost NativeRdpHost;",
            "typedef int32_t NavopRdpResult;",
            "NAVOP_RDP_RESULT_OK",
            "NAVOP_RDP_RESULT_INVALID_ARGUMENT",
            "NAVOP_RDP_RESULT_ABI_MISMATCH",
            "NAVOP_RDP_RESULT_ALLOCATION_FAILED",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
            "NAVOP_RDP_RESULT_UNAVAILABLE",
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT",
            "NAVOP_RDP_RESULT_INVALID_STATE",
            "NAVOP_RDP_MAX_HOST_UTF16_CODE_UNITS UINT32_C(255)",
            "NAVOP_RDP_LAST_ERROR_LEGACY_SIZE UINT32_C(24)",
            "NAVOP_RDP_CREATE_STAGE_NONE UINT32_C(0)",
            "NAVOP_RDP_CREATE_STAGE_OLE_INITIALIZE UINT32_C(1)",
            "NAVOP_RDP_CREATE_STAGE_ATL_AX_WIN_INIT UINT32_C(2)",
            "NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW UINT32_C(3)",
            "NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL UINT32_C(4)",
            "NAVOP_RDP_CREATE_STAGE_QUERY_CLIENT UINT32_C(5)",
            "NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE UINT32_C(6)",
            "NAVOP_RDP_CREATE_STAGE_SET_PARENT UINT32_C(7)",
            "NAVOP_RDP_CREATE_STAGE_EVENT_SUBSCRIPTION UINT32_C(8)",
            "NAVOP_RDP_CREATE_STAGE_EXCEPTION UINT32_C(9)",
            "NAVOP_RDP_STAGE_CONNECT_GET_CONNECTED UINT32_C(10)",
            "NAVOP_RDP_STAGE_CONNECT_SET_SERVER UINT32_C(11)",
            "NAVOP_RDP_STAGE_CONNECT_GET_ADVANCED_SETTINGS UINT32_C(12)",
            "NAVOP_RDP_STAGE_CONNECT_SET_RDP_PORT UINT32_C(13)",
            "NAVOP_RDP_STAGE_CONNECT_POLICY UINT32_C(14)",
            "NAVOP_RDP_STAGE_CONNECT_SET_DESKTOP_WIDTH UINT32_C(15)",
            "NAVOP_RDP_STAGE_CONNECT_SET_DESKTOP_HEIGHT UINT32_C(16)",
            "NAVOP_RDP_STAGE_CONNECT_SET_COLOR_DEPTH UINT32_C(17)",
            "NAVOP_RDP_STAGE_CONNECT_INVOKE UINT32_C(18)",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DESKTOP_SCALE_FACTOR UINT32_C(19)",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DEVICE_SCALE_FACTOR UINT32_C(20)",
            "typedef struct NavopRdpProbeOptions",
            "typedef struct NavopRdpProbeResult",
            "typedef struct NavopRdpCreateOptions",
            "typedef struct NavopRdpBorrowedUtf16",
            "typedef struct NavopRdpConnectionOptions",
            "NavopRdpBorrowedUtf16 host;",
            "uint32_t port;",
            "int32_t desktop_width;",
            "int32_t desktop_height;",
            "int32_t color_depth;",
            "uint32_t flags;",
            "NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION UINT32_C(1)",
            "NAVOP_RDP_SESSION_DISPLAY_SETTINGS_ABI_VERSION UINT32_C(1)",
            "typedef struct NavopRdpCreateWithParentOptions",
            "uintptr_t parent_hwnd;",
            "typedef struct NavopRdpLastError",
            "int32_t result;",
            "int32_t hresult;",
            "uint32_t has_hresult;",
            "uint32_t stage;",
            "uint32_t win32_code;",
            "uint32_t has_win32_code;",
            "typedef struct NavopRdpBounds",
            "int32_t x;",
            "int32_t y;",
            "int32_t width;",
            "int32_t height;",
            "parent window's client-area physical pixels",
            "width and height must be non-negative",
            "typedef struct NavopRdpSessionDisplaySettings",
            "uint32_t desktop_width;",
            "uint32_t desktop_height;",
            "uint32_t physical_width;",
            "uint32_t physical_height;",
            "uint32_t orientation;",
            "uint32_t desktop_scale_factor;",
            "uint32_t device_scale_factor;",
            "post-login RDP framebuffer",
            "distinct from NavopRdpBounds",
            "caller-owned",
            "non-owning native window handle",
            "owns only its hidden",
            "never destroys or otherwise takes ownership of the parent",
            "uint32_t struct_size;",
            "uint32_t abi_version;",
            "uint32_t generation_low;",
            "uint32_t generation_high;",
            "struct_size values greater than or equal to the",
            "preserve an",
            "caller-provided size",
            "leave unknown trailing fields",
            "extern \"C\"",
            "navop_rdp_probe(",
            "navop_rdp_create(",
            "navop_rdp_create_with_parent(",
            "navop_rdp_create_with_parent_v2(",
            "navop_rdp_get_last_error(",
            "navop_rdp_set_bounds(",
            "navop_rdp_update_session_display_settings(",
            "navop_rdp_set_visible(",
            "navop_rdp_focus(",
            "navop_rdp_connect(",
            "navop_rdp_get_connection_state(",
            "navop_rdp_request_close(",
            "navop_rdp_disconnect(",
            "host is borrowed UTF-16 data",
            "len is authoritative",
            "does not retain data after the call returns",
            "visible must be exactly 0 or 1",
            "NativeRdpHost** out_host",
            "navop_rdp_destroy(",
            "NativeRdpHost** host",
            "may release the native object only after",
            "leaves",
            "handle non-null",
            "retains ownership for the caller",
            "must not",
            "safe to retry",
        ],
    );
    assert_excludes_all(
        header,
        &[
            "typedef enum",
            "enum NavopRdpResult",
            "HWND",
            "IUnknown",
            "BSTR",
            "wchar_t",
            "std::",
        ],
    );
}

#[test]
fn cpp_and_rust_freeze_the_same_struct_layout() {
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "static_assert(sizeof(NavopRdpResult) == 4)",
            "static_assert(sizeof(NavopRdpProbeOptions) == 8)",
            "static_assert(alignof(NavopRdpProbeOptions) == 4)",
            "static_assert(offsetof(NavopRdpProbeOptions, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpProbeOptions, abi_version) == 4)",
            "static_assert(sizeof(NavopRdpProbeResult) == 16)",
            "static_assert(alignof(NavopRdpProbeResult) == 4)",
            "static_assert(offsetof(NavopRdpProbeResult, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpProbeResult, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpProbeResult, available) == 8)",
            "static_assert(offsetof(NavopRdpProbeResult, reserved) == 12)",
            "static_assert(sizeof(NavopRdpCreateOptions) == 16)",
            "static_assert(alignof(NavopRdpCreateOptions) == 4)",
            "static_assert(offsetof(NavopRdpCreateOptions, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpCreateOptions, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpCreateOptions, generation_low) == 8)",
            "static_assert(offsetof(NavopRdpCreateOptions, generation_high) == 12)",
            "static_assert(offsetof(NavopRdpBorrowedUtf16, data) == 0)",
            "static_assert(sizeof(NavopRdpBorrowedUtf16) == 16)",
            "static_assert(alignof(NavopRdpBorrowedUtf16) == 8)",
            "static_assert(offsetof(NavopRdpBorrowedUtf16, len) == 8)",
            "static_assert(sizeof(NavopRdpBorrowedUtf16) == 8)",
            "static_assert(alignof(NavopRdpBorrowedUtf16) == 4)",
            "static_assert(offsetof(NavopRdpBorrowedUtf16, len) == 4)",
            "static_assert(sizeof(NavopRdpConnectionOptions) == 152)",
            "static_assert(alignof(NavopRdpConnectionOptions) == 8)",
            "static_assert(sizeof(NavopRdpConnectionOptions) == 136)",
            "static_assert(alignof(NavopRdpConnectionOptions) == 4)",
            "static_assert(offsetof(NavopRdpConnectionOptions, host) == 8)",
            "static_assert(offsetof(NavopRdpConnectionOptions, port) == 24)",
            "static_assert(offsetof(NavopRdpConnectionOptions, desktop_width) == 28)",
            "static_assert(offsetof(NavopRdpConnectionOptions, desktop_height) == 32)",
            "static_assert(offsetof(NavopRdpConnectionOptions, color_depth) == 36)",
            "static_assert(offsetof(NavopRdpConnectionOptions, flags) == 40)",
            "static_assert(offsetof(NavopRdpConnectionOptions, legacy_reserved) == 44)",
            "static_assert(offsetof(NavopRdpConnectionOptions, display_mode) == 48)",
            "static_assert(offsetof(NavopRdpConnectionOptions, display_flags) == 52)",
            "offsetof(NavopRdpConnectionOptions, desktop_scale_factor) == 56",
            "offsetof(NavopRdpConnectionOptions, device_scale_factor) == 60",
            "static_assert(offsetof(NavopRdpConnectionOptions, resource_flags) == 64)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_mode) == 68)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_quality) == 72)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_flags) == 76)",
            "static_assert(offsetof(NavopRdpConnectionOptions, keyboard_hook_mode) == 80)",
            "static_assert(offsetof(NavopRdpConnectionOptions, input_flags) == 84)",
            "offsetof(NavopRdpConnectionOptions, performance_preset) == 88",
            "offsetof(NavopRdpConnectionOptions, performance_flags) == 92",
            "offsetof(NavopRdpConnectionOptions, network_connection_type) == 96",
            "static_assert(offsetof(NavopRdpConnectionOptions, security_flags) == 100)",
            "offsetof(NavopRdpConnectionOptions, authentication_level) == 104",
            "static_assert(offsetof(NavopRdpConnectionOptions, gateway_mode) == 108)",
            "static_assert(offsetof(NavopRdpConnectionOptions, gateway_flags) == 112)",
            "offsetof(NavopRdpConnectionOptions, gateway_credential_source) == 116",
            "offsetof(NavopRdpConnectionOptions, gateway_hostname) == 120",
            "offsetof(NavopRdpConnectionOptions, keep_alive_seconds) == 136",
            "static_assert(offsetof(NavopRdpConnectionOptions, timeout_seconds) == 140)",
            "static_assert(offsetof(NavopRdpConnectionOptions, connection_flags) == 144)",
            "offsetof(NavopRdpConnectionOptions, max_reconnect_attempts) == 148",
            "static_assert(offsetof(NavopRdpConnectionOptions, port) == 16)",
            "static_assert(offsetof(NavopRdpConnectionOptions, desktop_width) == 20)",
            "static_assert(offsetof(NavopRdpConnectionOptions, desktop_height) == 24)",
            "static_assert(offsetof(NavopRdpConnectionOptions, color_depth) == 28)",
            "static_assert(offsetof(NavopRdpConnectionOptions, flags) == 32)",
            "static_assert(offsetof(NavopRdpConnectionOptions, legacy_reserved) == 36)",
            "static_assert(offsetof(NavopRdpConnectionOptions, display_mode) == 40)",
            "static_assert(offsetof(NavopRdpConnectionOptions, display_flags) == 44)",
            "offsetof(NavopRdpConnectionOptions, desktop_scale_factor) == 48",
            "offsetof(NavopRdpConnectionOptions, device_scale_factor) == 52",
            "static_assert(offsetof(NavopRdpConnectionOptions, resource_flags) == 56)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_mode) == 60)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_quality) == 64)",
            "static_assert(offsetof(NavopRdpConnectionOptions, audio_flags) == 68)",
            "static_assert(offsetof(NavopRdpConnectionOptions, keyboard_hook_mode) == 72)",
            "static_assert(offsetof(NavopRdpConnectionOptions, input_flags) == 76)",
            "offsetof(NavopRdpConnectionOptions, performance_preset) == 80",
            "offsetof(NavopRdpConnectionOptions, performance_flags) == 84",
            "offsetof(NavopRdpConnectionOptions, network_connection_type) == 88",
            "static_assert(offsetof(NavopRdpConnectionOptions, security_flags) == 92)",
            "offsetof(NavopRdpConnectionOptions, authentication_level) == 96",
            "static_assert(offsetof(NavopRdpConnectionOptions, gateway_mode) == 100)",
            "static_assert(offsetof(NavopRdpConnectionOptions, gateway_flags) == 104)",
            "offsetof(NavopRdpConnectionOptions, gateway_credential_source) == 108",
            "offsetof(NavopRdpConnectionOptions, gateway_hostname) == 112",
            "offsetof(NavopRdpConnectionOptions, keep_alive_seconds) == 120",
            "static_assert(offsetof(NavopRdpConnectionOptions, timeout_seconds) == 124)",
            "static_assert(offsetof(NavopRdpConnectionOptions, connection_flags) == 128)",
            "offsetof(NavopRdpConnectionOptions, max_reconnect_attempts) == 132",
            "static_assert(sizeof(NavopRdpCreateWithParentOptions) >= 20)",
            "static_assert(alignof(NavopRdpCreateWithParentOptions) == alignof(uintptr_t))",
            "static_assert(offsetof(NavopRdpCreateWithParentOptions, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpCreateWithParentOptions, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpCreateWithParentOptions, generation_low) == 8)",
            "static_assert(offsetof(NavopRdpCreateWithParentOptions, generation_high) == 12)",
            "static_assert(offsetof(NavopRdpCreateWithParentOptions, parent_hwnd) == 16)",
            "sizeof(NavopRdpCreateWithParentOptions) == 24",
            "alignof(NavopRdpCreateWithParentOptions) == 8",
            "sizeof(NavopRdpCreateWithParentOptions) == 20",
            "alignof(NavopRdpCreateWithParentOptions) == 4",
            "static_assert(sizeof(NavopRdpLastError) == 36)",
            "static_assert(alignof(NavopRdpLastError) == 4)",
            "static_assert(offsetof(NavopRdpLastError, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpLastError, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpLastError, result) == 8)",
            "static_assert(offsetof(NavopRdpLastError, hresult) == 12)",
            "static_assert(offsetof(NavopRdpLastError, has_hresult) == 16)",
            "static_assert(offsetof(NavopRdpLastError, reserved) == 20)",
            "static_assert(offsetof(NavopRdpLastError, stage) == 24)",
            "static_assert(offsetof(NavopRdpLastError, win32_code) == 28)",
            "static_assert(offsetof(NavopRdpLastError, has_win32_code) == 32)",
            "NAVOP_RDP_LAST_ERROR_LEGACY_SIZE",
            "static_assert(sizeof(NavopRdpBounds) == 16)",
            "static_assert(alignof(NavopRdpBounds) == 4)",
            "static_assert(offsetof(NavopRdpBounds, x) == 0)",
            "static_assert(offsetof(NavopRdpBounds, y) == 4)",
            "static_assert(offsetof(NavopRdpBounds, width) == 8)",
            "static_assert(offsetof(NavopRdpBounds, height) == 12)",
            "static_assert(sizeof(NavopRdpSessionDisplaySettings) == 36)",
            "static_assert(alignof(NavopRdpSessionDisplaySettings) == 4)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, desktop_width) == 8)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, desktop_height) == 12)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, physical_width) == 16)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, physical_height) == 20)",
            "static_assert(offsetof(NavopRdpSessionDisplaySettings, orientation) == 24)",
            "offsetof(NavopRdpSessionDisplaySettings, desktop_scale_factor) == 28",
            "offsetof(NavopRdpSessionDisplaySettings, device_scale_factor) == 32",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "pub(crate) const ABI_VERSION: u32 = 1;",
            "#[repr(C)]",
            "struct NavopRdpProbeOptions",
            "struct NavopRdpProbeResult",
            "struct NavopRdpCreateOptions",
            "CREATE_WITH_PARENT_ABI_VERSION",
            "struct NavopRdpCreateWithParentOptions",
            "parent_hwnd: usize",
            "struct NavopRdpLastError",
            "LAST_ERROR_LEGACY_SIZE",
            "size_of::<NavopRdpLastError>()",
            "align_of::<NavopRdpLastError>()",
            "CREATE_STAGE_EXCEPTION",
            "STAGE_CONNECT_GET_CONNECTED",
            "STAGE_CONNECT_DISPLAY_DESKTOP_SCALE_FACTOR",
            "STAGE_CONNECT_DISPLAY_DEVICE_SCALE_FACTOR",
            "size_of::<NavopRdpProbeOptions>()",
            "align_of::<NavopRdpProbeOptions>()",
            "size_of::<NavopRdpProbeResult>()",
            "align_of::<NavopRdpProbeResult>()",
            "size_of::<NavopRdpCreateOptions>()",
            "align_of::<NavopRdpCreateOptions>()",
            "offset_of!(NavopRdpCreateWithParentOptions, parent_hwnd) == 16",
            "size_of::<NavopRdpCreateWithParentOptions>() == 24",
            "align_of::<NavopRdpCreateWithParentOptions>() == 8",
            "size_of::<NavopRdpCreateWithParentOptions>() == 20",
            "align_of::<NavopRdpCreateWithParentOptions>() == 4",
            "struct NavopRdpBounds",
            "size_of::<NavopRdpBounds>() == 16",
            "align_of::<NavopRdpBounds>() == 4",
            "offset_of!(NavopRdpBounds, x) == 0",
            "offset_of!(NavopRdpBounds, y) == 4",
            "offset_of!(NavopRdpBounds, width) == 8",
            "offset_of!(NavopRdpBounds, height) == 12",
            "SESSION_DISPLAY_SETTINGS_ABI_VERSION",
            "struct NavopRdpSessionDisplaySettings",
            "size_of::<NavopRdpSessionDisplaySettings>() == 36",
            "align_of::<NavopRdpSessionDisplaySettings>() == 4",
            "offset_of!(NavopRdpSessionDisplaySettings, struct_size) == 0",
            "offset_of!(NavopRdpSessionDisplaySettings, abi_version) == 4",
            "offset_of!(NavopRdpSessionDisplaySettings, desktop_width) == 8",
            "offset_of!(NavopRdpSessionDisplaySettings, desktop_height) == 12",
            "offset_of!(NavopRdpSessionDisplaySettings, physical_width) == 16",
            "offset_of!(NavopRdpSessionDisplaySettings, physical_height) == 20",
            "offset_of!(NavopRdpSessionDisplaySettings, orientation) == 24",
            "offset_of!(NavopRdpSessionDisplaySettings, desktop_scale_factor) == 28",
            "offset_of!(NavopRdpSessionDisplaySettings, device_scale_factor) == 32",
            "struct NavopRdpBorrowedUtf16",
            "struct NavopRdpConnectionOptions",
            "size_of::<NavopRdpBorrowedUtf16>() == 16",
            "align_of::<NavopRdpBorrowedUtf16>() == 8",
            "offset_of!(NavopRdpBorrowedUtf16, len) == 8",
            "size_of::<NavopRdpConnectionOptions>() == 152",
            "align_of::<NavopRdpConnectionOptions>() == 8",
            "offset_of!(NavopRdpConnectionOptions, port) == 24",
            "offset_of!(NavopRdpConnectionOptions, flags) == 40",
            "size_of::<NavopRdpBorrowedUtf16>() == 8",
            "align_of::<NavopRdpBorrowedUtf16>() == 4",
            "offset_of!(NavopRdpBorrowedUtf16, len) == 4",
            "size_of::<NavopRdpConnectionOptions>() == 136",
            "align_of::<NavopRdpConnectionOptions>() == 4",
            "offset_of!(NavopRdpConnectionOptions, port) == 16",
            "offset_of!(NavopRdpConnectionOptions, flags) == 32",
        ],
    );
}

#[test]
fn event_callback_abi_is_versioned_owned_and_architecture_independent() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "typedef struct NavopRdpEvent",
            "typedef struct NavopRdpEventCallbackOptions",
            "typedef void (*NavopRdpEventCallback)(",
            "void* context",
            "const NavopRdpEvent* event",
            "const uint8_t* payload",
            "uint32_t kind;",
            "uint32_t reserved;",
            "int32_t code;",
            "uint32_t payload_len;",
            "NAVOP_RDP_EVENT_CONNECTING UINT32_C(1)",
            "NAVOP_RDP_EVENT_CONNECTED UINT32_C(2)",
            "NAVOP_RDP_EVENT_LOGIN_COMPLETE UINT32_C(3)",
            "NAVOP_RDP_EVENT_RECONNECTING UINT32_C(4)",
            "NAVOP_RDP_EVENT_RECONNECTED UINT32_C(5)",
            "NAVOP_RDP_EVENT_NETWORK_STATUS_CHANGED UINT32_C(6)",
            "NAVOP_RDP_EVENT_REMOTE_DESKTOP_SIZE_CHANGED UINT32_C(7)",
            "NAVOP_RDP_EVENT_ENTER_FULLSCREEN UINT32_C(8)",
            "NAVOP_RDP_EVENT_LEAVE_FULLSCREEN UINT32_C(9)",
            "NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISPLAYED UINT32_C(10)",
            "NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISMISSED UINT32_C(11)",
            "NAVOP_RDP_EVENT_WARNING UINT32_C(12)",
            "NAVOP_RDP_EVENT_FATAL_ERROR UINT32_C(13)",
            "NAVOP_RDP_EVENT_LOGON_ERROR UINT32_C(14)",
            "NAVOP_RDP_EVENT_DISCONNECTED UINT32_C(15)",
            "NAVOP_RDP_EVENT_CLOSE_CONFIRMED UINT32_C(16)",
            "NAVOP_RDP_EVENT_FOCUS_RELEASED UINT32_C(17)",
            "NAVOP_RDP_MAX_EVENT_PAYLOAD_BYTES UINT32_C(65536)",
            "architecture-independent byte protocol",
            "Every integer is little-endian",
            "payload_len is 4 or 8",
            "payload_len is 0 or 4",
            "payload_len is 8",
            "optional extended_code:i32",
            "Within the same ABI version, unknown kinds and malformed known",
            "payload schemas are immutable",
            "payload_len must not exceed NAVOP_RDP_MAX_EVENT_PAYLOAD_BYTES",
            "callback payload is borrowed only for the duration",
            "owner thread",
            "does not retain callback or callback_context",
            "no callback is in flight",
            "must not synchronously call",
            "navop_rdp_register_event_callback(",
            "navop_rdp_unregister_event_callback(",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/event.rs"),
        &[
            "struct OwnedNativeEvent",
            "pub struct WindowsRdpRawEvent",
            "pub enum WindowsRdpEvent",
            "struct EventBridge",
            "VecDeque<OwnedNativeEvent>",
            "AtomicU8",
            "Mutex<VecDeque<OwnedNativeEvent>>",
            "unsafe extern \"C\" fn native_event_callback",
            "catch_unwind",
            "payload.to_vec()",
            "event_generation != self.generation",
            "CallbackLifecycle::Closing",
            "CallbackLifecycle::Closed",
            "impl From<WindowsRdpRawEvent> for WindowsRdpEvent",
            "EVENT_REMOTE_DESKTOP_SIZE_CHANGED",
            "decode_reconnecting",
            "decode_optional_u32",
            "decode_optional_i32",
            "decode_u32_pair",
            "decoded.unwrap_or(Self::Unknown { event })",
            "payload_len > MAX_EVENT_PAYLOAD_BYTES",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "const EVENT_CONNECTING: u32 = 1",
            "const EVENT_CONNECTED: u32 = 2",
            "const EVENT_LOGIN_COMPLETE: u32 = 3",
            "const EVENT_RECONNECTING: u32 = 4",
            "const EVENT_RECONNECTED: u32 = 5",
            "const EVENT_NETWORK_STATUS_CHANGED: u32 = 6",
            "const EVENT_REMOTE_DESKTOP_SIZE_CHANGED: u32 = 7",
            "const EVENT_ENTER_FULLSCREEN: u32 = 8",
            "const EVENT_LEAVE_FULLSCREEN: u32 = 9",
            "const EVENT_AUTHENTICATION_WARNING_DISPLAYED: u32 = 10",
            "const EVENT_AUTHENTICATION_WARNING_DISMISSED: u32 = 11",
            "const EVENT_WARNING: u32 = 12",
            "const EVENT_FATAL_ERROR: u32 = 13",
            "const EVENT_LOGON_ERROR: u32 = 14",
            "const EVENT_DISCONNECTED: u32 = 15",
            "const EVENT_CLOSE_CONFIRMED: u32 = 16",
            "const EVENT_FOCUS_RELEASED: u32 = 17",
            "const MAX_EVENT_PAYLOAD_BYTES: u32 = 65_536",
        ],
    );
}

#[test]
fn native_callback_gate_validates_before_retaining_and_closes_before_destroy() {
    let source = &format!("{HOST_CRATE}/native/host.cpp");

    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_register_event_callback(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_unregister_event_callback(",
        &[
            "try {",
            "host == nullptr",
            "options == nullptr",
            "callback == nullptr",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "join_generation(",
            "generation != host->generation",
            "host->callback_state != CallbackState::Open",
            "host->callback != nullptr",
            "host->callback = callback;",
            "host->callback_context = callback_context;",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_unregister_event_callback(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_destroy(",
        &[
            "try {",
            "host == nullptr",
            "close_callback_gate(host);",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_destroy(",
        "\n}",
        &[
            "NativeRdpHost* owned = *host;",
            "close_callback_gate(owned);",
            "*host = nullptr;",
            "delete owned;",
        ],
    );
}

#[test]
fn native_callback_dispatch_enforces_owner_thread_and_quiescent_close() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");
    let internal_header = &format!("{HOST_CRATE}/native/host_internal.h");
    let dispatch_source = &format!("{HOST_CRATE}/native/event_dispatch.cpp");

    assert_contains_all(
        header,
        &[
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT",
            "Wrong-thread calls",
            "callback is in flight",
            "preserve",
            "later owner-thread turn",
        ],
    );
    assert_contains_all(
        internal_header,
        &[
            "uint32_t owner_thread_id;",
            "uint32_t callbacks_in_flight;",
            "ensure_owner_thread(",
            "close_callback_gate(",
            "dispatch_event(",
        ],
    );
    assert_tokens_in_scope(
        dispatch_source,
        "NavopRdpResult close_callback_gate(",
        "\n}\n\nNavopRdpResult dispatch_event(",
        &[
            "host->callback_state == CallbackState::Closed",
            "host->callback_state == CallbackState::Open",
            "host->callback_state = CallbackState::Closing",
            "host->callbacks_in_flight != UINT32_C(0)",
            "host->callback = nullptr",
            "host->callback_context = nullptr",
            "host->callback_state = CallbackState::Closed",
        ],
    );
    assert_contains_all(
        dispatch_source,
        &[
            "#include <windows.h>",
            "class CallbackDispatchScope",
            "host_->callbacks_in_flight += UINT32_C(1);",
            "host_->callbacks_in_flight -= UINT32_C(1);",
            "GetCurrentThreadId()",
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT",
            "host->callback_state != CallbackState::Open",
            "host->callback == nullptr",
            "host->callbacks_in_flight == UINT32_MAX",
            "NavopRdpEventCallback callback = host->callback;",
            "void* callback_context = host->callback_context;",
            "CallbackDispatchScope callback_scope(host);",
            "callback(callback_context, event, payload);",
            "host->callback = nullptr;",
            "host->callback_context = nullptr;",
            "host->callback_state = CallbackState::Closed;",
            "extern \"C\" NavopRdpResult navop_rdp_test_dispatch_event(",
            "try {",
            "catch (...)",
        ],
    );
    assert_excludes_all(
        header,
        &["navop_rdp_test_dispatch_event(", "callbacks_in_flight"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/host.cpp"),
        &[
            "GetCurrentThreadId()",
            "ensure_owner_thread(host)",
            "NavopRdpResult close_result = close_callback_gate(owned);",
            "if (close_result != NAVOP_RDP_RESULT_OK)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/credential.cpp"),
        &["ensure_owner_thread(host)"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &["RESULT_WRONG_THREAD", "RESULT_CALLBACK_IN_FLIGHT"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &[
            "WrongThread",
            "CallbackInFlight",
            "ffi::RESULT_WRONG_THREAD",
            "ffi::RESULT_CALLBACK_IN_FLIGHT",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/native_tests.rs"),
        &[
            "native_dispatch_invokes_the_registered_callback_once",
            "reentrant_unregister_is_rejected_until_callback_returns",
            "reentrant_destroy_preserves_the_handle_until_callback_returns",
            "nested_dispatch_result",
            "wrong_thread_dispatch_unregister_and_destroy_are_rejected",
            "native_dispatch_rejects_invalid_events_without_poisoning_callback",
            "navop_rdp_test_dispatch_event",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/event_dispatch.cpp",
            ".file(\"native/event_dispatch.cpp\")",
        ],
    );
}

#[test]
fn event_callback_layout_is_frozen_without_pointer_sized_struct_fields() {
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "static_assert(sizeof(NavopRdpEvent) == 32)",
            "static_assert(alignof(NavopRdpEvent) == 4)",
            "static_assert(offsetof(NavopRdpEvent, struct_size) == 0)",
            "static_assert(offsetof(NavopRdpEvent, abi_version) == 4)",
            "static_assert(offsetof(NavopRdpEvent, kind) == 8)",
            "static_assert(offsetof(NavopRdpEvent, reserved) == 12)",
            "static_assert(offsetof(NavopRdpEvent, generation_low) == 16)",
            "static_assert(offsetof(NavopRdpEvent, generation_high) == 20)",
            "static_assert(offsetof(NavopRdpEvent, code) == 24)",
            "static_assert(offsetof(NavopRdpEvent, payload_len) == 28)",
            "static_assert(sizeof(NavopRdpEventCallbackOptions) == 16)",
            "static_assert(alignof(NavopRdpEventCallbackOptions) == 4)",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "struct NavopRdpEvent",
            "struct NavopRdpEventCallbackOptions",
            "size_of::<NavopRdpEvent>()",
            "align_of::<NavopRdpEvent>()",
            "size_of::<NavopRdpEventCallbackOptions>()",
            "align_of::<NavopRdpEventCallbackOptions>()",
            "const _: () = {",
            "unsafe extern \"C\" fn(",
            "register_event_callback",
            "unregister_event_callback",
        ],
    );
}

#[test]
fn credential_transport_is_versioned_borrowed_and_architecture_specific() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        header,
        &[
            "typedef struct NavopRdpBorrowedSecret",
            "const uint16_t* data;",
            "uint32_t len;",
            "typedef struct NavopRdpCredentialBundle",
            "uint32_t struct_size;",
            "uint32_t abi_version;",
            "NavopRdpBorrowedSecret server_password;",
            "NavopRdpBorrowedSecret gateway_password;",
            "uint32_t flags;",
            "NavopRdpBorrowedUtf16 username;",
            "NavopRdpBorrowedUtf16 domain;",
            "NavopRdpBorrowedUtf16 gateway_username;",
            "NavopRdpBorrowedUtf16 gateway_domain;",
            "NAVOP_RDP_CREDENTIAL_LEGACY_SIZE",
            "append-only fields",
            "borrowed only for the synchronous call",
            "must not retain",
            "navop_rdp_apply_credentials(",
            "const NavopRdpCredentialBundle* credentials",
            "INTPTR_MAX == INT64_MAX",
            "sizeof(NavopRdpBorrowedSecret) == 16",
            "alignof(NavopRdpBorrowedSecret) == 8",
            "offsetof(NavopRdpBorrowedSecret, data) == 0",
            "offsetof(NavopRdpBorrowedSecret, len) == 8",
            "sizeof(NavopRdpCredentialBundle) == 112",
            "alignof(NavopRdpCredentialBundle) == 8",
            "offsetof(NavopRdpCredentialBundle, struct_size) == 0",
            "offsetof(NavopRdpCredentialBundle, abi_version) == 4",
            "offsetof(NavopRdpCredentialBundle, server_password) == 8",
            "offsetof(NavopRdpCredentialBundle, gateway_password) == 24",
            "offsetof(NavopRdpCredentialBundle, flags) == 40",
            "offsetof(NavopRdpCredentialBundle, username) == 48",
            "offsetof(NavopRdpCredentialBundle, domain) == 64",
            "offsetof(NavopRdpCredentialBundle, gateway_username) == 80",
            "offsetof(NavopRdpCredentialBundle, gateway_domain) == 96",
            "INTPTR_MAX == INT32_MAX",
            "sizeof(NavopRdpBorrowedSecret) == 8",
            "alignof(NavopRdpBorrowedSecret) == 4",
            "offsetof(NavopRdpBorrowedSecret, len) == 4",
            "sizeof(NavopRdpCredentialBundle) == 60",
            "alignof(NavopRdpCredentialBundle) == 4",
            "offsetof(NavopRdpCredentialBundle, server_password) == 8",
            "offsetof(NavopRdpCredentialBundle, gateway_password) == 16",
            "offsetof(NavopRdpCredentialBundle, flags) == 24",
            "offsetof(NavopRdpCredentialBundle, username) == 28",
            "offsetof(NavopRdpCredentialBundle, domain) == 36",
            "offsetof(NavopRdpCredentialBundle, gateway_username) == 44",
            "offsetof(NavopRdpCredentialBundle, gateway_domain) == 52",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "struct NavopRdpBorrowedSecret",
            "data: *const u16",
            "len: u32",
            "struct NavopRdpCredentialBundle",
            "server_password: NavopRdpBorrowedSecret",
            "gateway_password: NavopRdpBorrowedSecret",
            "username: NavopRdpBorrowedUtf16",
            "domain: NavopRdpBorrowedUtf16",
            "gateway_username: NavopRdpBorrowedUtf16",
            "gateway_domain: NavopRdpBorrowedUtf16",
            "type ApplyCredentialsFn",
            "apply_credentials: ApplyCredentialsFn",
            "navop_rdp_apply_credentials(",
            "target_pointer_width = \"64\"",
            "size_of::<NavopRdpBorrowedSecret>() == 16",
            "size_of::<NavopRdpCredentialBundle>() == 112",
            "target_pointer_width = \"32\"",
            "size_of::<NavopRdpBorrowedSecret>() == 8",
            "size_of::<NavopRdpCredentialBundle>() == 60",
        ],
    );
}

#[test]
fn rust_credentials_are_zeroizing_redacted_and_not_persisted_in_the_host() {
    assert_contains_all(
        &format!("{HOST_CRATE}/Cargo.toml"),
        &["[dependencies]", "zeroize.workspace = true"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/credential.rs"),
        &[
            "pub struct WindowsRdpCredentialBundle",
            "Zeroizing<Vec<u16>>",
            "Zeroizing::new(password)",
            "encode_utf16()",
            "impl fmt::Debug for WindowsRdpCredentialBundle",
            "\"<redacted",
            "NavopRdpBorrowedUtf16",
            "NavopRdpBorrowedSecret",
            "NavopRdpCredentialBundle",
            "u32::try_from",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/credential.rs"),
        &[
            "derive(Clone",
            "derive(Serialize",
            "derive(Deserialize",
            "impl Clone for WindowsRdpCredentialBundle",
            "impl serde::Serialize",
            "impl serde::Deserialize",
            "use serde",
            "serde::",
            "log::",
            "tracing::",
            "println!",
            "eprintln!",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "pub fn apply_credentials(",
            "credentials.as_native()",
            "(self.bindings.apply_credentials)(self.raw, &native_credentials)",
            "HostLifecycle::Open",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "server_password:",
            "gateway_password:",
            "credentials: WindowsRdpCredentialBundle",
        ],
    );
}

#[test]
fn native_credentials_validate_copy_and_wipe_on_every_exit_path() {
    let source = &format!("{HOST_CRATE}/native/credential.cpp");

    assert_contains_all(
        source,
        &[
            "#include <windows.h>",
            "class SensitiveUtf16Buffer",
            "~SensitiveUtf16Buffer() noexcept",
            "SecureZeroMemory(",
            "delete[]",
            "std::memcpy(",
            "std::nothrow",
            "validate_borrowed_secret(",
            "validate_borrowed_utf16(",
            "secret.len == UINT32_C(0)",
            "secret.data == nullptr",
            "(std::numeric_limits<size_t>::max)()",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_apply_credentials(",
        "\n}",
        &[
            "try {",
            "host == nullptr",
            "credentials == nullptr",
            "validate_struct_size(",
            "credentials->struct_size",
            "NAVOP_RDP_CREDENTIAL_LEGACY_SIZE",
            "validate_abi_version(",
            "credentials->abi_version",
            "credentials->flags != UINT32_C(0)",
            "host->callback_state != CallbackState::Open",
            "validate_borrowed_secret(credentials->server_password)",
            "validate_borrowed_secret(credentials->gateway_password)",
            "credential_field_available<NavopRdpBorrowedUtf16>",
            "validate_borrowed_utf16(username)",
            "validate_borrowed_utf16(domain)",
            "validate_borrowed_utf16(gateway_username)",
            "validate_borrowed_utf16(gateway_domain)",
            "SensitiveUtf16Buffer server_password;",
            "SensitiveUtf16Buffer gateway_password;",
            "server_password.copy_from(credentials->server_password)",
            "gateway_password.copy_from(credentials->gateway_password)",
            "apply_active_x_credentials(",
            "server_password.borrowed()",
            "gateway_username",
            "gateway_domain",
            "gateway_password.borrowed()",
            "return NAVOP_RDP_RESULT_OK;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_excludes_all(
        source,
        &[
            "std::wstring",
            "std::u16string",
            "wcslen",
            "lstrlenW",
            "CComBSTR",
            "BSTR",
            "IMsRdp",
            "AtlAx",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/active_x_host.cpp"),
        &[
            "NavopRdpResult apply_active_x_credentials(",
            "put_UserName(",
            "put_Domain(",
            "resources->state.non_scriptable->put_ClearTextPassword(",
            "put_ClearTextPassword(",
            "CComQIPtr<IMsRdpClient7>",
            "get_TransportSettings2(",
            "IMsRdpClientTransportSettings2",
            "put_GatewayUsername(",
            "put_GatewayDomain(",
            "put_GatewayPassword(",
            "class SensitiveBstr",
            "SecureZeroMemory(",
            "SysFreeString(",
            "record_last_hresult(",
            "get_Connected(",
            "NAVOP_RDP_RESULT_INVALID_STATE",
        ],
    );
    assert_tokens_in_scope(
        &format!("{HOST_CRATE}/native/active_x_host.cpp"),
        "NavopRdpResult apply_active_x_credentials(",
        "\n}\n\nNavopRdpResult get_active_x_connection_state(",
        &[
            "NavopRdpBorrowedUtf16 gateway_username",
            "NavopRdpBorrowedUtf16 gateway_domain",
            "NavopRdpBorrowedSecret gateway_password",
            "trace_native_stage(\"credentials.set_password.before\")",
            "resources->state.non_scriptable->put_ClearTextPassword(",
            "trace_native_hresult(\n            \"credentials.set_password.after\"",
            "client7->get_TransportSettings2(&transport)",
            "trace_native_stage(\"credentials.gateway.username.before\")",
            "transport->put_GatewayUsername(gateway_username_bstr)",
            "trace_native_stage(\"credentials.gateway.domain.before\")",
            "transport->put_GatewayDomain(gateway_domain_bstr)",
            "trace_native_stage(\"credentials.gateway.password.before\")",
            "transport->put_GatewayPassword(",
            "gateway_password_bstr.get()",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/native/active_x_host.cpp"),
        &[
            "resources->state.control->QueryInterface(\n            IID_PPV_ARGS(&advanced_settings))",
            "advanced_settings->put_ClearTextPassword(",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/credential.cpp",
            ".file(\"native/credential.cpp\")",
        ],
    );
}

#[test]
fn credentials_do_not_expand_options_events_errors_or_dump_collection() {
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/options.rs"),
        &[
            "password",
            "secret",
            "server_password",
            "gateway_password",
            "NavopRdpCredentialBundle",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/event.rs"),
        &["password", "credential", "secret"],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &["password", "credential", "secret"],
    );
    for path in [
        "src/lib.rs",
        "src/ffi.rs",
        "src/credential.rs",
        "src/handle.rs",
        "native/windows_rdp_host.h",
        "native/host.cpp",
        "native/credential.cpp",
        "build.rs",
    ] {
        assert_excludes_all(
            &format!("{HOST_CRATE}/{path}"),
            &[
                "MiniDumpWriteDump",
                "MiniDumpWithFullMemory",
                "WER_DUMP_TYPE",
                "DumpType = 2",
            ],
        );
    }
}

#[test]
fn native_entrypoints_validate_headers_and_contain_failures() {
    let source = &format!("{HOST_CRATE}/native/host.cpp");

    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_probe(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_create(",
        &[
            "try {",
            "options == nullptr",
            "out_result == nullptr",
            "const uint32_t caller_result_size = out_result->struct_size;",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "validate_struct_size(",
            "caller_result_size",
            "validate_abi_version(",
            "out_result->abi_version",
            "out_result->struct_size = caller_result_size;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_create(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_create_with_parent(",
        &[
            "try {",
            "out_host == nullptr",
            "*out_host = nullptr;",
            "options == nullptr",
            "validate_struct_size(",
            "options->struct_size",
            "validate_abi_version(",
            "options->abi_version",
            "new (std::nothrow) NativeRdpHost",
            "NAVOP_RDP_RESULT_ALLOCATION_FAILED",
            "*out_host = host;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_create_with_parent(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_set_bounds(",
        &[
            "try {",
            "out_host == nullptr",
            "*out_host = nullptr;",
            "options == nullptr",
            "validate_struct_size(",
            "options->struct_size",
            "NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION",
            "options->parent_hwnd == 0",
            "reinterpret_cast<HWND>(options->parent_hwnd)",
            "IsWindow(parent)",
            "GetWindowThreadProcessId(parent, nullptr)",
            "GetCurrentThreadId()",
            "NAVOP_RDP_RESULT_WRONG_THREAD",
            "new (std::nothrow) NativeRdpHost",
            "create_active_x_resources(",
            "delete host;",
            "*out_host = host;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_set_bounds(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_set_visible(",
        &[
            "try {",
            "if (host == nullptr)",
            "const NavopRdpResult owner_result = ensure_owner_thread(host)",
            "if (owner_result != NAVOP_RDP_RESULT_OK)",
            "clear_last_error(host)",
            "bounds == nullptr || bounds->width < 0 || bounds->height < 0",
            "host->callback_state != CallbackState::Open",
            "record_last_error(",
            "set_active_x_bounds(host->active_x_resources, *bounds)",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_set_visible(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_focus(",
        &[
            "try {",
            "if (host == nullptr)",
            "const NavopRdpResult owner_result = ensure_owner_thread(host)",
            "if (owner_result != NAVOP_RDP_RESULT_OK)",
            "clear_last_error(host)",
            "visible > UINT32_C(1)",
            "host->callback_state != CallbackState::Open",
            "record_last_error(",
            "set_active_x_visible(",
            "visible == UINT32_C(1)",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_focus(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_register_event_callback(",
        &[
            "try {",
            "host == nullptr",
            "ensure_owner_thread(host)",
            "host->callback_state != CallbackState::Open",
            "focus_active_x(host->active_x_resources)",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_tokens_in_scope(
        source,
        "extern \"C\" NavopRdpResult navop_rdp_destroy(",
        "\n}",
        &[
            "try {",
            "host == nullptr",
            "*host == nullptr",
            "NativeRdpHost* owned = *host;",
            "*host = nullptr;",
            "delete owned;",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
    assert_excludes_all(
        source,
        &[
            "validate_header(",
            "OleInitialize",
            "AtlAx",
            "mstscax",
            "CComPtr",
        ],
    );
}

#[test]
fn native_type_library_bindings_are_generated_before_parallel_host_compilation() {
    let build_script = &format!("{HOST_CRATE}/build.rs");
    let importer = &format!("{HOST_CRATE}/native/mstscax_import.cpp");

    assert_contains_all(
        importer,
        &[
            "#pragma warning(disable : 4192)",
            "#pragma warning(disable : 4471)",
            "#import \"libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489\"",
            "raw_interfaces_only, named_guids, no_namespace, exclude(\"UINT_PTR\")",
        ],
    );
    assert_contains_all(
        build_script,
        &[
            "cargo:rerun-if-changed=native/mstscax_import.cpp",
            ".file(\"native/mstscax_import.cpp\")",
            ".try_compile_intermediates()",
            "out_dir.join(\"mstscax.tlh\")",
        ],
    );
    assert_tokens_in_scope(
        build_script,
        "fn build_native_host()",
        "\n}",
        &[
            "generate_type_library_bindings(&out_dir);",
            ".file(\"native/diagnostic.cpp\")",
            ".file(\"native/event_sink.cpp\")",
            ".file(\"native/active_x_host.cpp\")",
            ".compile(\"windows_rdp_host\");",
        ],
    );
    for consumer in [
        "native/event_sink.cpp",
        "native/active_x_host.cpp",
        "native/audio_redirection.cpp",
        "native/connection_policy_display.cpp",
        "native/connection_policy_gateway.cpp",
        "native/connection_policy_runtime.cpp",
        "native/connection_policy_session.cpp",
    ] {
        let path = &format!("{HOST_CRATE}/{consumer}");
        assert_contains_all(
            path,
            &[
                "#pragma warning(push)",
                "#pragma warning(disable : 4471)",
                "#include \"mstscax.tlh\"",
                "#pragma warning(pop)",
            ],
        );
        assert_excludes_all(
            path,
            &["#import \"libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489\""],
        );
    }
}

#[test]
fn native_audio_policy_uses_complete_options_and_secured_settings3() {
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED UINT32_C(1)",
            "NAVOP_RDP_CONNECTION_FLAGS_KNOWN",
            "unknown connection flag",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED: u32 = 1",
            "CONNECTION_FLAGS_KNOWN",
            "flags & !CONNECTION_FLAGS_KNOWN != 0",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/configuration.cpp"),
        &["(options.flags & ~NAVOP_RDP_CONNECTION_FLAGS_KNOWN) != 0"],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/audio_redirection.cpp",
            ".file(\"native/audio_redirection.cpp\")",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/audio_redirection.cpp"),
        &[
            "constexpr LONG kAudioRedirectionRedirectToLocal = 0",
            "constexpr LONG kAudioRedirectionPlayOnRemote = 1",
            "constexpr LONG kAudioRedirectionDisabled = 2",
            "IMsRdpClientSecuredSettings2",
            "get_SecuredSettings3",
            "put_AudioRedirectionMode",
            "options.audio_mode",
            "options.audio_quality",
            "options.audio_flags",
            "AudioQualityMode",
            "AudioCaptureRedirectionMode",
            "connect.get_secured_settings3.before",
            "connect.get_secured_settings3.after",
            "connect.set_audio_redirection_mode.before",
            "connect.set_audio_redirection_mode.after",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/native/audio_redirection.cpp"),
        &["NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED"],
    );
}

#[test]
fn active_x_connection_policy_consumes_the_complete_normalized_options() {
    let build_script = &format!("{HOST_CRATE}/build.rs");
    let internal_header = &format!("{HOST_CRATE}/native/host_internal.h");
    let policy_source = &format!("{HOST_CRATE}/native/connection_policy.cpp");
    let dispatch_source = &format!("{HOST_CRATE}/native/dispatch_property.cpp");
    let active_x_source = &format!("{HOST_CRATE}/native/active_x_host.cpp");

    assert_contains_all(
        build_script,
        &[
            "cargo:rerun-if-changed=native/connection_policy.cpp",
            "cargo:rerun-if-changed=native/dispatch_property.cpp",
            ".file(\"native/connection_policy.cpp\")",
            ".file(\"native/dispatch_property.cpp\")",
        ],
    );
    assert_contains_all(
        internal_header,
        &[
            "struct NativeRdpConnectionPolicyContext",
            "IUnknown* non_scriptable3;",
            "IUnknown* non_scriptable5;",
            "configure_active_x_connection_policy(",
            "set_dispatch_bool(",
            "set_dispatch_long(",
            "set_dispatch_utf16(",
        ],
    );
    assert_contains_all(
        policy_source,
        &[
            "options.display_mode",
            "options.display_flags",
            "options.desktop_scale_factor",
            "options.device_scale_factor",
            "options.resource_flags",
            "options.audio_mode",
            "options.audio_quality",
            "options.audio_flags",
            "options.keyboard_hook_mode",
            "options.input_flags",
            "options.performance_preset",
            "options.performance_flags",
            "options.network_connection_type",
            "options.security_flags",
            "options.authentication_level",
            "options.gateway_mode",
            "options.gateway_flags",
            "options.gateway_credential_source",
            "options.gateway_hostname",
            "options.keep_alive_seconds",
            "options.timeout_seconds",
            "options.connection_flags",
            "options.max_reconnect_attempts",
        ],
    );
    assert_contains_all(
        dispatch_source,
        &[
            "QueryInterface(IID_PPV_ARGS(&dispatch))",
            "GetIDsOfNames",
            "DISPID_PROPERTYPUT",
            "DISPATCH_PROPERTYPUT",
            "Invoke(",
        ],
    );
    assert_tokens_in_scope(
        active_x_source,
        "NavopRdpResult connect_active_x(",
        "\n}\n\nNavopRdpResult apply_active_x_credentials(",
        &[
            "connect.get_connected.before",
            "connect.get_connected.after",
            "connect.server.before",
            "connect.server.after",
            "connect.get_advanced_settings.before",
            "connect.get_advanced_settings.after",
            "connect.rdp_port.before",
            "connect.rdp_port.after",
            "configure_active_x_connection_policy(",
            "connect.desktop_width.before",
            "connect.desktop_width.after",
            "connect.desktop_height.before",
            "connect.desktop_height.after",
            "connect.color_depth.before",
            "connect.color_depth.after",
            "connect.invoke.before",
            "Connect()",
            "connect.invoke.after",
        ],
    );
    assert_excludes_all(
        active_x_source,
        &[
            "configure_audio_redirection(\n        owner,\n        resources->state.client,\n        options.flags)",
            "put_AuthenticationLevel(0)",
            "put_EncryptionEnabled(1)",
        ],
    );
}

#[test]
fn active_x_host_subclasses_an_isolated_native_child_and_releases_owned_resources() {
    let source = &format!("{HOST_CRATE}/native/active_x_host.cpp");

    assert_contains_all(
        source,
        &[
            "#include <windows.h>",
            "#include <atlbase.h>",
            "#include <atlhost.h>",
            "#include \"mstscax.tlh\"",
            "kNativeHostWindowClassName",
            "class WindowsRdpAtlModule final",
            "public CAtlModuleT<WindowsRdpAtlModule>",
            "WindowsRdpAtlModule windows_rdp_atl_module;",
            "native_host_window_procedure(",
            "RegisterClassExW(",
            "struct ActiveXCleanup",
            "HWND parent_window",
            "HWND host_window",
            "CComPtr<IUnknown> container;",
            "CComPtr<IUnknown> control;",
            "CComPtr<IOleInPlaceObject> in_place_object;",
            "CComPtr<IMsRdpClient9> client;",
            "CComPtr<IMsRdpClientNonScriptable3> non_scriptable3;",
            "CComPtr<IMsRdpClientNonScriptable5> non_scriptable5;",
            "CComPtr<IPersistStreamInit> persist_stream_init;",
            "NativeRdpEventSubscription* event_subscription = nullptr;",
            "CComPtr<IMsRdpClientNonScriptable2> non_scriptable;",
            "OleInitialize(nullptr)",
            "AtlAxWinInit()",
            "CreateWindowExW(",
            "WS_EX_NOPARENTNOTIFY",
            "WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS",
            "CoCreateInstance(",
            "CAxHostWindow::_CreatorClass::CreateInstance(",
            "CComPtr<IAxWinHostWindow> host;",
            "host->AttachControl(",
            "kMsRdpClient9NotSafeForScriptingClsid",
            "persist_stream_init->InitNew()",
            "QueryInterface(",
            "IID_PPV_ARGS(&resources->state.client)",
            "IID_PPV_ARGS(&resources->state.non_scriptable)",
            "resources->state.non_scriptable->put_UIParentWindowHandle(\n            reinterpret_cast<wireHWND>(parent))",
            "*out_resources = resources.release();",
            "validate_resources(",
            "IsWindow(",
            "SetWindowPos(",
            "SWP_NOACTIVATE",
            "SWP_NOZORDER | SWP_NOACTIVATE",
            "synchronize_control_bounds(",
            "GetClientRect(",
            "SetObjectRects(",
            "GetWindow(&control_window)",
            "RedrawWindow(",
            "RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN",
            "window_or_descendant_has_focus",
            "SetFocus(",
            "ShowWindow(",
            "SW_SHOWNA",
            "SW_HIDE",
            "GetWindowLongPtrW(",
            "GWL_STYLE",
            "WS_VISIBLE",
            "set_active_x_bounds(",
            "set_active_x_visible(",
            "focus_active_x(",
        ],
    );
    assert_tokens_in_scope(
        source,
        "HRESULT attach_control_with_traces(",
        "\n}\n\nHRESULT synchronize_control_bounds",
        &[
            "trace_native_pointer(\n        \"create.atl_module\"",
            "CAxHostWindow::_CreatorClass::CreateInstance(",
            "reinterpret_cast<void**>(&resources.container)",
            "CComPtr<IAxWinHostWindow> host;",
            "resources.container->QueryInterface(&host)",
            "host->AttachControl(",
            "resources.control",
            "resources.host_window",
        ],
    );
    assert_tokens_in_scope(
        source,
        "HRESULT position_direct_control_window(",
        "\n}\n\nvoid trace_presentation_window_state",
        &[
            "GetParent(control_window)",
            "trace_native_pointer(\n        \"presentation.control_parent\"",
            "if (control_parent != resources.host_window)",
            "IsChild(resources.host_window, control_window)",
            "presentation.position_control_window.control_not_descendant",
            "kPresentationIncompleteHresult",
            "presentation.position_control_window.non_direct_descendant",
            "SetLastError(ERROR_SUCCESS)",
            "const int mapped_points = MapWindowPoints(",
            "const DWORD map_error = GetLastError();",
            "presentation.position_control_window.map_points",
            "mapped_points == 0 && map_error != ERROR_SUCCESS",
            "presentation.position_control_window.map_failed",
            "UINT descendant_flags = SWP_NOZORDER | SWP_NOACTIVATE",
            "descendant_flags |= SWP_SHOWWINDOW",
            "mapped_rect.left",
            "mapped_rect.top",
            "mapped_rect.right - mapped_rect.left",
            "mapped_rect.bottom - mapped_rect.top",
            "UINT position_flags = SWP_NOZORDER | SWP_NOACTIVATE",
            "IsWindowVisible(resources.host_window)",
            "position_flags |= SWP_SHOWWINDOW",
            "presentation.position_control_window.before",
            "SetWindowPos(",
            "control_window",
            "nullptr",
            "position_flags",
            "presentation.position_control_window.after",
        ],
    );
    assert_tokens_in_scope(
        source,
        "PresentationWindowError presentation_window_error(",
        "\n}\n\nHRESULT position_direct_control_window",
        &[
            "error == ERROR_SUCCESS",
            "ERROR_INVALID_WINDOW_HANDLE",
            "code == ERROR_INVALID_WINDOW_HANDLE",
            "kPresentationIncompleteHresult",
            "HRESULT_FROM_WIN32(code)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "HRESULT position_direct_control_window(",
        "\n}\n\nvoid trace_presentation_window_state",
        &[
            "const PresentationWindowError error =\n                presentation_window_error(map_error);",
            "presentation.position_control_window.map_failed",
            "static_cast<uint32_t>(error.code)",
            "return error.result;",
            "const PresentationWindowError error =\n                presentation_window_error(GetLastError());",
            "presentation.position_control_window.descendant_failed",
            "const PresentationWindowError error =\n        presentation_window_error(GetLastError());",
            "presentation.position_control_window.failed",
        ],
    );
    assert_excludes_all(
        source,
        &["presentation.position_control_window.skipped_non_direct_child"],
    );
    assert_tokens_in_scope(
        source,
        "void trace_presentation_window_state(",
        "\n}\n\nHRESULT synchronize_control_bounds",
        &[
            "GetParent(resources.host_window)",
            "GetAncestor(control_window, GA_ROOT)",
            "GetAncestor(control_window, GA_ROOTOWNER)",
            "trace_native_win32(\n        \"presentation.control_is_host_descendant\"",
            "IsChild(resources.host_window, control_window)",
            "IsWindowVisible(resources.host_window)",
            "IsWindowVisible(control_window)",
            "GetWindowRect(resources.host_window, &host_rect)",
            "trace_native_rect(\n            \"presentation.host_window_rect\"",
            "GetWindowRect(control_window, &control_rect)",
        ],
    );
    assert_tokens_in_scope(
        source,
        "HRESULT synchronize_control_bounds(",
        "\n}\n\nbool window_or_descendant_has_focus",
        &[
            "GetClientRect(resources.host_window, &client_rect)",
            "trace_native_rect(\n        \"presentation.host_client_rect\"",
            "resources.in_place_object->SetObjectRects(",
            "&client_rect,\n            &client_rect",
            "resources.in_place_object->GetWindow(&control_window)",
            "trace_native_pointer(\n        \"presentation.control_window\"",
            "position_direct_control_window(",
            "trace_presentation_window_state(resources, control_window)",
            "RedrawWindow(",
            "resources.host_window",
            "RDW_INVALIDATE | RDW_UPDATENOW | RDW_ALLCHILDREN",
        ],
    );
    assert_tokens_in_scope(
        source,
        "NavopRdpResult create_active_x_resources(",
        "\n}\n\nvoid destroy_active_x_resources",
        &[
            "const HWND parent = reinterpret_cast<HWND>(parent_hwnd);",
            "resources->state.parent_window = parent;",
            "OleInitialize(nullptr)",
            "AtlAxWinInit()",
            "ensure_native_host_window_class(instance)",
            "resources->state.host_window = CreateWindowExW(",
            "WS_EX_NOPARENTNOTIFY",
            "L\"\"",
            "const HRESULT control_result = CoCreateInstance(",
            "kMsRdpClient9NotSafeForScriptingClsid",
            "IID_PPV_ARGS(&resources->state.client)",
            "IID_PPV_ARGS(&resources->state.non_scriptable)",
            "resources->state.non_scriptable == nullptr",
            "IID_PPV_ARGS(&resources->state.non_scriptable3)",
            "resources->state.non_scriptable3 == nullptr",
            "IID_PPV_ARGS(&resources->state.non_scriptable5)",
            "resources->state.non_scriptable5 == nullptr",
            "CComPtr<IPersistStreamInit> persist_stream_init;",
            "IID_PPV_ARGS(&persist_stream_init)",
            "const HRESULT initialize_result = persist_stream_init->InitNew();",
            "if (FAILED(initialize_result))",
            "const NavopRdpResult subscription_result = create_event_subscription(",
            "owner,",
            "&resources->state.event_subscription",
            "const HRESULT attach_result =\n        attach_control_with_traces(resources->state);",
            "IID_PPV_ARGS(&resources->state.in_place_object)",
            "synchronize_control_bounds(resources->state)",
            "const HRESULT ui_parent_result =\n        resources->state.non_scriptable->put_UIParentWindowHandle(\n            reinterpret_cast<wireHWND>(parent));",
            "if (FAILED(ui_parent_result))",
            "*out_resources = resources.release();",
        ],
    );
    assert_tokens_in_scope(
        source,
        "NavopRdpResult create_active_x_resources(",
        "\n}\n\nvoid destroy_active_x_resources",
        &[
            "trace_native_stage(\"create.ole_initialize.before\")",
            "trace_native_hresult(\n        \"create.ole_initialize.after\"",
            "trace_native_stage(\"create.atl_ax_win_init.before\")",
            "trace_native_stage(\"create.atl_ax_win_init.after\")",
            "trace_native_stage(\"create.host_class.before\")",
            "trace_native_win32(\n        \"create.host_class.after\"",
            "trace_native_stage(\"create.host_window.before\")",
            "trace_native_pointer(\n        \"create.host_window.after\"",
            "trace_native_stage(\"create.control_instance.before\")",
            "trace_native_hresult(\n        \"create.control_instance.after\"",
            "trace_native_stage(\"create.query_client.before\")",
            "trace_native_stage(\"create.query_non_scriptable.before\")",
            "trace_native_stage(\"create.query_non_scriptable3.before\")",
            "\"create.query_non_scriptable3.after\"",
            "trace_native_stage(\"create.query_non_scriptable5.before\")",
            "\"create.query_non_scriptable5.after\"",
            "trace_native_stage(\"create.query_persist_stream_init.before\")",
            "trace_native_hresult(\n        \"create.query_persist_stream_init.after\"",
            "trace_native_stage(\"create.initialize_control.before\")",
            "trace_native_hresult(\n        \"create.initialize_control.after\"",
            "trace_native_stage(\"create.event_subscription.before\")",
            "attach_control_with_traces(resources->state)",
            "trace_native_stage(\"create.query_in_place_object.before\")",
            "trace_native_hresult(\n        \"create.query_in_place_object.after\"",
            "trace_native_stage(\"create.synchronize_bounds.before\")",
            "trace_native_hresult(\n        \"create.synchronize_bounds.after\"",
            "initial_layout_result != kPresentationIncompleteHresult",
            "initial_layout_result == kPresentationIncompleteHresult",
            "trace_native_stage(\"create.synchronize_bounds.deferred\")",
            "trace_native_stage(\"create.set_ui_parent.before\")",
            "trace_native_stage(\"create.complete\")",
        ],
    );
    assert_tokens_in_scope(
        source,
        "~ActiveXCleanup() noexcept",
        "\n    }\n};",
        &[
            "destroy_event_subscription(event_subscription);",
            "event_subscription = nullptr;",
            "non_scriptable5.Release();",
            "non_scriptable3.Release();",
            "non_scriptable.Release();",
            "client.Release();",
            "in_place_object.Release();",
            "control.Release();",
            "container.Release();",
            "DestroyWindow(host_window);",
            "AtlAxWinTerm();",
            "OleUninitialize();",
        ],
    );
    assert_tokens_in_scope(
        source,
        "NavopRdpResult set_active_x_bounds(",
        "\n}\n\nNavopRdpResult set_active_x_visible",
        &[
            "SetWindowPos(",
            "resources->state.host_window",
            "nullptr",
            "SWP_NOZORDER | SWP_NOACTIVATE",
            "synchronize_control_bounds(resources->state)",
            "if (FAILED(layout_result))",
            "if (layout_result == kPresentationIncompleteHresult)",
            "return NAVOP_RDP_RESULT_PRESENTATION_INCOMPLETE;",
            "return NAVOP_RDP_RESULT_INTERNAL_ERROR;",
            "return NAVOP_RDP_RESULT_OK;",
        ],
    );
    assert_tokens_in_scope(
        source,
        "NavopRdpResult set_active_x_visible(",
        "\n}\n\nNavopRdpResult focus_active_x",
        &[
            "synchronize_control_bounds(resources->state)",
            "if (FAILED(layout_result))",
            "if (layout_result == kPresentationIncompleteHresult)",
            "return NAVOP_RDP_RESULT_PRESENTATION_INCOMPLETE;",
            "return NAVOP_RDP_RESULT_INTERNAL_ERROR;",
            "return NAVOP_RDP_RESULT_OK;",
        ],
    );
    assert_excludes_all(
        source,
        &[
            "DestroyWindow(parent)",
            "L\"AtlAxWin\"",
            "TEXT(ATLAXWIN_CLASS)",
            "AtlAxCreateControlEx(",
            "AtlAxAttachControl(",
            "AtlAxGetControl(",
            "AtlAxGetHost(",
            "945EE98E-B376-4EC2-B2E5-64C9410F93B7",
            "SetParent(",
            "put_UIParentWindowHandle(static_cast<LONG>",
            "put_UIParentWindowHandle(static_cast<long>",
            "put_UIParentWindowHandle(static_cast<LONG_PTR>",
            "put_UIParentWindowHandle(static_cast<long long>",
            "put_UIParentWindowHandle(reinterpret_cast<LONG>",
            "put_UIParentWindowHandle(reinterpret_cast<LONG_PTR>",
            "put_UIParentWindowHandle((LONG)",
            "put_UIParentWindowHandle((long)",
        ],
    );
}

#[test]
fn gpui_smoke_uses_a_true_child_overlay_before_showing_the_active_x_host() {
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/native_overlay.rs",
        &[
            "ensure_owner_clips_children(owner_window)?",
            "GetParent(overlay)",
        ],
    );
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/native_overlay/window.rs",
        &[
            "const WS_CHILD: u32",
            "SetWindowLongPtrW(owner, GWL_STYLE, style_after as isize)",
            "style_before | WS_CLIPCHILDREN as usize",
            "SWP_FRAMECHANGED",
            "WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT",
            "CreateWindowExW(",
            "WS_EX_NOPARENTNOTIFY",
            "SetWindowPos(",
            "SWP_NOACTIVATE | SWP_SHOWWINDOW",
        ],
    );
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/native_overlay/diagnostics.rs",
        &[
            "GetWindow(window, GW_HWNDFIRST)",
            "ScreenToClient(owner, &mut origin)",
            "owner_clip_children={}",
            "overlay_is_first={}",
        ],
    );
    assert_excludes_all(
        "tools/gpui-rdp-smoke/src/native_overlay.rs",
        &["WS_POPUP", "WS_EX_TOOLWINDOW", "ClientToScreen", "GW_OWNER"],
    );
    assert_tokens_in_scope(
        "tools/gpui-rdp-smoke/src/windows_app/session.rs",
        "fn finish_initialization(",
        "\n}\n\nfn configure_presentation",
        &[
            "session.overlay.synchronize((0, 0, bounds.0, bounds.1))",
            "configure_presentation(&mut session, bounds)",
            "connect_session(&mut session, &credentials, &connection_options)",
        ],
    );
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/windows_app/view.rs",
        &[
            "const LOGIN_PRESENTATION_REFRESH_DELAY: Duration = Duration::from_millis(300)",
            "login_presentation_refresh_task: Option<Task<()>>",
        ],
    );
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/windows_app/view/events.rs",
        &["support::spawn_login_presentation_refresh("],
    );
    assert_contains_all(
        "tools/gpui-rdp-smoke/src/windows_app/view/presentation.rs",
        &["session.overlay.refresh((0, 0, bounds.0, bounds.1))"],
    );
    assert_tokens_in_scope(
        "tools/gpui-rdp-smoke/src/windows_app/view/support.rs",
        "pub(super) fn spawn_login_presentation_refresh(",
        "\n}\n\npub(super) fn defer_initialization",
        &[
            "cx.background_executor().timer(delay).await",
            "view.update_in(cx, |view, window, _cx|",
            "current_generation != Some(token.generation)",
            "view.present_login_complete(",
        ],
    );
}

#[test]
fn product_native_rdp_uses_a_true_child_overlay_without_implicit_show() {
    assert_contains_all(
        "crates/remote_desktop_view/src/view/windows_native_overlay/window.rs",
        &[
            "const WS_CHILD: u32",
            "style_before | WS_CLIPCHILDREN as usize",
            "SetWindowLongPtrW(owner, GWL_STYLE, style_after as isize)",
            "SWP_FRAMECHANGED",
            "WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS | SS_BLACKRECT",
            "CreateWindowExW(",
            "WS_EX_NOPARENTNOTIFY",
            "GetParent(overlay)",
            "SetWindowPos(",
            "SWP_NOACTIVATE",
        ],
    );
    assert_excludes_all(
        "crates/remote_desktop_view/src/view/windows_native_overlay/window.rs",
        &["WS_POPUP", "WS_EX_TOOLWINDOW", "SWP_SHOWWINDOW"],
    );
}

#[test]
fn product_native_rdp_hosts_active_x_below_the_overlay() {
    assert_tokens_in_scope(
        "crates/remote_desktop_view/src/view/windows_native.rs",
        "pub(crate) fn create_with_owner(",
        "\n    pub(crate) fn create(",
        &[
            "WindowsNativeOverlay::create(owner, generation)?",
            "WindowsRdpParentWindow::from_raw(overlay.hwnd())",
            "WindowsRdpHost::create_with_parent(",
            "overlay,",
            "host,",
        ],
    );
}

#[test]
fn product_native_rdp_shutdown_keeps_overlay_alive_until_host_is_destroyed() {
    assert_tokens_in_scope(
        "crates/remote_desktop_view/src/view/windows_native.rs",
        "pub(crate) fn finish_destroy(",
        "\n    pub(crate) fn force_close",
        &[
            "match self.host.close()",
            "self.overlay.close()?",
            "self.presentation.finish_destroy()",
        ],
    );
    assert_tokens_in_scope(
        "crates/remote_desktop_view/src/view/windows_native.rs",
        "pub(crate) fn finish_destroy(",
        "\n    pub(crate) fn force_close",
        &[
            "WindowsRdpHostError::CallbackInFlight",
            "NativeDestroyProgress::PendingCallbacks",
        ],
    );
}

#[test]
fn product_native_rdp_coordinates_overlay_and_host_presentation_order() {
    assert_tokens_in_scope(
        "crates/remote_desktop_view/src/view/windows_native.rs",
        "impl NativePresentationSink for WindowsNativePresentationSink<'_> {",
        "\n}\n\n#[cfg(test)]",
        &[
            "fn set_bounds",
            "self.overlay.set_bounds(",
            "self.host.set_bounds(0, 0",
            "fn show",
            "self.overlay.show()?",
            "self.host.set_visible(true)",
            "self.overlay.log_composition_diagnostics(\"show_complete\")",
            "fn hide",
            "self.host.set_visible(false)",
            "self.overlay.hide()",
        ],
    );
}

#[test]
fn product_native_rdp_close_attempts_destroy_even_when_hide_fails() {
    let path = "crates/remote_desktop_view/src/view/windows_native_overlay/lifecycle.rs";
    assert_tokens_in_scope(
        path,
        "pub(crate) fn close(&mut self)",
        "\n    pub(crate) fn abandon",
        &[
            "let hide_error = self.hide_actual().err();",
            "DestroyWindow(window)",
            "if let Some(error) = hide_error",
            "failed to hide Windows native RDP overlay before destroy",
            "self.window = 0;",
            "self.last_bounds = None;",
        ],
    );
    let source = read(path);
    let close = source
        .split("pub(crate) fn close(&mut self)")
        .nth(1)
        .unwrap()
        .split("\n    pub(crate) fn abandon")
        .next()
        .unwrap();
    assert!(!close.contains("self.hide_actual()?;"));
}

#[test]
fn active_x_event_sink_maps_known_dispids_and_unadvises_before_releasing_the_control() {
    let sink = &format!("{HOST_CRATE}/native/event_sink.cpp");
    let active_x = &format!("{HOST_CRATE}/native/active_x_host.cpp");
    let internal = &format!("{HOST_CRATE}/native/host_internal.h");
    let public_header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");

    assert_contains_all(
        sink,
        &[
            "class RdpEventSink final : public IDispatch",
            "IID_IUnknown",
            "IID_IDispatch",
            "__uuidof(IMsTscAxEvents)",
            "InterlockedIncrement(&ref_count_)",
            "InterlockedDecrement(&ref_count_)",
            "NativeRdpHost* host_;",
            "void detach() noexcept",
            "class RdpEventSink",
            "IConnectionPointContainer",
            "FindConnectionPoint(",
            "connection_point->Advise(",
            "trace_native_stage(\"event_subscription.query_container.before\")",
            "trace_native_stage(\"event_subscription.find_connection_point.before\")",
            "trace_native_stage(\"event_subscription.advise.before\")",
            "trace_native_stage(\"event_subscription.complete\")",
            "DWORD advise_cookie = 0;",
            "subscription->advise_cookie = advise_cookie;",
            "NAVOP_RDP_EVENT_CONNECTING",
            "NAVOP_RDP_EVENT_CONNECTED",
            "NAVOP_RDP_EVENT_LOGIN_COMPLETE",
            "NAVOP_RDP_EVENT_RECONNECTING",
            "NAVOP_RDP_EVENT_RECONNECTED",
            "NAVOP_RDP_EVENT_NETWORK_STATUS_CHANGED",
            "NAVOP_RDP_EVENT_REMOTE_DESKTOP_SIZE_CHANGED",
            "NAVOP_RDP_EVENT_ENTER_FULLSCREEN",
            "NAVOP_RDP_EVENT_LEAVE_FULLSCREEN",
            "NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISPLAYED",
            "NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISMISSED",
            "NAVOP_RDP_EVENT_WARNING",
            "NAVOP_RDP_EVENT_FATAL_ERROR",
            "NAVOP_RDP_EVENT_LOGON_ERROR",
            "NAVOP_RDP_EVENT_DISCONNECTED",
            "NAVOP_RDP_EVENT_CLOSE_CONFIRMED",
            "NAVOP_RDP_EVENT_FOCUS_RELEASED",
            "dispatch_disconnected_from_parameters(host, parameters)",
            "event_sink.detached_callback",
            "event_sink.dispatch_failure",
            "event_sink.exception",
            "event_sink.unknown_dispatch_id",
            "event_sink.size_change.invalid_parameter_count",
            "event_sink.size_change.invalid_parameter_types",
            "event_sink.focus_released.direction",
            "event_sink.network_status.bandwidth",
            "event_sink.network_status.round_trip_time",
            "event_sink.auto_reconnecting2.disconnect_reason",
            "event_subscription.destroy.unadvise.before",
            "event_subscription.destroy.unadvise.after",
            "event_subscription.destroy.release_connection_point.before",
            "event_subscription.destroy.release_sink.before",
            "event_subscription.destroy.complete",
            "get_active_x_extended_disconnect_reason(",
            "trace_active_x_disconnect_description(",
            "extended_result == NAVOP_RDP_RESULT_OK",
            "parameters->rgvarg[1]",
            "parameters->rgvarg[0]",
            "encode_u32_le(",
            "default:",
            "return S_OK;",
            "extern \"C\" NavopRdpResult navop_rdp_test_invoke_active_x_event(",
            "extern \"C\" NavopRdpResult navop_rdp_test_dispatch_disconnect_event(",
        ],
    );
    assert_tokens_in_scope(
        sink,
        "void destroy_event_subscription(",
        "\n}\n\nextern \"C\" NavopRdpResult navop_rdp_test_invoke_active_x_event(",
        &[
            "sink->detach();",
            "subscription->connection_point->Unadvise(",
            "subscription->advise_cookie = 0;",
            "subscription->connection_point.Release();",
            "sink->Release();",
            "delete subscription;",
        ],
    );
    assert_tokens_in_scope(
        active_x,
        "~ActiveXCleanup() noexcept",
        "\n    }\n};",
        &[
            "destroy_event_subscription(event_subscription);",
            "client.Release();",
            "control.Release();",
            "container.Release();",
            "DestroyWindow(host_window);",
            "AtlAxWinTerm();",
            "OleUninitialize();",
        ],
    );
    assert_contains_all(
        internal,
        &[
            "struct NativeRdpEventSubscription;",
            "NavopRdpResult create_event_subscription(",
            "void destroy_event_subscription(",
            "NavopRdpResult get_active_x_extended_disconnect_reason(",
            "void trace_active_x_disconnect_description(",
        ],
    );
    assert_contains_all(
        active_x,
        &[
            "ExtendedDisconnectReasonCode extended_reason{};",
            "resources->state.client->get_ExtendedDisconnectReason(",
            "*out_extended_code = static_cast<int32_t>(extended_reason);",
            "resources->state.client->GetErrorDescription(",
            "trace_native_utf16(",
            "\"disconnect.error_description\"",
        ],
    );
    assert_excludes_all(
        public_header,
        &[
            "NativeRdpEventSubscription",
            "navop_rdp_test_invoke_active_x_event",
            "navop_rdp_test_dispatch_disconnect_event",
            "IConnectionPoint",
            "IMsTscAxEvents",
        ],
    );
    assert_excludes_all(
        sink,
        &[
            "CComPtr<NativeRdpHost>",
            "CComPtr<IMsRdpClient",
            "std::shared_ptr<NativeRdpHost>",
        ],
    );
}

#[test]
fn connection_options_and_rust_facade_keep_the_minimal_slice_separate() {
    assert_contains_all(
        &format!("{HOST_CRATE}/src/options.rs"),
        &[
            "WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS",
            "pub enum WindowsRdpColorDepth",
            "pub struct WindowsRdpConnectionOptions",
            "host_utf16_len",
            "encode_utf16",
            "host.contains('\\0')",
            "pub(crate) fn as_native",
            "_host_utf16: Vec<u16>",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &[
            "InvalidState",
            "ffi::RESULT_INVALID_STATE",
            "operation is invalid in the current state",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "pub enum WindowsRdpConnectionState",
            "pub enum WindowsRdpRequestCloseStatus",
            "pub fn connect(",
            "pub fn connection_state(",
            "pub fn request_close(",
            "pub fn disconnect(",
            "CONNECTION_STATE_DISCONNECTED",
            "REQUEST_CLOSE_CAN_PROCEED",
            "InvalidNativeResponse",
            "native_options owns the UTF-16 storage",
            "retains neither the struct nor its host pointer",
        ],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/options.rs"),
        &[
            "server_password",
            "gateway_password",
            "NavopRdpCredentialBundle",
            "password",
            "secret",
        ],
    );
}

#[test]
fn native_connection_entrypoints_validate_gate_outputs_and_exceptions() {
    for path in ["native/configuration.cpp", "native/lifecycle.cpp"] {
        assert_contains_all(
            &format!("{HOST_CRATE}/{path}"),
            &[
                "try {",
                "ensure_owner_thread(host)",
                "callback_state != CallbackState::Open",
                "catch (...)",
                "NAVOP_RDP_RESULT_INTERNAL_ERROR",
            ],
        );
    }
    assert_contains_all(
        &format!("{HOST_CRATE}/native/configuration.cpp"),
        &[
            "struct_size < NAVOP_RDP_CONNECTION_LEGACY_SIZE",
            "connection_field_available",
            "read_connection_field",
            "validate_abi_version",
            "(options.flags & ~NAVOP_RDP_CONNECTION_FLAGS_KNOWN) != 0",
            "NAVOP_RDP_MAX_HOST_UTF16_CODE_UNITS",
            "options.host.data == nullptr",
            "options.port > UINT32_C(65535)",
            "options.desktop_width <= 0",
            "options.desktop_height <= 0",
            "valid_color_depth",
            "text.data[index] == 0",
            "connect_active_x(host, host->active_x_resources, normalized)",
        ],
    );
    assert_tokens_in_scope(
        &format!("{HOST_CRATE}/native/lifecycle.cpp"),
        "extern \"C\" NavopRdpResult navop_rdp_get_connection_state(",
        "extern \"C\" NavopRdpResult navop_rdp_request_close(",
        &[
            "out_state == nullptr",
            "*out_state = UINT32_C(0)",
            "host == nullptr",
            "NavopRdpResult result = ensure_owner_thread(host)",
            "if (result != NAVOP_RDP_RESULT_OK)",
            "clear_last_error(host)",
            "callback_state != CallbackState::Open",
        ],
    );
    assert_tokens_in_scope(
        &format!("{HOST_CRATE}/native/lifecycle.cpp"),
        "extern \"C\" NavopRdpResult navop_rdp_request_close(",
        "extern \"C\" NavopRdpResult navop_rdp_disconnect(",
        &[
            "out_status == nullptr",
            "*out_status = UINT32_C(0)",
            "host == nullptr",
            "NavopRdpResult result = ensure_owner_thread(host)",
            "if (result != NAVOP_RDP_RESULT_OK)",
            "clear_last_error(host)",
            "callback_state != CallbackState::Open",
            "return request_close_active_x(host, host->active_x_resources, out_status);",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/lifecycle.cpp"),
        &["return disconnect_active_x(host, host->active_x_resources);"],
    );
}

#[test]
fn native_session_display_entrypoint_validates_headers_and_state() {
    assert_tokens_in_scope(
        &format!("{HOST_CRATE}/native/host.cpp"),
        "extern \"C\" NavopRdpResult navop_rdp_update_session_display_settings(",
        "extern \"C\" NavopRdpResult navop_rdp_set_visible(",
        &[
            "try {",
            "if (host == nullptr)",
            "const NavopRdpResult owner_result = ensure_owner_thread(host)",
            "if (owner_result != NAVOP_RDP_RESULT_OK)",
            "clear_last_error(host)",
            "settings == nullptr",
            "validate_struct_size(",
            "settings->struct_size",
            "sizeof(NavopRdpSessionDisplaySettings)",
            "settings->abi_version",
            "NAVOP_RDP_SESSION_DISPLAY_SETTINGS_ABI_VERSION",
            "settings->desktop_width == UINT32_C(0)",
            "settings->desktop_height == UINT32_C(0)",
            "settings->physical_width == UINT32_C(0)",
            "settings->physical_height == UINT32_C(0)",
            "settings->desktop_scale_factor == UINT32_C(0)",
            "settings->device_scale_factor == UINT32_C(0)",
            "host->callback_state != CallbackState::Open",
            "NAVOP_RDP_RESULT_INVALID_STATE",
            "update_active_x_session_display_settings(",
            "catch (...)",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
        ],
    );
}

#[test]
fn active_x_session_display_requires_connected_and_preserves_hresult() {
    assert_tokens_in_scope(
        &format!("{HOST_CRATE}/native/active_x_host.cpp"),
        "NavopRdpResult update_active_x_session_display_settings(",
        "NavopRdpResult set_active_x_visible(",
        &[
            "validate_resources(resources)",
            "short connected = 0;",
            "resources->state.client->get_Connected(&connected)",
            "\"display.get_connected.after\"",
            "\"display.connected_state\"",
            "if (FAILED(result))",
            "record_last_hresult(",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
            "static_cast<int32_t>(result)",
            "if (connected != 1)",
            "record_last_error(owner, NAVOP_RDP_RESULT_INVALID_STATE)",
            "\"display.update_session_display_settings.before\"",
            "resources->state.client->UpdateSessionDisplaySettings(",
            "static_cast<ULONG>(settings.desktop_width)",
            "static_cast<ULONG>(settings.desktop_height)",
            "static_cast<ULONG>(settings.physical_width)",
            "static_cast<ULONG>(settings.physical_height)",
            "static_cast<ULONG>(settings.orientation)",
            "static_cast<ULONG>(settings.desktop_scale_factor)",
            "static_cast<ULONG>(settings.device_scale_factor)",
            "\"display.update_session_display_settings.after\"",
            "if (FAILED(result))",
            "record_last_hresult(",
            "NAVOP_RDP_RESULT_INTERNAL_ERROR",
            "static_cast<int32_t>(result)",
            "return NAVOP_RDP_RESULT_OK;",
        ],
    );
}

#[test]
fn active_x_connect_aligns_initial_display_properties_with_axhost() {
    let display_policy = &format!("{HOST_CRATE}/native/connection_policy_display.cpp");
    let active_x = &format!("{HOST_CRATE}/native/active_x_host.cpp");

    assert_contains_all(
        display_policy,
        &[
            "configure_display_policy(",
            "L\"SmartSizing\"",
            "CComQIPtr<IMsRdpClientNonScriptable5>",
            "non_scriptable5->put_UseMultimon(",
            "L\"ContainerHandledFullScreen\"",
            "CComPtr<IMsRdpExtendedSettings> extended_settings;",
            "L\"DesktopScaleFactor\"",
            "L\"DeviceScaleFactor\"",
            "extended_settings->put_Property(",
            "connect.display.smart_sizing",
            "connect.display.use_multimon.before",
            "connect.display.use_multimon.after",
            "connect.display.container_handled_full_screen",
            "connect.display.desktop_scale_factor",
            "connect.display.device_scale_factor",
            "connect.display.span_monitors.best_effort_unsupported",
            "static_cast<ULONG>(options.desktop_scale_factor)",
            "static_cast<ULONG>(options.device_scale_factor)",
            "result == E_FAIL",
            "connect.display.extended_property.unsupported",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DESKTOP_SCALE_FACTOR",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DEVICE_SCALE_FACTOR",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/connection_policy.cpp"),
        &["get_advanced_settings8(", "get_AdvancedSettings9"],
    );
    assert_tokens_in_scope(
        active_x,
        "NavopRdpResult connect_active_x(",
        "NavopRdpResult apply_active_x_credentials(",
        &[
            "get_AdvancedSettings2",
            "put_RDPPort",
            "configure_active_x_connection_policy(",
            "put_DesktopWidth",
            "put_DesktopHeight",
            "Connect",
        ],
    );
    assert_excludes_all(
        active_x,
        &[
            "configure_extended_display_settings",
            "kDefaultDesktopScaleFactor",
            "kDefaultDeviceScaleFactor",
            "put_SmartSizing",
            "put_ContainerHandledFullScreen",
            "put_EncryptionEnabled",
            "put_PublicMode",
            "put_EnableCredSspSupport",
            "put_AuthenticationLevel(0)",
        ],
    );
}

#[test]
fn active_x_connect_order_and_borrowed_endpoint_contract_are_frozen() {
    let active_x = &format!("{HOST_CRATE}/native/active_x_host.cpp");
    assert_tokens_in_scope(
        active_x,
        "NavopRdpResult connect_active_x(",
        "NavopRdpResult get_active_x_connection_state(",
        &[
            "get_Connected",
            "put_Server",
            "get_AdvancedSettings2",
            "put_RDPPort",
            "configure_active_x_connection_policy(",
            "put_DesktopWidth",
            "put_DesktopHeight",
            "put_ColorDepth",
            "Connect",
        ],
    );
    assert_contains_all(
        active_x,
        &[
            "if (FAILED(result))",
            "CComBSTR server(",
            "static_cast<int>(options.host.len)",
            "reinterpret_cast<LPCOLESTR>(options.host.data)",
            "IMsRdpClientAdvancedSettings",
            "RequestClose",
            "Disconnect",
            "controlCloseCanProceed",
            "controlCloseWaitForEvents",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/connection_policy_session.cpp"),
        &[
            "IMsRdpClientAdvancedSettings8",
            "L\"EnableCredSspSupport\"",
            "L\"PublicMode\"",
            "L\"AuthenticationLevel\"",
            "L\"EncryptionEnabled\"",
        ],
    );
    let runtime_policy = &format!("{HOST_CRATE}/native/connection_policy_runtime.cpp");
    assert_contains_all(
        runtime_policy,
        &[
            "CComQIPtr<IMsRdpClientNonScriptable3>",
            "non_scriptable3->put_RedirectDynamicDrives(",
            "non_scriptable3->put_RedirectDynamicDevices(",
            "advanced->put_RedirectPorts(",
            "connect.resource.redirect_dynamic_drives.before",
            "connect.resource.redirect_dynamic_devices.before",
            "connect.resource.redirect_serial_ports.before",
            "L\"PerformanceFlags\"",
            "L\"NetworkConnectionType\"",
        ],
    );
    assert_excludes_all(
        runtime_policy,
        &[
            "L\"RedirectDynamicDrives\"",
            "L\"RedirectDynamicDevices\"",
            "L\"RedirectSerialPorts\"",
        ],
    );
    assert_excludes_all(active_x, &["wcslen", "lstrlenW", "printf", "std::cout"]);
    assert_excludes_all(
        &format!("{HOST_CRATE}/native/configuration.cpp"),
        &[
            "wcslen",
            "lstrlenW",
            "printf",
            "std::cout",
            "std::cerr",
            "OutputDebugString",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/native/windows_rdp_host.h"),
        &[
            "not",
            "NUL-terminated",
            "len is authoritative",
            "copies the endpoint into a temporary COM string",
            "does not retain data after the call returns",
        ],
    );
}

#[test]
fn native_unsupported_resource_and_gateway_requests_fail_closed() {
    let runtime_policy = &format!("{HOST_CRATE}/native/connection_policy_runtime.cpp");
    assert_contains_all(
        runtime_policy,
        &[
            "if ((options.resource_flags & NAVOP_RDP_RESOURCE_FLAG_CAMERAS) != 0)",
            "connect.resource.cameras.unavailable",
            "NAVOP_RDP_RESULT_UNAVAILABLE",
        ],
    );
    assert_excludes_all(runtime_policy, &["connect.resource.cameras.unsupported"]);

    let gateway_policy = &format!("{HOST_CRATE}/native/connection_policy_gateway.cpp");
    let gateway_contents = read(gateway_policy);
    let disabled_end = gateway_contents
        .find("if (options.gateway_mode == NAVOP_RDP_GATEWAY_MODE_NONE)")
        .expect("gateway disabled branch must remain explicit");
    let bypass_check = gateway_contents
        .find("if ((options.gateway_flags & NAVOP_RDP_GATEWAY_FLAG_BYPASS_LOCAL) != 0)")
        .expect("gateway bypass-local check must remain explicit");
    assert!(
        disabled_end < bypass_check,
        "disabled gateway must short-circuit before bypass-local rejection"
    );
    assert_contains_all(
        gateway_policy,
        &[
            "connect.gateway.bypass_local.unavailable",
            "return record_last_error(",
            "NAVOP_RDP_RESULT_UNAVAILABLE",
        ],
    );
    assert_excludes_all(
        gateway_policy,
        &["connect.gateway.bypass_local.best_effort"],
    );
}

#[test]
fn native_input_policy_sets_keyboard_hook_mode_through_secured_settings() {
    let runtime_policy = &format!("{HOST_CRATE}/native/connection_policy_runtime.cpp");
    assert_tokens_in_scope(
        runtime_policy,
        "NavopRdpResult configure_input_policy(",
        "NavopRdpResult configure_performance_policy(",
        &[
            "CComQIPtr<IMsRdpClient7> client7(context.client);",
            "CComPtr<IMsRdpClientSecuredSettings2> secured_settings3;",
            "connect.input.get_secured_settings3.before",
            "client7->get_SecuredSettings3(&secured_settings3)",
            "connect.input.get_secured_settings3.after",
            "connect.input.keyboard_hook_mode.before",
            "secured_settings3->put_KeyboardHookMode(",
            "options.keyboard_hook_mode",
            "connect.input.keyboard_hook_mode.after",
            "hresult == DISP_E_UNKNOWNNAME",
            "connect.input.keyboard_hook_mode.unsupported",
            "get_advanced_settings8(",
            "L\"EnableWindowsKey\"",
            "L\"GrabFocusOnConnect\"",
        ],
    );

    let contents = read(runtime_policy);
    let (_, input_policy) = contents
        .split_once("NavopRdpResult configure_input_policy(")
        .unwrap_or_else(|| panic!("{runtime_policy} must define configure_input_policy"));
    let (input_policy, _) = input_policy
        .split_once("NavopRdpResult configure_performance_policy(")
        .unwrap_or_else(|| panic!("{runtime_policy} must define configure_performance_policy"));
    for forbidden in [
        "L\"KeyboardHookMode\"",
        "NativeRdpDispatchTarget keyboard_hook",
    ] {
        assert!(
            !input_policy.contains(forbidden),
            "{runtime_policy} input policy must not contain `{forbidden}`"
        );
    }
}

#[test]
fn native_connection_sources_are_registered_in_the_windows_build() {
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rerun-if-changed=native/configuration.cpp",
            "cargo:rerun-if-changed=native/lifecycle.cpp",
            ".file(\"native/configuration.cpp\")",
            ".file(\"native/lifecycle.cpp\")",
        ],
    );
}

#[test]
fn rust_facade_owns_only_the_opaque_handle_and_uses_idempotent_destroy() {
    assert_contains_all(
        &format!("{HOST_CRATE}/src/lib.rs"),
        &[
            "pub use capabilities::WindowsRdpHostCapabilities;",
            "pub use error::WindowsRdpHostError;",
            "WindowsRdpDiagnosticCategory",
            "WindowsRdpDisconnectReason",
            "WindowsRdpDiagnosticContext",
            "WindowsRdpDiagnosticSnapshot",
            "WindowsRdpUsernameRedaction",
            "WindowsRdpEvent",
            "WindowsRdpRawEvent",
            "WindowsRdpConnectionState",
            "WindowsRdpHost",
            "WindowsRdpRequestCloseStatus",
            "pub use display::WindowsRdpSessionDisplaySettings;",
            "pub use lifecycle::WindowsRdpHostLifecycle;",
            "WINDOWS_RDP_MAX_HOST_UTF16_CODE_UNITS",
            "WindowsRdpColorDepth",
            "WindowsRdpConnectionOptions",
            "WindowsRdpHostOptions",
            "WindowsRdpParentWindow",
            "mod diagnostic;",
            "mod display;",
            "mod event;",
            "mod lifecycle;",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/handle.rs"),
        &[
            "pub struct WindowsRdpHost",
            "raw: *mut NativeRdpHost",
            "pub fn probe()",
            "pub fn create(",
            "pub unsafe fn create_with_parent(",
            "pub fn set_bounds(",
            "pub fn update_session_display_settings(",
            "pub fn set_visible(",
            "pub fn focus(",
            "pub fn connect(",
            "pub fn connection_state(",
            "pub fn request_close(",
            "pub fn disconnect(",
            "pub fn drain_events(&self) -> Vec<WindowsRdpRawEvent>",
            "pub fn close(&mut self)",
            "pub const fn lifecycle(&self) -> WindowsRdpHostLifecycle",
            "impl Drop for WindowsRdpHost",
            "(self.bindings.destroy)(&mut self.raw)",
            "WindowsRdpHostLifecycle::Open",
            "WindowsRdpHostLifecycle::Closing",
            "WindowsRdpHostLifecycle::Closed",
            "begin_closing",
            "unregister_event_callback",
            "close_retries_unregister_then_destroy_failures_without_reopening_callback_gate",
            "registration_failure_preserves_original_error_when_destroy_does_not_clear_handle",
            "drop_preserves_callback_context_when_unregister_keeps_failing",
            "if self.close().is_err() && self.callback_registered",
            "Box::leak(event_bridge)",
            "(self.bindings.set_bounds)(self.raw, &bounds)",
            "(self.bindings.update_session_display_settings)(self.raw, &native_settings)",
            "(self.bindings.set_visible)",
            "(self.bindings.focus)(self.raw)",
            "presentation_controls_forward_bounds_visibility_and_focus",
            "negative_presentation_dimensions_are_rejected_before_native_call",
            "presentation_failures_map_without_changing_lifecycle",
            "presentation_controls_are_rejected_before_native_when_closing_or_closed",
            "session_display_settings_forward_the_current_abi_and_all_fields",
            "session_display_settings_failures_map_without_changing_lifecycle",
            "session_display_settings_are_rejected_before_native_when_closing_or_closed",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/ffi.rs"),
        &[
            "type SetBoundsFn",
            "type UpdateSessionDisplaySettingsFn",
            "type SetVisibleFn",
            "type FocusFn",
            "type ConnectFn",
            "type GetConnectionStateFn",
            "type RequestCloseFn",
            "type DisconnectFn",
            "set_bounds: SetBoundsFn",
            "update_session_display_settings: UpdateSessionDisplaySettingsFn",
            "set_visible: SetVisibleFn",
            "focus: FocusFn",
            "connect: ConnectFn",
            "get_connection_state: GetConnectionStateFn",
            "request_close: RequestCloseFn",
            "disconnect: DisconnectFn",
            "navop_rdp_set_bounds(",
            "navop_rdp_update_session_display_settings(",
            "navop_rdp_set_visible(",
            "navop_rdp_focus(",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/options.rs"),
        &[
            "pub struct WindowsRdpParentWindow(usize);",
            "pub const unsafe fn from_raw(raw: usize) -> Self",
            "pub const fn as_raw(self) -> usize",
            "caller-owned native parent window handle",
            "host owner/UI thread",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/lifecycle.rs"),
        &[
            "pub enum WindowsRdpHostLifecycle",
            "Open",
            "Closing",
            "Closed",
            "callback admission",
        ],
    );
    for path in [
        "src/lib.rs",
        "src/ffi.rs",
        "src/handle.rs",
        "src/lifecycle.rs",
        "src/event.rs",
        "src/options.rs",
        "src/capabilities.rs",
        "src/error.rs",
    ] {
        assert_excludes_all(
            &format!("{HOST_CRATE}/{path}"),
            &[
                "HWND",
                "IUnknown",
                "BSTR",
                "CComPtr",
                "gpui",
                "remote_desktop_view",
            ],
        );
    }
}

#[test]
fn build_is_windows_hosted_msvc_only_and_ci_runs_host_tests() {
    assert_contains_all(
        &format!("{HOST_CRATE}/build.rs"),
        &[
            "cargo:rustc-check-cfg=cfg(windows_rdp_host_native)",
            "CARGO_CFG_TARGET_OS",
            "CARGO_CFG_TARGET_ENV",
            "CARGO_CFG_TARGET_ARCH",
            "HOST",
            "TARGET",
            "OUT_DIR",
            "host.cpp",
            "active_x_host.cpp",
            "event_sink.cpp",
            "cpp(true)",
            "/std:c++17",
            "/EHsc",
            "/W4",
            "/WX",
            "windows_rdp_host_native",
            "x86_64",
            "x86",
            "msvc",
            "cargo:rerun-if-env-changed=VCToolsInstallDir",
            "VCToolsInstallDir",
            "atlmfc",
            "atls.lib",
            "cargo:rustc-link-lib=static=atls",
            "\"ole32\", \"oleaut32\", \"user32\", \"uuid\"",
            ".define(\"UNICODE\", None)",
            ".define(\"_UNICODE\", None)",
        ],
    );
    let script_path = "script/build-windows-rdp-probe.ps1";
    assert_contains_all(
        script_path,
        &[
            "$supportedVisualStudioVersionRange = \"[17.0,19.0)\"",
            "cargo build --locked -p windows-rdp-probe --target $RustTarget",
            HOST_TEST,
            "cargo test --locked -p remote_desktop_view ",
            "--features windows-native-rdp --target $RustTarget",
            "Compile-only probe gate and native runtime tests",
        ],
    );
    let script = read(script_path);
    assert_eq!(
        script
            .matches("cargo test --locked -p windows_rdp_host --target $RustTarget")
            .count(),
        1,
        "{script_path} must contain exactly one host test command"
    );
    assert_eq!(
        script
            .matches("cargo test --locked -p remote_desktop_view ")
            .count(),
        1,
        "{script_path} must contain exactly one presentation feature test command"
    );
    assert_excludes_all(
        script_path,
        &[
            "cargo test --locked -p windows_rdp_host --target $RustTarget --no-run",
            "windows_rdp_host.exe",
            "windows-rdp-probe.exe",
        ],
    );
    assert_contains_all(
        ".github/workflows/ci.yml",
        &[
            "\"run_on\":\"windows-2022\"",
            "run: ./script/test-windows.ps1",
            "choco install nasm --no-progress --yes",
            "nasm -v",
        ],
    );
    assert_tokens_in_scope(
        ".github/workflows/ci.yml",
        "  test:",
        "  ci-gate:",
        &[
            "- uses: actions/checkout@v7",
            "- name: Install NASM",
            "choco install nasm --no-progress --yes",
            "$nasmDir = Join-Path $env:ProgramFiles \"NASM\"",
            "$nasmDir | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append",
            "nasm -v",
            "- name: Setup Rust toolchain",
            "- name: Build ATL/MSVC probe (x64 + x86)",
        ],
    );
    assert_contains_all(
        ".github/workflows/release.yml",
        &[
            "choco install nasm --no-progress --yes",
            "$nasmDir = Join-Path $env:ProgramFiles \"NASM\"",
            "$nasmDir | Out-File -FilePath $env:GITHUB_PATH -Encoding utf8 -Append",
            "nasm -v",
        ],
    );
    assert_contains_all(
        "script/test-windows.ps1",
        &[
            "Microsoft.VisualStudio.Workload.NativeDesktop",
            "Microsoft.VisualStudio.Component.VC.ATL",
            "vcvarsall.bat",
            "\"call `\"$vcvarsall`\" x64\"",
            "\"cargo test --all\"",
        ],
    );
}

#[test]
fn active_x_policy_sections_run_explicitly_in_frozen_order() {
    let policy_source = &format!("{HOST_CRATE}/native/connection_policy.cpp");
    let contents = read(policy_source);
    let (_, body) = contents
        .split_once("NavopRdpResult configure_active_x_connection_policy(")
        .unwrap_or_else(|| panic!("{policy_source} must contain the unified policy entrypoint"));

    // Every section is an explicit fail-fast call in the frozen order:
    // security, reconnect, input, resource, audio, display, performance,
    // gateway. A section failure short-circuits before the next section runs.
    let mut remaining = body;
    for token in [
        "configure_security_policy(context, options)",
        "configure_reconnect_policy(context, options)",
        "configure_input_policy(context, options)",
        "configure_resource_policy(context, options)",
        "configure_audio_redirection(",
        "configure_display_policy(context, options)",
        "configure_performance_policy(context, options)",
        "configure_gateway_policy(context, options)",
    ] {
        let position = remaining
            .find(token)
            .unwrap_or_else(|| panic!("{policy_source} must call `{token}` in the frozen order"));
        remaining = &remaining[position + token.len()..];
    }
    // The old noexcept function-pointer array must be gone.
    assert_excludes_all(
        policy_source,
        &["ConfigureSection sections[]", "using ConfigureSection"],
    );
    // Shared helpers are called from other policy translation units, so their
    // definitions must have external linkage (outside the anonymous namespace)
    // or MSVC leaves an unresolved external symbol.
    let namespace_close = contents
        .find("}  // namespace")
        .unwrap_or_else(|| panic!("{policy_source} must close its anonymous namespace"));
    for helper in [
        "NavopRdpResult get_advanced_settings8(",
        "NavopRdpResult configure_redirect_bool(",
    ] {
        let position = contents
            .find(helper)
            .unwrap_or_else(|| panic!("{policy_source} must define `{helper}`"));
        assert!(
            position > namespace_close,
            "`{helper}` must be defined outside the anonymous namespace"
        );
    }
    // The policy header is included before the generated mstscax.tlh in every
    // policy TU; it must forward declare the generated COM interface so the
    // declaration of get_advanced_settings8 remains valid.
    assert_contains_all(
        &format!("{HOST_CRATE}/native/connection_policy_internal.h"),
        &["struct IMsRdpClientAdvancedSettings8;"],
    );

    // The unified call must run before Connect and replace every inline
    // hardcoded ActiveX property.
    let active_x = &format!("{HOST_CRATE}/native/active_x_host.cpp");
    assert_tokens_in_scope(
        active_x,
        "NavopRdpResult connect_active_x(",
        "NavopRdpResult apply_active_x_credentials(",
        &["configure_active_x_connection_policy(", "Connect"],
    );
    assert_excludes_all(
        active_x,
        &[
            "configure_audio_redirection(\n        owner,\n        resources->state.client,\n        options.flags)",
            "put_AuthenticationLevel(0)",
            "put_EncryptionEnabled(1)",
        ],
    );
}

#[test]
fn native_dispatch_helpers_own_variant_and_bstr_lifecycles() {
    let dispatch_source = &format!("{HOST_CRATE}/native/dispatch_property.cpp");
    let internal_header = &format!("{HOST_CRATE}/native/host_internal.h");
    let policy_source = &format!("{HOST_CRATE}/native/connection_policy.cpp");
    let display_source = &format!("{HOST_CRATE}/native/connection_policy_display.cpp");

    // bool conversion must use an independent converted VARIANT, never an
    // in-place VariantChangeType on the source value.
    assert_contains_all(
        dispatch_source,
        &[
            "VariantChangeType(&converted, &value, 0, VT_BOOL)",
            "VariantClear(&converted)",
        ],
    );
    assert_excludes_all(dispatch_source, &["VariantChangeType(&value, &value"]);
    // Every property-get failure path clears the VARIANT it initialized.
    assert_contains_all(
        dispatch_source,
        &[
            "if (FAILED(result)) {\n        VariantClear(&value);\n        return result;\n    }",
            "if (FAILED(result)) {\n        // A failed property-get may still have written a partial VARIANT;",
            "VariantClear(out_value);",
        ],
    );
    // get_dispatch_object returns an AddRef'd pointer; CComPtr callers must
    // Attach it, and both VT_DISPATCH and VT_UNKNOWN results are resolved.
    assert_contains_all(
        dispatch_source,
        &[
            "value.vt == VT_DISPATCH",
            "value.vt == VT_UNKNOWN",
            "resolved->AddRef();",
            "must use Attach, never assignment",
        ],
    );
    // An empty string is a valid empty BSTR; only a non-zero length with a
    // null pointer is E_POINTER.
    assert_contains_all(
        dispatch_source,
        &[
            "len == 0 with a null\n    // data pointer must succeed",
            "if (value.len != UINT32_C(0) && value.data == nullptr)",
            "return E_POINTER;",
            "SysAllocStringLen(",
        ],
    );
    assert_contains_all(
        internal_header,
        &[
            "HRESULT get_dispatch_object(",
            "HRESULT get_dispatch_bool(",
            "HRESULT set_dispatch_bool(",
            "HRESULT set_dispatch_long(",
            "HRESULT set_dispatch_utf16(",
        ],
    );
    // Version-dependent display enhancements may be absent from an older
    // mstscax.dll. Dispatch properties ignore only an absent name. Extended
    // scale properties also tolerate the compatibility HRESULTs returned by
    // controls that expose the interface without implementing the property.
    assert_tokens_in_scope(
        policy_source,
        "NavopRdpResult set_optional_dispatch_bool_if_supported(",
        "\n}\n\nNavopRdpResult configure_active_x_connection_policy(",
        &[
            "set_dispatch_bool(",
            "trace_native_hresult(",
            "result == DISP_E_UNKNOWNNAME",
            "return NAVOP_RDP_RESULT_OK;",
            "if (FAILED(result))",
            "record_last_hresult(",
            "static_cast<int32_t>(result)",
            "return NAVOP_RDP_RESULT_OK;",
        ],
    );
    assert_tokens_in_scope(
        policy_source,
        "NavopRdpResult set_required_dispatch_bool(",
        "NavopRdpResult set_required_dispatch_long(",
        &[
            "set_dispatch_bool(",
            "return required_dispatch_result(owner, target, result);",
        ],
    );
    assert_tokens_in_scope(
        display_source,
        "NavopRdpResult optional_extended_property_result(",
        "NavopRdpResult configure_extended_scale_factors(",
        &[
            "result == DISP_E_UNKNOWNNAME",
            "result == E_NOTIMPL",
            "result == E_NOINTERFACE",
            "result == E_FAIL",
            "connect.display.extended_property.unsupported",
            "return NAVOP_RDP_RESULT_OK;",
            "if (FAILED(result))",
            "record_last_stage_hresult(",
            "static_cast<int32_t>(result)",
            "return NAVOP_RDP_RESULT_OK;",
        ],
    );
    assert_tokens_in_scope(
        display_source,
        "NavopRdpResult configure_extended_scale_factors(",
        "\nNavopRdpResult configure_display_policy(",
        &[
            "L\"DesktopScaleFactor\"",
            "optional_extended_property_result(",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DESKTOP_SCALE_FACTOR",
            "L\"DeviceScaleFactor\"",
            "NAVOP_RDP_STAGE_CONNECT_DISPLAY_DEVICE_SCALE_FACTOR",
        ],
    );
    assert_tokens_in_scope(
        display_source,
        "NavopRdpResult configure_display_policy(",
        "return configure_extended_scale_factors(",
        &[
            "L\"SmartSizing\"",
            "set_optional_dispatch_bool_if_supported(",
            "non_scriptable5->put_UseMultimon(",
            "L\"ContainerHandledFullScreen\"",
            "set_optional_dispatch_bool_if_supported(",
        ],
    );
}

#[test]
fn reconnect_defaults_and_bounds_are_shared_across_the_abi() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");
    let ffi = &format!("{HOST_CRATE}/src/ffi.rs");
    let policy = &format!("{HOST_CRATE}/src/policy/reconnect.rs");
    let native_config = &format!("{HOST_CRATE}/native/configuration.cpp");
    let core_security = "crates/core/src/storage/rdp_settings/security.rs";

    assert_contains_all(header, &["NAVOP_RDP_MAX_RECONNECT_ATTEMPTS UINT32_C(200)"]);
    assert_contains_all(
        ffi,
        &[
            "MAX_RECONNECT_ATTEMPTS: u32 = 200",
            "MAX_KEEP_ALIVE_SECONDS: u32 = (i32::MAX / 1_000) as u32",
            "options.keep_alive_seconds > MAX_KEEP_ALIVE_SECONDS",
            "options.max_reconnect_attempts > MAX_RECONNECT_ATTEMPTS",
        ],
    );
    assert_contains_all(
        policy,
        &[
            "max_reconnect_attempts: MAX_RECONNECT_ATTEMPTS",
            "keep_alive_valid = self.keep_alive_seconds <= MAX_KEEP_ALIVE_SECONDS",
            "reconnect_valid = self.max_reconnect_attempts <= MAX_RECONNECT_ATTEMPTS",
        ],
    );
    assert_contains_all(
        native_config,
        &[
            "normalized.max_reconnect_attempts = NAVOP_RDP_MAX_RECONNECT_ATTEMPTS;",
            "options.max_reconnect_attempts > NAVOP_RDP_MAX_RECONNECT_ATTEMPTS",
            "(std::numeric_limits<int32_t>::max)() / INT32_C(1000)",
        ],
    );
    assert_contains_all(
        core_security,
        &["DEFAULT_RDP_MAX_RECONNECT_ATTEMPTS: u32 = 200"],
    );
    assert_excludes_all(
        &format!("{HOST_CRATE}/src/options_tests.rs"),
        &["max_reconnect_attempts: 20"],
    );
}

#[test]
fn presentation_state_query_abi_is_frozen() {
    let header = &format!("{HOST_CRATE}/native/windows_rdp_host.h");
    let host_source = &format!("{HOST_CRATE}/native/host.cpp");
    let active_x = &format!("{HOST_CRATE}/native/active_x_host.cpp");
    let ffi = &format!("{HOST_CRATE}/src/ffi.rs");
    let handle = &format!("{HOST_CRATE}/src/handle.rs");

    assert_contains_all(
        header,
        &[
            "NAVOP_RDP_PRESENTATION_STATE_ABI_VERSION UINT32_C(1)",
            "typedef struct NavopRdpPresentationState",
            "uint32_t control_hwnd_valid;",
            "uint32_t host_rect_nonzero;",
            "uint32_t control_rect_nonzero;",
            "uint32_t control_visible;",
            "uint32_t control_is_host_descendant;",
            "uint32_t host_visible;",
            "navop_rdp_get_presentation_state(",
            "static_assert(sizeof(NavopRdpPresentationState) == 32)",
            "static_assert(alignof(NavopRdpPresentationState) == 4)",
        ],
    );
    assert_contains_all(
        ffi,
        &[
            "RESULT_PRESENTATION_INCOMPLETE: NativeResult = 9",
            "struct NavopRdpPresentationState",
            "PRESENTATION_STATE_ABI_VERSION",
            "type GetPresentationStateFn",
            "get_presentation_state: GetPresentationStateFn",
            "fn navop_rdp_get_presentation_state(",
            "size_of::<NavopRdpPresentationState>()",
        ],
    );
    assert_contains_all(
        handle,
        &[
            "pub fn presentation_state(",
            "WindowsRdpPresentationState::from_native",
        ],
    );
    assert_contains_all(
        host_source,
        &["extern \"C\" NavopRdpResult navop_rdp_get_presentation_state("],
    );
    assert_contains_all(
        active_x,
        &[
            "NavopRdpResult get_active_x_presentation_state(",
            "control_is_host_descendant",
            "presentation.state.control_hwnd_valid",
        ],
    );
    assert_contains_all(
        &format!("{HOST_CRATE}/src/error.rs"),
        &["RESULT_PRESENTATION_INCOMPLETE", "PresentationIncomplete"],
    );
}

#[test]
fn policy_section_sources_are_registered_in_the_windows_build() {
    let build_script = &format!("{HOST_CRATE}/build.rs");
    assert_contains_all(
        build_script,
        &[
            "cargo:rerun-if-changed=native/connection_policy.cpp",
            "cargo:rerun-if-changed=native/connection_policy_internal.h",
            "cargo:rerun-if-changed=native/connection_policy_display.cpp",
            "cargo:rerun-if-changed=native/connection_policy_runtime.cpp",
            "cargo:rerun-if-changed=native/connection_policy_gateway.cpp",
            "cargo:rerun-if-changed=native/connection_policy_session.cpp",
            "cargo:rerun-if-changed=native/dispatch_property.cpp",
            ".file(\"native/connection_policy.cpp\")",
            ".file(\"native/connection_policy_display.cpp\")",
            ".file(\"native/connection_policy_runtime.cpp\")",
            ".file(\"native/connection_policy_session.cpp\")",
            ".file(\"native/connection_policy_gateway.cpp\")",
            ".file(\"native/dispatch_property.cpp\")",
        ],
    );
}
