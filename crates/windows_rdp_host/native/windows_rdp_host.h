#pragma once

#include <stddef.h>
#include <stdint.h>

#define NAVOP_RDP_ABI_VERSION UINT32_C(1)
#define NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION UINT32_C(1)
#define NAVOP_RDP_SESSION_DISPLAY_SETTINGS_ABI_VERSION UINT32_C(1)
#define NAVOP_RDP_PRESENTATION_STATE_ABI_VERSION UINT32_C(1)

typedef struct NativeRdpHost NativeRdpHost;

typedef int32_t NavopRdpResult;

#define NAVOP_RDP_RESULT_OK INT32_C(0)
#define NAVOP_RDP_RESULT_INVALID_ARGUMENT INT32_C(1)
#define NAVOP_RDP_RESULT_ABI_MISMATCH INT32_C(2)
#define NAVOP_RDP_RESULT_ALLOCATION_FAILED INT32_C(3)
#define NAVOP_RDP_RESULT_INTERNAL_ERROR INT32_C(4)
#define NAVOP_RDP_RESULT_UNAVAILABLE INT32_C(5)
#define NAVOP_RDP_RESULT_WRONG_THREAD INT32_C(6)
#define NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT INT32_C(7)
#define NAVOP_RDP_RESULT_INVALID_STATE INT32_C(8)
/*
 * The native child window is live but the ActiveX drawing window has not been
 * positioned inside the host subtree yet. The connection may still complete;
 * callers should re-synchronize bounds after the next LoginComplete/Reconnected
 * event instead of treating this as a terminal failure.
 */
#define NAVOP_RDP_RESULT_PRESENTATION_INCOMPLETE INT32_C(9)
#define NAVOP_RDP_MAX_HOST_UTF16_CODE_UNITS UINT32_C(255)
#define NAVOP_RDP_LAST_ERROR_LEGACY_SIZE UINT32_C(24)
#if INTPTR_MAX == INT64_MAX
#define NAVOP_RDP_CREDENTIAL_LEGACY_SIZE UINT32_C(48)
#define NAVOP_RDP_CONNECTION_LEGACY_SIZE UINT32_C(48)
#elif INTPTR_MAX == INT32_MAX
#define NAVOP_RDP_CREDENTIAL_LEGACY_SIZE UINT32_C(28)
#define NAVOP_RDP_CONNECTION_LEGACY_SIZE UINT32_C(36)
#else
#error Unsupported pointer width for the Windows RDP ABI
#endif

#define NAVOP_RDP_CREATE_STAGE_NONE UINT32_C(0)
#define NAVOP_RDP_CREATE_STAGE_OLE_INITIALIZE UINT32_C(1)
#define NAVOP_RDP_CREATE_STAGE_ATL_AX_WIN_INIT UINT32_C(2)
#define NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW UINT32_C(3)
#define NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL UINT32_C(4)
#define NAVOP_RDP_CREATE_STAGE_QUERY_CLIENT UINT32_C(5)
#define NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE UINT32_C(6)
#define NAVOP_RDP_CREATE_STAGE_SET_PARENT UINT32_C(7)
#define NAVOP_RDP_CREATE_STAGE_EVENT_SUBSCRIPTION UINT32_C(8)
#define NAVOP_RDP_CREATE_STAGE_EXCEPTION UINT32_C(9)

/*
 * Versioned structs accept struct_size values greater than or equal to the
 * current layout. Implementations access only the known prefix, preserve an
 * output struct's caller-provided size, and leave unknown trailing fields
 * untouched.
 */
typedef struct NavopRdpProbeOptions {
    uint32_t struct_size;
    uint32_t abi_version;
} NavopRdpProbeOptions;

typedef struct NavopRdpProbeResult {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t available;
    uint32_t reserved;
} NavopRdpProbeResult;

/*
 * Synchronous native diagnostics preserve the stable NavopRdpResult together
 * with an optional raw signed HRESULT, a numeric creation stage, and an
 * optional raw Win32 error code. No native text or connection secrets cross
 * this ABI. has_hresult and has_win32_code are exactly 0 or 1, and reserved is
 * always zero.
 *
 * The first NAVOP_RDP_LAST_ERROR_LEGACY_SIZE bytes are the stable legacy
 * prefix. New implementations accept that prefix and write extension fields
 * only when the caller-provided struct_size includes them.
 */
typedef struct NavopRdpLastError {
    uint32_t struct_size;
    uint32_t abi_version;
    int32_t result;
    int32_t hresult;
    uint32_t has_hresult;
    uint32_t reserved;
    uint32_t stage;
    uint32_t win32_code;
    uint32_t has_win32_code;
} NavopRdpLastError;

typedef struct NavopRdpCreateOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t generation_low;
    uint32_t generation_high;
} NavopRdpCreateOptions;

/*
 * host is borrowed UTF-16 data for navop_rdp_connect only. It is not
 * NUL-terminated, so len is authoritative. The native implementation scans
 * exactly len code units, copies the endpoint into a temporary COM string, and
 * does not retain data after the call returns.
 */
typedef struct NavopRdpBorrowedUtf16 {
    const uint16_t* data;
    uint32_t len;
} NavopRdpBorrowedUtf16;

/*
 * Connection flags are a versioned bitmask. When the audio playback disabled
 * bit is clear, remote audio is redirected to the local computer. When it is
 * set, remote audio playback is disabled. Any unknown connection flag causes
 * navop_rdp_connect to return NAVOP_RDP_RESULT_INVALID_ARGUMENT.
 */
#define NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED UINT32_C(1)
#define NAVOP_RDP_CONNECTION_FLAGS_KNOWN \
    NAVOP_RDP_CONNECTION_FLAG_AUDIO_PLAYBACK_DISABLED

