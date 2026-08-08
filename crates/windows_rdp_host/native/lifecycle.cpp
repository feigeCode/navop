#include "host_internal.h"

extern "C" NavopRdpResult navop_rdp_get_connection_state(
    NativeRdpHost* host,
    uint32_t* out_state) noexcept {
    try {
        if (out_state == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        *out_state = UINT32_C(0);
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        if (host->callback_state != CallbackState::Open) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        return get_active_x_connection_state(host->active_x_resources, out_state);
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_request_close(
    NativeRdpHost* host,
    uint32_t* out_status) noexcept {
    try {
        if (out_status == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        *out_status = UINT32_C(0);
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        if (host->callback_state != CallbackState::Open) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        return request_close_active_x(host->active_x_resources, out_status);
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_disconnect(
    NativeRdpHost* host) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        if (host->callback_state != CallbackState::Open) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        return disconnect_active_x(host->active_x_resources);
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}
