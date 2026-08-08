#include "host_internal.h"

#include <cstdint>

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

bool valid_color_depth(int32_t color_depth) noexcept {
    return color_depth == 8 ||
        color_depth == 15 ||
        color_depth == 16 ||
        color_depth == 24 ||
        color_depth == 32;
}

NavopRdpResult validate_connection_options(
    const NavopRdpConnectionOptions* options) noexcept {
    if (options == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    NavopRdpResult result = validate_struct_size(
        options->struct_size,
        static_cast<uint32_t>(sizeof(NavopRdpConnectionOptions)));
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    result = validate_abi_version(options->abi_version);
    if (result != NAVOP_RDP_RESULT_OK) {
        return result;
    }

    if (options->flags != 0 ||
        options->host.len == 0 ||
        options->host.len > NAVOP_RDP_MAX_HOST_UTF16_CODE_UNITS ||
        options->host.data == nullptr ||
        options->port == 0 ||
        options->port > UINT32_C(65535) ||
        options->desktop_width <= 0 ||
        options->desktop_height <= 0 ||
        !valid_color_depth(options->color_depth)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    for (uint32_t index = 0; index < options->host.len; ++index) {
        if (options->host.data[index] == 0) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
    }
    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

extern "C" NavopRdpResult navop_rdp_connect(
    NativeRdpHost* host,
    const NavopRdpConnectionOptions* options) noexcept {
    try {
        if (host == nullptr || options == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        if (host->callback_state != CallbackState::Open) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        result = validate_connection_options(options);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        return connect_active_x(host->active_x_resources, *options);
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}
