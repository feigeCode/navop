#pragma once

#include "windows_rdp_host.h"

enum class CallbackState : uint32_t {
    Open,
    Closing,
    Closed,
};

struct NativeRdpActiveXResources;

NavopRdpResult create_active_x_resources(
    uintptr_t parent_hwnd,
    NativeRdpActiveXResources** out_resources) noexcept;

void destroy_active_x_resources(
    NativeRdpActiveXResources* resources) noexcept;

NavopRdpResult set_active_x_bounds(
    NativeRdpActiveXResources* resources,
    const NavopRdpBounds& bounds) noexcept;

NavopRdpResult set_active_x_visible(
    NativeRdpActiveXResources* resources,
    bool visible) noexcept;

NavopRdpResult focus_active_x(
    NativeRdpActiveXResources* resources) noexcept;

struct NativeRdpHost {
    ~NativeRdpHost() noexcept;

    uint64_t generation;
    uint32_t owner_thread_id;
    uint32_t callbacks_in_flight;
    NavopRdpEventCallback callback;
    void* callback_context;
    CallbackState callback_state;
    NativeRdpActiveXResources* active_x_resources;
};

NavopRdpResult ensure_owner_thread(
    const NativeRdpHost* host) noexcept;

NavopRdpResult close_callback_gate(
    NativeRdpHost* host) noexcept;

NavopRdpResult dispatch_event(
    NativeRdpHost* host,
    const NavopRdpEvent* event,
    const uint8_t* payload) noexcept;