/*
 * Append-only connection policy constants. Numeric enum values are part of
 * the C ABI and must remain synchronized with the Rust ffi module.
 */
#define NAVOP_RDP_DISPLAY_MODE_DYNAMIC UINT32_C(0)
#define NAVOP_RDP_DISPLAY_MODE_FIXED UINT32_C(1)
#define NAVOP_RDP_DISPLAY_FLAG_SMART_SIZING (UINT32_C(1) << 0)
#define NAVOP_RDP_DISPLAY_FLAG_USE_MULTIMON (UINT32_C(1) << 1)
#define NAVOP_RDP_DISPLAY_FLAG_SPAN_MONITORS (UINT32_C(1) << 2)
#define NAVOP_RDP_DISPLAY_FLAGS_KNOWN \
    (NAVOP_RDP_DISPLAY_FLAG_SMART_SIZING | \
     NAVOP_RDP_DISPLAY_FLAG_USE_MULTIMON | \
     NAVOP_RDP_DISPLAY_FLAG_SPAN_MONITORS)

#define NAVOP_RDP_RESOURCE_FLAG_CLIPBOARD (UINT32_C(1) << 0)
#define NAVOP_RDP_RESOURCE_FLAG_DRIVES (UINT32_C(1) << 1)
#define NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DRIVES (UINT32_C(1) << 2)
#define NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DEVICES (UINT32_C(1) << 3)
#define NAVOP_RDP_RESOURCE_FLAG_PRINTERS (UINT32_C(1) << 4)
#define NAVOP_RDP_RESOURCE_FLAG_SERIAL_PORTS (UINT32_C(1) << 5)
#define NAVOP_RDP_RESOURCE_FLAG_SMART_CARDS (UINT32_C(1) << 6)
#define NAVOP_RDP_RESOURCE_FLAG_CAMERAS (UINT32_C(1) << 7)
#define NAVOP_RDP_RESOURCE_FLAG_MICROPHONES (UINT32_C(1) << 8)
#define NAVOP_RDP_RESOURCE_FLAG_POS_DEVICES (UINT32_C(1) << 9)
#define NAVOP_RDP_RESOURCE_FLAGS_KNOWN \
    (NAVOP_RDP_RESOURCE_FLAG_CLIPBOARD | \
     NAVOP_RDP_RESOURCE_FLAG_DRIVES | \
     NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DRIVES | \
     NAVOP_RDP_RESOURCE_FLAG_DYNAMIC_DEVICES | \
     NAVOP_RDP_RESOURCE_FLAG_PRINTERS | \
     NAVOP_RDP_RESOURCE_FLAG_SERIAL_PORTS | \
     NAVOP_RDP_RESOURCE_FLAG_SMART_CARDS | \
     NAVOP_RDP_RESOURCE_FLAG_CAMERAS | \
     NAVOP_RDP_RESOURCE_FLAG_MICROPHONES | \
     NAVOP_RDP_RESOURCE_FLAG_POS_DEVICES)

#define NAVOP_RDP_AUDIO_MODE_LOCAL UINT32_C(0)
#define NAVOP_RDP_AUDIO_MODE_REMOTE UINT32_C(1)
#define NAVOP_RDP_AUDIO_MODE_DISABLED UINT32_C(2)
#define NAVOP_RDP_AUDIO_QUALITY_DYNAMIC UINT32_C(0)
#define NAVOP_RDP_AUDIO_QUALITY_MEDIUM UINT32_C(1)
#define NAVOP_RDP_AUDIO_QUALITY_HIGH UINT32_C(2)
#define NAVOP_RDP_AUDIO_FLAG_CAPTURE (UINT32_C(1) << 0)
#define NAVOP_RDP_AUDIO_FLAGS_KNOWN NAVOP_RDP_AUDIO_FLAG_CAPTURE

#define NAVOP_RDP_KEYBOARD_HOOK_LOCAL UINT32_C(0)
#define NAVOP_RDP_KEYBOARD_HOOK_REMOTE UINT32_C(1)
#define NAVOP_RDP_KEYBOARD_HOOK_FULLSCREEN UINT32_C(2)
#define NAVOP_RDP_INPUT_FLAG_ENABLE_WINDOWS_KEY (UINT32_C(1) << 0)
#define NAVOP_RDP_INPUT_FLAG_GRAB_FOCUS_ON_CONNECT (UINT32_C(1) << 1)
#define NAVOP_RDP_INPUT_FLAGS_KNOWN \
    (NAVOP_RDP_INPUT_FLAG_ENABLE_WINDOWS_KEY | \
     NAVOP_RDP_INPUT_FLAG_GRAB_FOCUS_ON_CONNECT)

