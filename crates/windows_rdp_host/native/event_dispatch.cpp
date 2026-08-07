#include "host_internal.h"

#include <windows.h>

namespace {

uint64_t join_generation(uint32_t low, uint32_t high) noexcept {
    return static_cast<uint64_t>(low) |
        (static_cast<uint64_t>(high) << 32U);
}

class CallbackDispatchScope {
public:
    explicit CallbackDispatchScope(NativeRdpHost* host) noexcept
        : host_(host) {
        host_->callbacks_in_flight += UINT32_C(1);
    }

    ~CallbackDispatchScope() noexcept {
        host_->callbacks_in_flight -= UINT32_C(1);
    }

    CallbackDispatchScope(const CallbackDispatchScope&) = delete;
    CallbackDispatchScope& operator=(const CallbackDispatchScope&) = delete;
    CallbackDispatchScope(CallbackDispatchScope&&) = delete;
    CallbackDispatchScope& operator=(CallbackDispatchScope&&) = delete;

private:
    NativeRdpHost* host_;
};

}  // namespace

NavopRdpResult ensure_owner_thread(
    const NativeRdpHost* host) noexcept {
    if (host == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    if (host->owner_thread_id !=
        static_cast<uint32_t>(GetCurrentThreadId())) {
        return NAVOP_RDP_RESULT_WRONG_THREAD;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult close_callback_gate(
    NativeRdpHost* host) noexcept {
    if (host == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    if (host->callbacks_in_flight != UINT32_C(0)) {
        return NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT;
    }
    if (host->callback_state == CallbackState::Closed) {
        return NAVOP_RDP_RESULT_OK;
    }

    host->callback_state = CallbackState::Closing;
    host->callback = nullptr;
    host->callback_context = nullptr;
    host->callback_state = CallbackState::Closed;
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult dispatch_event(
    NativeRdpHost* host,
    const NavopRdpEvent* event,
    const uint8_t* payload) noexcept {
    try {
        if (host == nullptr || event == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }

        NavopRdpResult result = ensure_owner_thread(host);
        if (result != NAVOP_RDP_RESULT_OK) {
            return result;
        }
        if (event->struct_size <
            static_cast<uint32_t>(sizeof(NavopRdpEvent))) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (event->abi_version != NAVOP_RDP_ABI_VERSION) {
            return NAVOP_RDP_RESULT_ABI_MISMATCH;
        }
        if (event->reserved != UINT32_C(0)) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (join_generation(
                event->generation_low,
                event->generation_high) != host->generation) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (event->payload_len != UINT32_C(0) && payload == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (host->callback_state != CallbackState::Open ||
            host->callback == nullptr) {
            return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
        }
        if (host->callbacks_in_flight == UINT32_MAX) {
            return NAVOP_RDP_RESULT_CALLBACK_IN_FLIGHT;
        }

        NavopRdpEventCallback callback = host->callback;
        void* callback_context = host->callback_context;
        CallbackDispatchScope callback_scope(host);
        callback(callback_context, event, payload);
        return NAVOP_RDP_RESULT_OK;
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}

extern "C" NavopRdpResult navop_rdp_test_dispatch_event(
    NativeRdpHost* host,
    const NavopRdpEvent* event,
    const uint8_t* payload) noexcept {
    try {
        return dispatch_event(host, event, payload);
    } catch (...) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
}
