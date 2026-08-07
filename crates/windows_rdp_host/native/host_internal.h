#pragma once

#include "windows_rdp_host.h"

enum class CallbackState : uint32_t {
    Open,
    Closing,
    Closed,
};

struct NativeRdpHost {
    uint64_t generation;
    uint32_t owner_thread_id;
    uint32_t callbacks_in_flight;
    NavopRdpEventCallback callback;
    void* callback_context;
    CallbackState callback_state;
};

NavopRdpResult ensure_owner_thread(
    const NativeRdpHost* host) noexcept;

NavopRdpResult close_callback_gate(
    NativeRdpHost* host) noexcept;

NavopRdpResult dispatch_event(
    NativeRdpHost* host,
    const NavopRdpEvent* event,
    const uint8_t* payload) noexcept;