#define NAVOP_RDP_PERFORMANCE_PRESET_AUTO UINT32_C(0)
#define NAVOP_RDP_PERFORMANCE_PRESET_MODEM UINT32_C(1)
#define NAVOP_RDP_PERFORMANCE_PRESET_BROADBAND_LOW UINT32_C(2)
#define NAVOP_RDP_PERFORMANCE_PRESET_SATELLITE UINT32_C(3)
#define NAVOP_RDP_PERFORMANCE_PRESET_LAN UINT32_C(4)
#define NAVOP_RDP_PERFORMANCE_FLAG_WALLPAPER (UINT32_C(1) << 0)
#define NAVOP_RDP_PERFORMANCE_FLAG_FULL_WINDOW_DRAG (UINT32_C(1) << 1)
#define NAVOP_RDP_PERFORMANCE_FLAG_MENU_ANIMATIONS (UINT32_C(1) << 2)
#define NAVOP_RDP_PERFORMANCE_FLAG_THEMES (UINT32_C(1) << 3)
#define NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SHADOW (UINT32_C(1) << 4)
#define NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SETTINGS (UINT32_C(1) << 5)
#define NAVOP_RDP_PERFORMANCE_FLAG_FONT_SMOOTHING (UINT32_C(1) << 6)
#define NAVOP_RDP_PERFORMANCE_FLAG_DESKTOP_COMPOSITION (UINT32_C(1) << 7)
#define NAVOP_RDP_PERFORMANCE_FLAG_BITMAP_CACHE (UINT32_C(1) << 8)
#define NAVOP_RDP_PERFORMANCE_FLAGS_KNOWN \
    (NAVOP_RDP_PERFORMANCE_FLAG_WALLPAPER | \
     NAVOP_RDP_PERFORMANCE_FLAG_FULL_WINDOW_DRAG | \
     NAVOP_RDP_PERFORMANCE_FLAG_MENU_ANIMATIONS | \
     NAVOP_RDP_PERFORMANCE_FLAG_THEMES | \
     NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SHADOW | \
     NAVOP_RDP_PERFORMANCE_FLAG_CURSOR_SETTINGS | \
     NAVOP_RDP_PERFORMANCE_FLAG_FONT_SMOOTHING | \
     NAVOP_RDP_PERFORMANCE_FLAG_DESKTOP_COMPOSITION | \
     NAVOP_RDP_PERFORMANCE_FLAG_BITMAP_CACHE)

#define NAVOP_RDP_NETWORK_CONNECTION_MODEM UINT32_C(0)
#define NAVOP_RDP_NETWORK_CONNECTION_BROADBAND_LOW UINT32_C(1)
#define NAVOP_RDP_NETWORK_CONNECTION_SATELLITE UINT32_C(2)
#define NAVOP_RDP_NETWORK_CONNECTION_BROADBAND_HIGH UINT32_C(3)
#define NAVOP_RDP_NETWORK_CONNECTION_WAN UINT32_C(4)
#define NAVOP_RDP_NETWORK_CONNECTION_LAN UINT32_C(5)
#define NAVOP_RDP_NETWORK_CONNECTION_AUTODETECT UINT32_C(6)

#define NAVOP_RDP_SECURITY_FLAG_ENABLE_CREDSSP (UINT32_C(1) << 0)
#define NAVOP_RDP_SECURITY_FLAG_PUBLIC_MODE (UINT32_C(1) << 1)
#define NAVOP_RDP_SECURITY_FLAG_ENCRYPTION_ENABLED (UINT32_C(1) << 2)
#define NAVOP_RDP_SECURITY_FLAGS_KNOWN \
    (NAVOP_RDP_SECURITY_FLAG_ENABLE_CREDSSP | \
     NAVOP_RDP_SECURITY_FLAG_PUBLIC_MODE | \
     NAVOP_RDP_SECURITY_FLAG_ENCRYPTION_ENABLED)
#define NAVOP_RDP_AUTHENTICATION_LEVEL_CONNECT UINT32_C(0)
#define NAVOP_RDP_AUTHENTICATION_LEVEL_WARN UINT32_C(1)
#define NAVOP_RDP_AUTHENTICATION_LEVEL_REJECT UINT32_C(2)

#define NAVOP_RDP_GATEWAY_MODE_NONE UINT32_C(0)
#define NAVOP_RDP_GATEWAY_MODE_EXPLICIT UINT32_C(1)
#define NAVOP_RDP_GATEWAY_MODE_AUTO_DETECT UINT32_C(2)
#define NAVOP_RDP_GATEWAY_FLAG_BYPASS_LOCAL (UINT32_C(1) << 0)
#define NAVOP_RDP_GATEWAY_FLAGS_KNOWN NAVOP_RDP_GATEWAY_FLAG_BYPASS_LOCAL
#define NAVOP_RDP_GATEWAY_CREDENTIAL_PASSWORD UINT32_C(0)
#define NAVOP_RDP_GATEWAY_CREDENTIAL_SMART_CARD UINT32_C(1)
#define NAVOP_RDP_GATEWAY_CREDENTIAL_ANY UINT32_C(4)
#define NAVOP_RDP_MAX_GATEWAY_HOST_UTF16_CODE_UNITS UINT32_C(255)

#define NAVOP_RDP_CONNECTION_POLICY_FLAG_ADMIN_SESSION (UINT32_C(1) << 0)
#define NAVOP_RDP_CONNECTION_POLICY_FLAG_AUTO_RECONNECT (UINT32_C(1) << 1)
#define NAVOP_RDP_CONNECTION_POLICY_FLAGS_KNOWN \
    (NAVOP_RDP_CONNECTION_POLICY_FLAG_ADMIN_SESSION | \
     NAVOP_RDP_CONNECTION_POLICY_FLAG_AUTO_RECONNECT)
/*
 * Append-only reconnect policy bounds shared by the Rust facade and the native
 * validation. max_reconnect_attempts is capped at 200 so the retry dialog and
 * the MSTSC property stay within LONG range; keep_alive_seconds and
 * timeout_seconds are validated by each side against the LONG conversion they
 * feed (milliseconds for the keep-alive interval, seconds for the overall
 * connection timeout).
 */
#define NAVOP_RDP_MAX_RECONNECT_ATTEMPTS UINT32_C(200)

/*
 * The first NAVOP_RDP_CONNECTION_LEGACY_SIZE bytes are the stable legacy
 * prefix. legacy_reserved occupies historical tail padding and is ignored.
 * Every following field is append-only. Implementations must read only fields
 * fully covered by struct_size and use the documented current defaults for
 * missing fields. Unknown trailing bytes are ignored.
 */
