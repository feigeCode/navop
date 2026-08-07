#pragma once

#include <stddef.h>
#include <stdint.h>

#define NAVOP_RDP_ABI_VERSION UINT32_C(1)

typedef struct NativeRdpHost NativeRdpHost;

typedef int32_t NavopRdpResult;

#define NAVOP_RDP_RESULT_OK INT32_C(0)
#define NAVOP_RDP_RESULT_INVALID_ARGUMENT INT32_C(1)
#define NAVOP_RDP_RESULT_ABI_MISMATCH INT32_C(2)
#define NAVOP_RDP_RESULT_ALLOCATION_FAILED INT32_C(3)
#define NAVOP_RDP_RESULT_INTERNAL_ERROR INT32_C(4)
#define NAVOP_RDP_RESULT_UNAVAILABLE INT32_C(5)

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

NavopRdpResult navop_rdp_destroy(NativeRdpHost** host) NAVOP_RDP_NOEXCEPT;

#ifdef __cplusplus
}
#endif

#undef NAVOP_RDP_NOEXCEPT
