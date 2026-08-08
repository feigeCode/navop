#pragma once

#include <stddef.h>
#include <stdint.h>

#define NAVOP_RDP_ABI_VERSION UINT32_C(1)
#define NAVOP_RDP_CREATE_WITH_PARENT_ABI_VERSION UINT32_C(1)

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

typedef struct NavopRdpCreateOptions {
    uint32_t struct_size;
    uint32_t abi_version;
    uint32_t generation_low;
    uint32_t generation_high;
} NavopRdpCreateOptions;

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
 * Credential code units are borrowed only for the synchronous call. A zero
 * length accepts a null pointer; a non-zero length requires a non-null pointer.
 * The native host must not retain either pointer after apply returns. Server
 * and Gateway passwords remain separate fields and flags must be zero.
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
static_assert(sizeof(NavopRdpCreateOptions) == 16);
static_assert(alignof(NavopRdpCreateOptions) == 4);
static_assert(offsetof(NavopRdpCreateOptions, struct_size) == 0);
static_assert(offsetof(NavopRdpCreateOptions, abi_version) == 4);
static_assert(offsetof(NavopRdpCreateOptions, generation_low) == 8);
static_assert(offsetof(NavopRdpCreateOptions, generation_high) == 12);
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
static_assert(sizeof(NavopRdpCredentialBundle) == 48);
static_assert(alignof(NavopRdpCredentialBundle) == 8);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_password) == 24);
static_assert(offsetof(NavopRdpCredentialBundle, flags) == 40);
#elif INTPTR_MAX == INT32_MAX
static_assert(sizeof(NavopRdpBorrowedSecret) == 8);
static_assert(alignof(NavopRdpBorrowedSecret) == 4);
static_assert(offsetof(NavopRdpBorrowedSecret, len) == 4);
static_assert(sizeof(NavopRdpCredentialBundle) == 28);
static_assert(alignof(NavopRdpCredentialBundle) == 4);
static_assert(offsetof(NavopRdpCredentialBundle, gateway_password) == 16);
static_assert(offsetof(NavopRdpCredentialBundle, flags) == 24);
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

NavopRdpResult navop_rdp_set_bounds(
    NativeRdpHost* host,
    const NavopRdpBounds* bounds) NAVOP_RDP_NOEXCEPT;

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