typedef struct NavopRdpConnectionOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    NavopRdpBorrowedUtf16 host;
    uint32_t port;
    int32_t desktop_width;
    int32_t desktop_height;
    int32_t color_depth;
    uint32_t flags;
    uint32_t legacy_reserved;
    uint32_t display_mode;
    uint32_t display_flags;
    uint32_t desktop_scale_factor;
    uint32_t device_scale_factor;
    uint32_t resource_flags;
    uint32_t audio_mode;
    uint32_t audio_quality;
    uint32_t audio_flags;
    uint32_t keyboard_hook_mode;
    uint32_t input_flags;
    uint32_t performance_preset;
    uint32_t performance_flags;
    uint32_t network_connection_type;
    uint32_t security_flags;
    uint32_t authentication_level;
    uint32_t gateway_mode;
    uint32_t gateway_flags;
    uint32_t gateway_credential_source;
    NavopRdpBorrowedUtf16 gateway_hostname;
    uint32_t keep_alive_seconds;
    uint32_t timeout_seconds;
    uint32_t connection_flags;
    uint32_t max_reconnect_attempts;
} NavopRdpConnectionOptions;

/*
 * parent_hwnd is a caller-owned, non-null, non-owning native window handle
 * passed as a pointer-sized integer. The host creates and owns only its hidden
 * child window. The caller must keep the parent window valid on the host
 * owner/UI thread until the host has been successfully destroyed;
 * navop_rdp_destroy never destroys or otherwise takes ownership of the parent
 * window.
 */
typedef struct NavopRdpCreateWithParentOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t generation_low;
    uint32_t generation_high;
    uintptr_t parent_hwnd;
} NavopRdpCreateWithParentOptions;

/*
 * Bounds are expressed in the parent window's client-area physical pixels.
 * x and y may be negative; width and height must be non-negative. Zero-sized
 * bounds are valid and keep the child from presenting content.
 */
typedef struct NavopRdpBounds {
    int32_t x;
    int32_t y;
    int32_t width;
    int32_t height;
} NavopRdpBounds;

/*
 * Session display settings are the post-login RDP framebuffer dimensions and
 * scale factors passed to IMsRdpClient9::UpdateSessionDisplaySettings. They are
 * distinct from NavopRdpBounds, which only positions the native child window.
 * Width, height, and scale values must be non-zero. orientation is forwarded
 * verbatim; callers currently use zero for landscape.
 */
typedef struct NavopRdpSessionDisplaySettings {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t desktop_width;
    uint32_t desktop_height;
    uint32_t physical_width;
    uint32_t physical_height;
    uint32_t orientation;
    uint32_t desktop_scale_factor;
    uint32_t device_scale_factor;
} NavopRdpSessionDisplaySettings;

/*
 * Synchronous presentation-readiness snapshot for the native child window.
 * Every flag is exactly 0 or 1. The query never fails for a live host: when the
 * ActiveX resources are unavailable every flag is reported as 0. Callers
 * combine this snapshot with their own connection phase (LoginComplete /
 * Reconnected), non-zero logical bounds, and owner-window visibility before
 * deciding that the native presentation is ready.
 */
typedef struct NavopRdpPresentationState {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t control_hwnd_valid;
    uint32_t host_rect_nonzero;
    uint32_t control_rect_nonzero;
    uint32_t control_visible;
    uint32_t control_is_host_descendant;
    uint32_t host_visible;
} NavopRdpPresentationState;

#define NAVOP_RDP_EVENT_CONNECTING UINT32_C(1)
#define NAVOP_RDP_EVENT_CONNECTED UINT32_C(2)
#define NAVOP_RDP_EVENT_LOGIN_COMPLETE UINT32_C(3)
#define NAVOP_RDP_EVENT_RECONNECTING UINT32_C(4)
#define NAVOP_RDP_EVENT_RECONNECTED UINT32_C(5)
#define NAVOP_RDP_EVENT_NETWORK_STATUS_CHANGED UINT32_C(6)
#define NAVOP_RDP_EVENT_REMOTE_DESKTOP_SIZE_CHANGED UINT32_C(7)
#define NAVOP_RDP_EVENT_ENTER_FULLSCREEN UINT32_C(8)
#define NAVOP_RDP_EVENT_LEAVE_FULLSCREEN UINT32_C(9)
#define NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISPLAYED UINT32_C(10)
#define NAVOP_RDP_EVENT_AUTHENTICATION_WARNING_DISMISSED UINT32_C(11)
#define NAVOP_RDP_EVENT_WARNING UINT32_C(12)
#define NAVOP_RDP_EVENT_FATAL_ERROR UINT32_C(13)
#define NAVOP_RDP_EVENT_LOGON_ERROR UINT32_C(14)
#define NAVOP_RDP_EVENT_DISCONNECTED UINT32_C(15)
#define NAVOP_RDP_EVENT_CLOSE_CONFIRMED UINT32_C(16)
#define NAVOP_RDP_EVENT_FOCUS_RELEASED UINT32_C(17)
#define NAVOP_RDP_MAX_EVENT_PAYLOAD_BYTES UINT32_C(65536)

