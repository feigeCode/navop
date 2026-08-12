#pragma once

#include "windows_rdp_host.h"

struct IUnknown;

enum class CallbackState : uint32_t {
    Open,
    Closing,
    Closed,
};

struct NativeRdpHost;
struct NativeRdpActiveXResources;
struct NativeRdpEventSubscription;

void trace_native_stage(const char* stage) noexcept;

void trace_native_hresult(
    const char* stage,
    int32_t hresult) noexcept;

void trace_native_result(
    const char* stage,
    NavopRdpResult result) noexcept;

void trace_native_win32(
    const char* stage,
    uint32_t win32_code) noexcept;

void trace_native_pointer(
    const char* stage,
    uintptr_t pointer) noexcept;

NavopRdpResult create_active_x_resources(
    NativeRdpHost* owner,
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

NavopRdpResult connect_active_x(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    const NavopRdpConnectionOptions& options) noexcept;

NavopRdpResult apply_active_x_credentials(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    NavopRdpBorrowedUtf16 username,
    NavopRdpBorrowedUtf16 domain,
    NavopRdpBorrowedSecret server_password) noexcept;

NavopRdpResult get_active_x_connection_state(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    uint32_t* out_state) noexcept;

NavopRdpResult get_active_x_extended_disconnect_reason(
    NativeRdpActiveXResources* resources,
    int32_t* out_extended_code) noexcept;

NavopRdpResult request_close_active_x(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    uint32_t* out_status) noexcept;

NavopRdpResult disconnect_active_x(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources) noexcept;

NavopRdpResult create_event_subscription(
    NativeRdpHost* host,
    IUnknown* control,
    NativeRdpEventSubscription** out_subscription) noexcept;

void destroy_event_subscription(
    NativeRdpEventSubscription* subscription) noexcept;

struct NativeRdpHost {
    ~NativeRdpHost() noexcept;

    uint64_t generation;
    uint32_t owner_thread_id;
    uint32_t callbacks_in_flight;
    NavopRdpEventCallback callback;
    void* callback_context;
    CallbackState callback_state;
    NativeRdpActiveXResources* active_x_resources;
    NavopRdpResult last_result;
    int32_t last_hresult;
    uint32_t has_last_hresult;
    uint32_t last_stage;
    uint32_t last_win32_code;
    uint32_t has_last_win32_code;
};

void clear_last_error(NativeRdpHost* host) noexcept;

NavopRdpResult record_last_diagnostic(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    int32_t hresult,
    uint32_t has_hresult,
    uint32_t win32_code,
    uint32_t has_win32_code) noexcept;

NavopRdpResult record_last_error(
    NativeRdpHost* host,
    NavopRdpResult result) noexcept;

NavopRdpResult record_last_hresult(
    NativeRdpHost* host,
    NavopRdpResult result,
    int32_t hresult) noexcept;

NavopRdpResult record_last_stage_error(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage) noexcept;

NavopRdpResult record_last_stage_hresult(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    int32_t hresult) noexcept;

NavopRdpResult record_last_stage_win32(
    NativeRdpHost* host,
    NavopRdpResult result,
    uint32_t stage,
    uint32_t win32_code) noexcept;

NavopRdpResult ensure_owner_thread(
    const NativeRdpHost* host) noexcept;

NavopRdpResult close_callback_gate(
    NativeRdpHost* host) noexcept;

NavopRdpResult dispatch_event(
    NativeRdpHost* host,
    const NavopRdpEvent* event,
    const uint8_t* payload) noexcept;
