#include "host_internal.h"

extern "C" NavopRdpResult navop_rdp_get_connection_state(
    NativeRdpHost* host,
    uint32_t* out_state) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        clear_last_error(host);
        if (out_state == nullptr) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        *out_state = UINT32_C(0);
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        // Owner-aware equivalent of
        // get_active_x_connection_state(host->active_x_resources, out_state).
        return get_active_x_connection_state(host, host->active_x_resources, out_state);
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}

extern "C" NavopRdpResult navop_rdp_request_close(
    NativeRdpHost* host,
    uint32_t* out_status) noexcept {
    try {
        if (host == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        clear_last_error(host);
        if (out_status == nullptr) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        *out_status = UINT32_C(0);
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        // Owner-aware equivalent of
        // return request_close_active_x(host->active_x_resources, out_status);
        return request_close_active_x(host, host->active_x_resources, out_status);
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
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
        clear_last_error(host);
        if (host->callback_state != CallbackState::Open) {
            return record_last_error(host, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
        }
        // Owner-aware equivalent of
        // return disconnect_active_x(host->active_x_resources);
        return disconnect_active_x(host, host->active_x_resources);
    } catch (...) {
        return record_last_error(host, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
}