/*
 * Event payloads are an architecture-independent byte protocol, not copied C
 * structs. Every integer is little-endian and every known event accepts only
 * the exact payload forms listed below:
 *
 * - Connecting, Connected, LoginComplete, Reconnected, enter/leave fullscreen,
 *   authentication warning displayed/dismissed, CloseConfirmed, FocusReleased:
 *   code == 0 and payload_len == 0.
 * - Reconnecting: code == 0 and payload is attempt:u32 followed by an optional
 *   max_attempts:u32 (payload_len is 4 or 8).
 * - NetworkStatusChanged: code == 0 and payload is an optional quality:u32
 *   (payload_len is 0 or 4).
 * - RemoteDesktopSizeChanged: code == 0 and payload is width:u32, height:u32
 *   (payload_len is 8).
 * - Warning, FatalError, LogonError: code carries the raw native code and
 *   payload_len == 0.
 * - Disconnected: code carries the raw signed 32-bit disconnect code and
 *   payload is an optional extended_code:i32 (payload_len is 0 or 4).
 *
 * Within the same ABI version, unknown kinds and malformed known payloads must
 * be preserved as opaque raw events by consumers. Existing kind values and
 * payload schemas are immutable; additions or schema changes must use new kind
 * values. payload_len must not exceed NAVOP_RDP_MAX_EVENT_PAYLOAD_BYTES.
 */
typedef struct NavopRdpEvent {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t kind;
    uint32_t reserved;
    uint32_t generation_low;
    uint32_t generation_high;
    int32_t code;
    uint32_t payload_len;
} NavopRdpEvent;

typedef struct NavopRdpEventCallbackOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t generation_low;
    uint32_t generation_high;
} NavopRdpEventCallbackOptions;

typedef struct NavopRdpBorrowedSecret {
    const uint16_t* data;
    uint32_t len;
} NavopRdpBorrowedSecret;

typedef struct NavopRdpCredentialBundle {
    uint32_t struct_size;
    uint32_t abi_version;
    NavopRdpBorrowedSecret server_password;
    NavopRdpBorrowedSecret gateway_password;
    uint32_t flags;
    NavopRdpBorrowedUtf16 username;
    NavopRdpBorrowedUtf16 domain;
    NavopRdpBorrowedUtf16 gateway_username;
    NavopRdpBorrowedUtf16 gateway_domain;
} NavopRdpCredentialBundle;

/*
 * The callback payload is borrowed only for the duration of the callback.
 * Consumers must copy payload_len bytes before returning and must not free the
 * payload pointer.
 *
 * NativeRdpHost entrypoints and callbacks are owner thread/thread-affine and
 * must be serialized by the caller.
 * Callbacks must not synchronously call NativeRdpHost entrypoints; they may
 * only copy or queue data and schedule lifecycle work for a later owner-thread
 * turn.
 * Wrong-thread calls fail without changing the host, callback registration,
 * callback context, or caller-owned handle.
 * A failed callback registration does not retain callback or callback_context.
 * Successful unregistration guarantees no callback is in flight and neither
 * callback pointer is retained, so callback_context may be released afterward.
 *
 * If callback unregistration or destroy is called while a callback is in flight,
 * the operation fails without blocking and must preserve the callback,
 * callback_context, host, and caller-owned handle. The caller may retry from a
 * later owner-thread turn after the callback returns.
 */
/*
 * Identity and credential code units are borrowed only for the synchronous call.
 * A zero length accepts a null pointer; a non-zero length requires a non-null
 * pointer. The native host must not retain caller-owned pointers after apply
 * returns. Server and Gateway passwords remain separate fields and flags must
 * be zero.
 *
 * The first NAVOP_RDP_CREDENTIAL_LEGACY_SIZE bytes are the stable legacy
 * password-only prefix. username, domain, gateway_username, and gateway_domain
 * are append-only fields; callers using the legacy prefix are accepted and
 * treated as supplying no identity.
 */
#ifdef __cplusplus
extern "C" {
#endif

typedef void (*NavopRdpEventCallback)(
    void* context,
    const NavopRdpEvent* event,
    const uint8_t* payload);

#ifdef __cplusplus
}
#endif

#ifdef __cplusplus
static_assert(sizeof(NavopRdpResult) == 4);
static_assert(sizeof(NavopRdpProbeOptions) == 8);
static_assert(alignof(NavopRdpProbeOptions) == 4);
static_assert(offsetof(NavopRdpProbeOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpProbeOptions, abi_version) == 4);
static_assert(sizeof(NavopRdpProbeResult) == 16);
static_assert(alignof(NavopRdpProbeResult) == 4);
static_assert(offsetof(NavopRdpProbeResult, struct_size) == 0);
static_assert(offsetof(NavopRdpProbeResult, abi_version) == 4);
static_assert(offsetof(NavopRdpProbeResult, available) == 8);
static_assert(offsetof(NavopRdpProbeResult, reserved) == 12);
static_assert(sizeof(NavopRdpLastError) == 36);
static_assert(alignof(NavopRdpLastError) == 4);
static_assert(offsetof(NavopRdpLastError, struct_size) == 0);
static_assert(offsetof(NavopRdpLastError, abi_version) == 4);
static_assert(offsetof(NavopRdpLastError, result) == 8);
static_assert(offsetof(NavopRdpLastError, hresult) == 12);
static_assert(offsetof(NavopRdpLastError, has_hresult) == 16);
static_assert(offsetof(NavopRdpLastError, reserved) == 20);
static_assert(offsetof(NavopRdpLastError, stage) == 24);
static_assert(offsetof(NavopRdpLastError, win32_code) == 28);
static_assert(offsetof(NavopRdpLastError, has_win32_code) == 32);
static_assert(
    offsetof(NavopRdpLastError, stage) ==
    NAVOP_RDP_LAST_ERROR_LEGACY_SIZE);
