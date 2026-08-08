#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>

#pragma warning(push)
#pragma warning(disable : 4192)
#import "libid:8C11EFA1-92C3-11D1-BC1E-00C04FA31489" \
    raw_interfaces_only, named_guids, no_namespace, exclude("UINT_PTR")
#pragma warning(pop)
#include "mstscax.tlh"

#include <memory>
#include <new>

namespace {

struct ActiveXCleanup {
    bool ole_initialized = false;
    bool atl_initialized = false;
    HWND parent_window = nullptr;
    HWND child_window = nullptr;
    CComPtr<IUnknown> container;
    CComPtr<IUnknown> control;
    CComPtr<IMsRdpClient10> client;
    NativeRdpEventSubscription* event_subscription = nullptr;

    ~ActiveXCleanup() noexcept {
        destroy_event_subscription(event_subscription);
        event_subscription = nullptr;
        if (child_window != nullptr) {
            DestroyWindow(child_window);
        }
        client.Release();
        control.Release();
        container.Release();
        if (atl_initialized) {
            AtlAxWinTerm();
        }
        if (ole_initialized) {
            OleUninitialize();
        }
    }
};

HRESULT create_rdp_control(ActiveXCleanup& resources) noexcept {
    // The GUID is fixed by the Windows RDP ActiveX registration. Keeping the
    // class name in a local std::wstring avoids retaining COM-allocated text.
    constexpr wchar_t class_name[] =
        L"CLSID:{945EE98E-B376-4EC2-B2E5-64C9410F93B7}";
    return AtlAxCreateControlEx(
        class_name,
        resources.child_window,
        nullptr,
        &resources.container,
        &resources.control,
        IID_NULL,
        nullptr);
}

bool window_or_descendant_has_focus(HWND window) noexcept {
    const HWND focused = GetFocus();
    return focused != nullptr &&
        (focused == window || IsChild(window, focused));
}

NavopRdpResult validate_resources(
    NativeRdpActiveXResources* resources) noexcept;

}  // namespace

struct NativeRdpActiveXResources {
    ActiveXCleanup state;
};

namespace {

NavopRdpResult validate_resources(
    NativeRdpActiveXResources* resources) noexcept {
    if (resources == nullptr ||
        resources->state.parent_window == nullptr ||
        resources->state.child_window == nullptr) {
        return NAVOP_RDP_RESULT_UNAVAILABLE;
    }
    if (!IsWindow(resources->state.parent_window) ||
        !IsWindow(resources->state.child_window)) {
        return NAVOP_RDP_RESULT_UNAVAILABLE;
    }
    if (resources->state.control == nullptr || resources->state.client == nullptr) {
        return NAVOP_RDP_RESULT_UNAVAILABLE;
    }
    return NAVOP_RDP_RESULT_OK;
}

}  // namespace

