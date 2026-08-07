#include "windows_rdp_host.h"

#include <new>

struct NativeRdpHost {
    uint64_t generation;
};

namespace {

NavopRdpResult validate_struct_size(
    uint32_t struct_size,
    uint32_t required_size) noexcept {
    if (struct_size < required_size) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult validate_abi_version(uint32_t abi_version) noexcept {
    if (abi_version != NAVOP_RDP_ABI_VERSION) {
        return NAVOP_RDP_RESULT_ABI_MISMATCH;
    }
    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

extern "C" NavopRdpResult navop_rdp_probe(
    const NavopRdpProbeOptions* options,
    NavopRdpProbeResult* out_result) noexcept {
    try {
        if (options == nullptr || out_result == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        const uint32_t caller_result_size = out_result->struct_size;

        NavopRdpResult result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpProbeOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_struct_size(
            caller_result_size,
            static_cast<uint32_t>(sizeof(NavopRdpProbeResult)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(out_result->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        out_result->struct_size = caller_result_size;
        out_result->abi_version = NAVOP_RDP_ABI_VERSION;
        out_result->available = UINT32_C(1);
        out_result->reserved = UINT32_C(0);
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_create(
    const NavopRdpCreateOptions* options,
    NativeRdpHost** out_host) noexcept {
    try {
        if (out_host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        *out_host = nullptr;

        if (options == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = validate_struct_size(
            options->struct_size,
            static_cast<uint32_t>(sizeof(NavopRdpCreateOptions)));
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        result = validate_abi_version(options->abi_version);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }

        const uint64_t generation =
            static_cast<uint64_t>(options->generation_low) |
            (static_cast<uint64_t>(options->generation_high) << 32U);
        NativeRdpHost* host = new (std::nothrow) NativeRdpHost{generation};
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
        }

        *out_host = host;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_destroy(NativeRdpHost** host) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (*host == nullptr) {
            return NAVOP_RDP_RESULT_OK;
        }

        NativeRdpHost* owned = *host;
        *host = nullptr;
        delete owned;
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}