static_assert(sizeof(NavopRdpCreateOptions) == 16);
static_assert(alignof(NavopRdpCreateOptions) == 4);
static_assert(offsetof(NavopRdpCreateOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpCreateOptions, abi_version) == 4);
static_assert(offsetof(NavopRdpCreateOptions, generation_low) == 8);
static_assert(offsetof(NavopRdpCreateOptions, generation_high) == 12);
static_assert(offsetof(NavopRdpBorrowedUtf16, data) == 0);
static_assert(offsetof(NavopRdpConnectionOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpConnectionOptions, abi_version) == 4);
static_assert(offsetof(NavopRdpConnectionOptions, host) == 8);
#if INTPTR_MAX == INT64_MAX
static_assert(sizeof(NavopRdpBorrowedUtf16) == 16);
static_assert(alignof(NavopRdpBorrowedUtf16) == 8);
static_assert(offsetof(NavopRdpBorrowedUtf16, len) == 8);
static_assert(sizeof(NavopRdpConnectionOptions) == 152);
static_assert(alignof(NavopRdpConnectionOptions) == 8);
static_assert(offsetof(NavopRdpConnectionOptions, port) == 24);
static_assert(offsetof(NavopRdpConnectionOptions, desktop_width) == 28);
static_assert(offsetof(NavopRdpConnectionOptions, desktop_height) == 32);
static_assert(offsetof(NavopRdpConnectionOptions, color_depth) == 36);
static_assert(offsetof(NavopRdpConnectionOptions, flags) == 40);
static_assert(offsetof(NavopRdpConnectionOptions, legacy_reserved) == 44);
static_assert(offsetof(NavopRdpConnectionOptions, display_mode) == 48);
static_assert(offsetof(NavopRdpConnectionOptions, display_flags) == 52);
static_assert(
    offsetof(NavopRdpConnectionOptions, desktop_scale_factor) == 56);
static_assert(
    offsetof(NavopRdpConnectionOptions, device_scale_factor) == 60);
static_assert(offsetof(NavopRdpConnectionOptions, resource_flags) == 64);
static_assert(offsetof(NavopRdpConnectionOptions, audio_mode) == 68);
static_assert(offsetof(NavopRdpConnectionOptions, audio_quality) == 72);
static_assert(offsetof(NavopRdpConnectionOptions, audio_flags) == 76);
static_assert(offsetof(NavopRdpConnectionOptions, keyboard_hook_mode) == 80);
static_assert(offsetof(NavopRdpConnectionOptions, input_flags) == 84);
static_assert(
    offsetof(NavopRdpConnectionOptions, performance_preset) == 88);
static_assert(
    offsetof(NavopRdpConnectionOptions, performance_flags) == 92);
static_assert(
    offsetof(NavopRdpConnectionOptions, network_connection_type) == 96);
static_assert(offsetof(NavopRdpConnectionOptions, security_flags) == 100);
static_assert(
    offsetof(NavopRdpConnectionOptions, authentication_level) == 104);
static_assert(offsetof(NavopRdpConnectionOptions, gateway_mode) == 108);
static_assert(offsetof(NavopRdpConnectionOptions, gateway_flags) == 112);
static_assert(
    offsetof(NavopRdpConnectionOptions, gateway_credential_source) == 116);
static_assert(
    offsetof(NavopRdpConnectionOptions, gateway_hostname) == 120);
static_assert(
    offsetof(NavopRdpConnectionOptions, keep_alive_seconds) == 136);
static_assert(offsetof(NavopRdpConnectionOptions, timeout_seconds) == 140);
static_assert(offsetof(NavopRdpConnectionOptions, connection_flags) == 144);
static_assert(
    offsetof(NavopRdpConnectionOptions, max_reconnect_attempts) == 148);
static_assert(
    offsetof(NavopRdpConnectionOptions, display_mode) ==
    NAVOP_RDP_CONNECTION_LEGACY_SIZE);
#elif INTPTR_MAX == INT32_MAX
static_assert(sizeof(NavopRdpBorrowedUtf16) == 8);
static_assert(alignof(NavopRdpBorrowedUtf16) == 4);
static_assert(offsetof(NavopRdpBorrowedUtf16, len) == 4);
static_assert(sizeof(NavopRdpConnectionOptions) == 136);
static_assert(alignof(NavopRdpConnectionOptions) == 4);
static_assert(offsetof(NavopRdpConnectionOptions, port) == 16);
static_assert(offsetof(NavopRdpConnectionOptions, desktop_width) == 20);
static_assert(offsetof(NavopRdpConnectionOptions, desktop_height) == 24);
static_assert(offsetof(NavopRdpConnectionOptions, color_depth) == 28);
static_assert(offsetof(NavopRdpConnectionOptions, flags) == 32);
static_assert(offsetof(NavopRdpConnectionOptions, legacy_reserved) == 36);
static_assert(offsetof(NavopRdpConnectionOptions, display_mode) == 40);
static_assert(offsetof(NavopRdpConnectionOptions, display_flags) == 44);
static_assert(
    offsetof(NavopRdpConnectionOptions, desktop_scale_factor) == 48);
static_assert(
    offsetof(NavopRdpConnectionOptions, device_scale_factor) == 52);
static_assert(offsetof(NavopRdpConnectionOptions, resource_flags) == 56);
static_assert(offsetof(NavopRdpConnectionOptions, audio_mode) == 60);
static_assert(offsetof(NavopRdpConnectionOptions, audio_quality) == 64);
static_assert(offsetof(NavopRdpConnectionOptions, audio_flags) == 68);
static_assert(offsetof(NavopRdpConnectionOptions, keyboard_hook_mode) == 72);
static_assert(offsetof(NavopRdpConnectionOptions, input_flags) == 76);
static_assert(
    offsetof(NavopRdpConnectionOptions, performance_preset) == 80);
static_assert(
    offsetof(NavopRdpConnectionOptions, performance_flags) == 84);
static_assert(
    offsetof(NavopRdpConnectionOptions, network_connection_type) == 88);