NavopRdpResult create_active_x_resources(
    NativeRdpHost* owner,
    uintptr_t parent_hwnd,
    NativeRdpActiveXResources** out_resources) noexcept {
    if (owner == nullptr || out_resources == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    *out_resources = nullptr;

    const HWND parent = reinterpret_cast<HWND>(parent_hwnd);
    if (parent == nullptr || !IsWindow(parent)) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    auto resources = std::unique_ptr<NativeRdpActiveXResources>(
        new (std::nothrow) NativeRdpActiveXResources());
    if (!resources) {
        return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
    }
    resources->state.parent_window = parent;

    const HRESULT ole_result = OleInitialize(nullptr);
    if (FAILED(ole_result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    resources->state.ole_initialized = true;

    if (!AtlAxWinInit()) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    resources->state.atl_initialized = true;

    resources->state.child_window = CreateWindowExW(
        0,
        L"AtlAxWin",
        L"",
        WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0,
        0,
        0,
        0,
        parent,
        nullptr,
        GetModuleHandleW(nullptr),
        nullptr);
    if (resources->state.child_window == nullptr) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    const HRESULT control_result = create_rdp_control(resources->state);
    if (FAILED(control_result) || resources->state.control == nullptr) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    const HRESULT query_result = resources->state.control->QueryInterface(
        IID_PPV_ARGS(&resources->state.client));
    if (FAILED(query_result) || resources->state.client == nullptr) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    CComPtr<IMsRdpClientNonScriptable2> non_scriptable;
    const HRESULT non_scriptable_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&non_scriptable));
    if (FAILED(non_scriptable_result) || non_scriptable == nullptr) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    const HRESULT ui_parent_result =
        non_scriptable->put_UIParentWindowHandle(
            reinterpret_cast<wireHWND>(parent));
    if (FAILED(ui_parent_result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    const NavopRdpResult subscription_result = create_event_subscription(
        owner,
        resources->state.control,
        &resources->state.event_subscription);
    if (subscription_result != NAVOP_RDP_RESULT_OK) {
        return subscription_result;
    }

    *out_resources = resources.release();
    return NAVOP_RDP_RESULT_OK;
}

void destroy_active_x_resources(
    NativeRdpActiveXResources* resources) noexcept {
    delete resources;
}

NavopRdpResult set_active_x_bounds(
    NativeRdpActiveXResources* resources,
    const NavopRdpBounds& bounds) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }
    if (bounds.width < 0 || bounds.height < 0) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    if (!SetWindowPos(
            resources->state.child_window,
            nullptr,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOZORDER | SWP_NOACTIVATE)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult set_active_x_visible(
    NativeRdpActiveXResources* resources,
    bool visible) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }

    if (!visible &&
        window_or_descendant_has_focus(resources->state.child_window)) {
        SetFocus(resources->state.parent_window);
    }
    ShowWindow(
        resources->state.child_window,
        visible ? SW_SHOWNA : SW_HIDE);

    const LONG_PTR style = GetWindowLongPtrW(
        resources->state.child_window,
        GWL_STYLE);
    const bool has_visible_style = (style & WS_VISIBLE) != 0;
    if (has_visible_style != visible) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult focus_active_x(
    NativeRdpActiveXResources* resources) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }
    if ((GetWindowLongPtrW(
             resources->state.child_window,
             GWL_STYLE) &
         WS_VISIBLE) == 0) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    SetFocus(resources->state.child_window);
    if (!window_or_descendant_has_focus(resources->state.child_window)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult connect_active_x(
    NativeRdpActiveXResources* resources,
    const NavopRdpConnectionOptions& options) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (connected != 0) {
        return NAVOP_RDP_RESULT_INVALID_STATE;
    }

    CComBSTR server(
        static_cast<int>(options.host.len),
        reinterpret_cast<LPCOLESTR>(options.host.data));
    if (server.m_str == nullptr) {
        return NAVOP_RDP_RESULT_ALLOCATION_FAILED;
    }

    result = resources->state.client->put_Server(server);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    CComPtr<IMsRdpClientAdvancedSettings> advanced_settings;
    result = resources->state.control->QueryInterface(
        IID_PPV_ARGS(&advanced_settings));
    if (FAILED(result) || advanced_settings == nullptr) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    result = advanced_settings->put_RDPPort(
        static_cast<LONG>(options.port));
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    result = resources->state.client->put_DesktopWidth(
        options.desktop_width);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    result = resources->state.client->put_DesktopHeight(
        options.desktop_height);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    result = resources->state.client->put_ColorDepth(options.color_depth);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    result = resources->state.client->Connect();
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult get_active_x_connection_state(
    NativeRdpActiveXResources* resources,
    uint32_t* out_state) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }
    if (out_state == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    short connected = 0;
    const HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (connected < 0 || connected > 2) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    *out_state = static_cast<uint32_t>(connected);
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult request_close_active_x(
    NativeRdpActiveXResources* resources,
    uint32_t* out_status) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }
    if (out_status == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (connected == 0) {
        *out_status = 0;
        return NAVOP_RDP_RESULT_OK;
    }
    if (connected != 1 && connected != 2) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    ControlCloseStatus status = controlCloseCanProceed;
    result = resources->state.client->RequestClose(&status);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (status != controlCloseCanProceed &&
        status != controlCloseWaitForEvents) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    *out_status = static_cast<uint32_t>(status);
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult disconnect_active_x(
    NativeRdpActiveXResources* resources) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (connected == 0) {
        return NAVOP_RDP_RESULT_OK;
    }
    if (connected != 1 && connected != 2) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }

    result = resources->state.client->Disconnect();
    return FAILED(result)
        ? NAVOP_RDP_RESULT_INTERNAL_ERROR
        : NAVOP_RDP_RESULT_OK;
}