static_assert(offsetof(NavopRdpConnectionOptions, security_flags) == 92);
static_assert(
    offsetof(NavopRdpConnectionOptions, authentication_level) == 96);
static_assert(offsetof(NavopRdpConnectionOptions, gateway_mode) == 100);
static_assert(offsetof(NavopRdpConnectionOptions, gateway_flags) == 104);
static_assert(
    offsetof(NavopRdpConnectionOptions, gateway_credential_source) == 108);
static_assert(
    offsetof(NavopRdpConnectionOptions, gateway_hostname) == 112);
static_assert(
    offsetof(NavopRdpConnectionOptions, keep_alive_seconds) == 120);
static_assert(offsetof(NavopRdpConnectionOptions, timeout_seconds) == 124);
static_assert(offsetof(NavopRdpConnectionOptions, connection_flags) == 128);
static_assert(
    offsetof(NavopRdpConnectionOptions, max_reconnect_attempts) == 132);
#else
#error Unsupported pointer width for the Windows RDP connection ABI
#endif
static_assert(sizeof(NavopRdpCreateWithParentOptions) >= 20);
static_assert(alignof(NavopRdpCreateWithParentOptions) == alignof(uintptr_t));
static_assert(offsetof(NavopRdpCreateWithParentOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpCreateWithParentOptions, abi_version) == 4);
static_assert(offsetof(NavopRdpCreateWithParentOptions, generation_low) == 8);
static_assert(offsetof(NavopRdpCreateWithParentOptions, generation_high) == 12);
static_assert(offsetof(NavopRdpCreateWithParentOptions, parent_hwnd) == 16);
#if INTPTR_MAX == INT64_MAX
static_assert(sizeof(NavopRdpCreateWithParentOptions) == 24);
static_assert(alignof(NavopRdpCreateWithParentOptions) == 8);
#elif INTPTR_MAX == INT32_MAX
static_assert(sizeof(NavopRdpCreateWithParentOptions) == 20);
static_assert(alignof(NavopRdpCreateWithParentOptions) == 4);
#else
#error Unsupported pointer width for the Windows RDP create-with-parent ABI
#endif
static_assert(sizeof(NavopRdpBounds) == 16);
static_assert(alignof(NavopRdpBounds) == 4);
static_assert(offsetof(NavopRdpBounds, x) == 0);
static_assert(offsetof(NavopRdpBounds, y) == 4);
static_assert(offsetof(NavopRdpBounds, width) == 8);
static_assert(offsetof(NavopRdpBounds, height) == 12);
static_assert(sizeof(NavopRdpSessionDisplaySettings) == 36);
static_assert(alignof(NavopRdpSessionDisplaySettings) == 4);
static_assert(offsetof(NavopRdpSessionDisplaySettings, struct_size) == 0);
static_assert(offsetof(NavopRdpSessionDisplaySettings, abi_version) == 4);
static_assert(offsetof(NavopRdpSessionDisplaySettings, desktop_width) == 8);
static_assert(offsetof(NavopRdpSessionDisplaySettings, desktop_height) == 12);
static_assert(offsetof(NavopRdpSessionDisplaySettings, physical_width) == 16);
static_assert(offsetof(NavopRdpSessionDisplaySettings, physical_height) == 20);
static_assert(offsetof(NavopRdpSessionDisplaySettings, orientation) == 24);
static_assert(
    offsetof(NavopRdpSessionDisplaySettings, desktop_scale_factor) == 28);
static_assert(
    offsetof(NavopRdpSessionDisplaySettings, device_scale_factor) == 32);
static_assert(sizeof(NavopRdpPresentationState) == 32);
static_assert(alignof(NavopRdpPresentationState) == 4);
static_assert(offsetof(NavopRdpPresentationState, struct_size) == 0);
static_assert(offsetof(NavopRdpPresentationState, abi_version) == 4);
static_assert(offsetof(NavopRdpPresentationState, control_hwnd_valid) == 8);
static_assert(offsetof(NavopRdpPresentationState, host_rect_nonzero) == 12);
static_assert(offsetof(NavopRdpPresentationState, control_rect_nonzero) == 16);
static_assert(offsetof(NavopRdpPresentationState, control_visible) == 20);
static_assert(
    offsetof(NavopRdpPresentationState, control_is_host_descendant) == 24);
static_assert(offsetof(NavopRdpPresentationState, host_visible) == 28);
static_assert(sizeof(NavopRdpEvent) == 32);
static_assert(alignof(NavopRdpEvent) == 4);
static_assert(offsetof(NavopRdpEvent, struct_size) == 0);
static_assert(offsetof(NavopRdpEvent, abi_version) == 4);
static_assert(offsetof(NavopRdpEvent, kind) == 8);
static_assert(offsetof(NavopRdpEvent, reserved) == 12);
static_assert(offsetof(NavopRdpEvent, generation_low) == 16);
static_assert(offsetof(NavopRdpEvent, generation_high) == 20);
static_assert(offsetof(NavopRdpEvent, code) == 24);
static_assert(offsetof(NavopRdpEvent, payload_len) == 28);
static_assert(sizeof(NavopRdpEventCallbackOptions) == 16);
static_assert(alignof(NavopRdpEventCallbackOptions) == 4);
static_assert(offsetof(NavopRdpEventCallbackOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpEventCallbackOptions, abi_version) == 4);
static_assert(offsetof(NavopRdpEventCallbackOptions, generation_low) == 8);
static_assert(offsetof(NavopRdpEventCallbackOptions, generation_high) == 12);
static_assert(offsetof(NavopRdpBorrowedSecret, data) == 0);
static_assert(offsetof(NavopRdpCredentialBundle, struct_size) == 0);
static_assert(offsetof(NavopRdpCredentialBundle, abi_version) == 4);
static_assert(offsetof(NavopRdpCredentialBundle, server_password) == 8);

#if INTPTR_MAX == INT64_MAX
static_assert(sizeof(NavopRdpBorrowedSecret) == 16);
static_assert(alignof(NavopRdpBorrowedSecret) == 8);
static_assert(offsetof(NavopRdpBorrowedSecret, len) == 8);
static_assert(sizeof(NavopRdpCredentialBundle) == 112);
static_assert(alignof(NavopRdpCredentialBundle) == 8);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_password) == 24);
static_assert(offsetof(NavopRdpCredentialBundle, flags) == 40);
static_assert(offsetof(NavopRdpCredentialBundle, username) == 48);
static_assert(offsetof(NavopRdpCredentialBundle, domain) == 64);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_username) == 80);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_domain) == 96);
static_assert(
    offsetof(NavopRdpCredentialBundle, username) ==
    NAVOP_RDP_CREDENTIAL_LEGACY_SIZE);
#elif INTPTR_MAX == INT32_MAX
static_assert(sizeof(NavopRdpBorrowedSecret) == 8);
static_assert(alignof(NavopRdpBorrowedSecret) == 4);
static_assert(offsetof(NavopRdpBorrowedSecret, len) == 4);
static_assert(sizeof(NavopRdpCredentialBundle) == 60);
static_assert(alignof(NavopRdpCredentialBundle) == 4);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_password) == 16);
static_assert(offsetof(NavopRdpCredentialBundle, flags) == 24);
static_assert(offsetof(NavopRdpCredentialBundle, username) == 28);
static_assert(offsetof(NavopRdpCredentialBundle, domain) == 36);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_username) == 44);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_domain) == 52);
static_assert(
    offsetof(NavopRdpCredentialBundle, username) ==
    NAVOP_RDP_CREDENTIAL_LEGACY_SIZE);
#else
#error Unsupported pointer width for the Windows RDP credential ABI
#endif

#define NAVOP_RDP_NOEXCEPT noexcept
extern "C" {
#else
#define NAVOP_RDP_NOEXCEPT
#endif

NavopRdpResult navop_rdp_probe(
    const NavopRdpProbeOptions* options,
    NavopRdpProbeResult* out_result) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_create(
    const NavopRdpCreateOptions* options,
    NativeRdpHost** out_host) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_create_with_parent(
    const NavopRdpCreateWithParentOptions* options,
    NativeRdpHost** out_host) NAVOP_RDP_NOEXCEPT;

/*
 * Enhanced create entrypoint. out_error is initialized on every call whose
 * output layout is valid, including failures that occur before a host can be
 * returned. The legacy entrypoint remains available for ABI compatibility.
 */
NavopRdpResult navop_rdp_create_with_parent_v2(
    const NavopRdpCreateWithParentOptions* options,
    NativeRdpHost** out_host,
    NavopRdpLastError* out_error) NAVOP_RDP_NOEXCEPT;

/*
 * Returns the most recent owner-thread synchronous operation diagnostic.
 * Reading does not clear the slot. A successful operation leaves result == OK
 * and has_hresult == 0. Wrong-thread calls do not overwrite the slot.
 */
NavopRdpResult navop_rdp_get_last_error(
    NativeRdpHost* host,
    NavopRdpLastError* out_error) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_set_bounds(
    NativeRdpHost* host,
    const NavopRdpBounds* bounds) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_update_session_display_settings(
    NativeRdpHost* host,
    const NavopRdpSessionDisplaySettings* settings) NAVOP_RDP_NOEXCEPT;

/*
 * Returns a presentation-readiness snapshot for the native ActiveX child.
 * out_state is initialized on every call whose output layout is valid,
 * including when the native resources are unavailable (all flags are zero).
 */
NavopRdpResult navop_rdp_get_presentation_state(
    NativeRdpHost* host,
    NavopRdpPresentationState* out_state) NAVOP_RDP_NOEXCEPT;

/*
 * visible must be exactly 0 or 1. Showing uses non-activating Win32 semantics.
 * Hiding first attempts to return keyboard focus to the caller-owned parent when
 * focus is currently inside the ActiveX child subtree.
 */
NavopRdpResult navop_rdp_set_visible(
    NativeRdpHost* host,
    uint32_t visible) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_focus(
    NativeRdpHost* host) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_register_event_callback(
    NativeRdpHost* host,
    const NavopRdpEventCallbackOptions* options,
    NavopRdpEventCallback callback,
    void* callback_context) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_unregister_event_callback(
    NativeRdpHost* host) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_apply_credentials(
    NativeRdpHost* host,
    const NavopRdpCredentialBundle* credentials) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_connect(
    NativeRdpHost* host,
    const NavopRdpConnectionOptions* options) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_get_connection_state(
    NativeRdpHost* host,
    uint32_t* out_state) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_request_close(
    NativeRdpHost* host,
    uint32_t* out_status) NAVOP_RDP_NOEXCEPT;

NavopRdpResult navop_rdp_disconnect(
    NativeRdpHost* host) NAVOP_RDP_NOEXCEPT;

/*
 * For a non-null owned host, destroy may release the native object only after
 * clearing the caller's handle. Any non-OK return, or any return that leaves
 * the caller's handle non-null, retains ownership for the caller, must not
 * release the native object, and is safe to retry.
 */
NavopRdpResult navop_rdp_destroy(NativeRdpHost** host) NAVOP_RDP_NOEXCEPT;

#ifdef __cplusplus
}
#endif

#undef NAVOP_RDP_NOEXCEPT
