#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>

#include "mstscax.tlh"

#include <limits>
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

class SensitiveBstr {
public:
    SensitiveBstr() noexcept = default;

    ~SensitiveBstr() noexcept {
        reset();
    }

    SensitiveBstr(const SensitiveBstr&) = delete;
    SensitiveBstr& operator=(const SensitiveBstr&) = delete;
    SensitiveBstr(SensitiveBstr&&) = delete;
    SensitiveBstr& operator=(SensitiveBstr&&) = delete;

    NavopRdpResult assign(NavopRdpBorrowedSecret secret) noexcept {
        reset();
        if (secret.len == UINT32_C(0)) {
            return NAVOP_RDP_RESULT_OK;
        }
        value_ = SysAllocStringLen(
            reinterpret_cast<const OLECHAR*>(secret.data),
            secret.len);
        return value_ == nullptr
            ? NAVOP_RDP_RESULT_ALLOCATION_FAILED
            : NAVOP_RDP_RESULT_OK;
    }

    BSTR get() const noexcept {
        return value_;
    }

private:
    void reset() noexcept {
        if (value_ == nullptr) {
            return;
        }
        SecureZeroMemory(
            value_,
            static_cast<size_t>(SysStringByteLen(value_)) +
                sizeof(OLECHAR));
        SysFreeString(value_);
        value_ = nullptr;
    }

    BSTR value_ = nullptr;
};

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
    trace_native_stage("create.begin");
    if (owner == nullptr || out_resources == nullptr) {
        trace_native_stage("create.invalid_output");
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    *out_resources = nullptr;

    const HWND parent = reinterpret_cast<HWND>(parent_hwnd);
    if (parent == nullptr || !IsWindow(parent)) {
        trace_native_win32(
            "create.invalid_parent",
            static_cast<uint32_t>(GetLastError()));
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }
    trace_native_pointer(
        "create.parent_valid",
        reinterpret_cast<uintptr_t>(parent));

    trace_native_stage("create.allocate.before");
    auto resources = std::unique_ptr<NativeRdpActiveXResources>(
        new (std::nothrow) NativeRdpActiveXResources());
    if (!resources) {
        trace_native_stage("create.allocate.failed");
        return record_last_error(owner, NAVOP_RDP_RESULT_ALLOCATION_FAILED);
    }
    trace_native_stage("create.allocate.after");
    resources->state.parent_window = parent;

    trace_native_stage("create.ole_initialize.before");
    const HRESULT ole_result = OleInitialize(nullptr);
    trace_native_hresult(
        "create.ole_initialize.after",
        static_cast<int32_t>(ole_result));
    if (FAILED(ole_result)) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_OLE_INITIALIZE,
            static_cast<int32_t>(ole_result));
    }
    resources->state.ole_initialized = true;

    SetLastError(ERROR_SUCCESS);
    trace_native_stage("create.atl_ax_win_init.before");
    if (!AtlAxWinInit()) {
        const DWORD win32_code = GetLastError();
        trace_native_win32(
            "create.atl_ax_win_init.failed",
            static_cast<uint32_t>(win32_code));
        return record_last_stage_win32(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_ATL_AX_WIN_INIT,
            static_cast<uint32_t>(win32_code));
    }
    trace_native_stage("create.atl_ax_win_init.after");
    resources->state.atl_initialized = true;

    SetLastError(ERROR_SUCCESS);
    trace_native_stage("create.window.before");
    resources->state.child_window = CreateWindowExW(
        0,
        TEXT(ATLAXWIN_CLASS),
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
    trace_native_pointer(
        "create.window.after",
        reinterpret_cast<uintptr_t>(resources->state.child_window));
    if (resources->state.child_window == nullptr) {
        const DWORD win32_code = GetLastError();
        trace_native_win32(
            "create.window.failed",
            static_cast<uint32_t>(win32_code));
        return record_last_stage_win32(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW,
            static_cast<uint32_t>(win32_code));
    }

    trace_native_stage("create.control.before");
    const HRESULT control_result = create_rdp_control(resources->state);
    trace_native_hresult(
        "create.control.after",
        static_cast<int32_t>(control_result));
    if (FAILED(control_result) || resources->state.control == nullptr) {
        if (FAILED(control_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
                static_cast<int32_t>(control_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL);
    }

    trace_native_stage("create.query_client.before");
    const HRESULT query_result = resources->state.control->QueryInterface(
        IID_PPV_ARGS(&resources->state.client));
    trace_native_hresult(
        "create.query_client.after",
        static_cast<int32_t>(query_result));
    if (FAILED(query_result) || resources->state.client == nullptr) {
        if (FAILED(query_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_QUERY_CLIENT,
                static_cast<int32_t>(query_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_QUERY_CLIENT);
    }

    CComPtr<IMsRdpClientNonScriptable2> non_scriptable;
    trace_native_stage("create.query_non_scriptable.before");
    const HRESULT non_scriptable_result =
        resources->state.control->QueryInterface(
            IID_PPV_ARGS(&non_scriptable));
    trace_native_hresult(
        "create.query_non_scriptable.after",
        static_cast<int32_t>(non_scriptable_result));
    if (FAILED(non_scriptable_result) || non_scriptable == nullptr) {
        if (FAILED(non_scriptable_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE,
                static_cast<int32_t>(non_scriptable_result));
        }
        return record_last_stage_error(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_QUERY_NON_SCRIPTABLE);
    }

    trace_native_stage("create.set_ui_parent.before");
    const HRESULT ui_parent_result =
        non_scriptable->put_UIParentWindowHandle(
            reinterpret_cast<wireHWND>(parent));
    trace_native_hresult(
        "create.set_ui_parent.after",
        static_cast<int32_t>(ui_parent_result));
    if (FAILED(ui_parent_result)) {
        return record_last_stage_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_SET_PARENT,
            static_cast<int32_t>(ui_parent_result));
    }

    trace_native_stage("create.event_subscription.before");
    const NavopRdpResult subscription_result = create_event_subscription(
        owner,
        resources->state.control,
        &resources->state.event_subscription);
    trace_native_result(
        "create.event_subscription.after",
        subscription_result);
    if (subscription_result != NAVOP_RDP_RESULT_OK) {
        return record_last_diagnostic(
            owner,
            subscription_result,
            NAVOP_RDP_CREATE_STAGE_EVENT_SUBSCRIPTION,
            owner->last_hresult,
            owner->has_last_hresult,
            owner->last_win32_code,
            owner->has_last_win32_code);
    }

    *out_resources = resources.release();
    trace_native_stage("create.complete");
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
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    const NavopRdpConnectionOptions& options) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected != 0) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_STATE);
    }

    CComBSTR server(
        static_cast<int>(options.host.len),
        reinterpret_cast<LPCOLESTR>(options.host.data));
    if (server.m_str == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_ALLOCATION_FAILED);
    }

    result = resources->state.client->put_Server(server);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }

    CComPtr<IMsRdpClientAdvancedSettings> advanced_settings;
    result = resources->state.control->QueryInterface(
        IID_PPV_ARGS(&advanced_settings));
    if (FAILED(result) || advanced_settings == nullptr) {
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    result = advanced_settings->put_RDPPort(
        static_cast<LONG>(options.port));
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    result = resources->state.client->put_DesktopWidth(
        options.desktop_width);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    result = resources->state.client->put_DesktopHeight(
        options.desktop_height);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    result = resources->state.client->put_ColorDepth(options.color_depth);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    result = resources->state.client->Connect();
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult apply_active_x_credentials(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    NavopRdpBorrowedUtf16 username,
    NavopRdpBorrowedUtf16 domain,
    NavopRdpBorrowedSecret server_password) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected != 0) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_STATE);
    }

    if (username.len > static_cast<uint32_t>((std::numeric_limits<int>::max)()) ||
        domain.len > static_cast<uint32_t>((std::numeric_limits<int>::max)())) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
    }

    if (username.len != UINT32_C(0)) {
        CComBSTR username_bstr(
            static_cast<int>(username.len),
            reinterpret_cast<LPCOLESTR>(username.data));
        if (username_bstr.m_str == nullptr) {
            return record_last_error(
                owner,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }
        result = resources->state.client->put_UserName(username_bstr);
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
    }

    if (domain.len != UINT32_C(0)) {
        CComBSTR domain_bstr(
            static_cast<int>(domain.len),
            reinterpret_cast<LPCOLESTR>(domain.data));
        if (domain_bstr.m_str == nullptr) {
            return record_last_error(
                owner,
                NAVOP_RDP_RESULT_ALLOCATION_FAILED);
        }
        result = resources->state.client->put_Domain(domain_bstr);
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
    }

    if (server_password.len != UINT32_C(0)) {
        SensitiveBstr password_bstr;
        const NavopRdpResult password_result =
            password_bstr.assign(server_password);
        if (password_result != NAVOP_RDP_RESULT_OK) {
            return record_last_error(owner, password_result);
        }

        CComPtr<IMsRdpClientAdvancedSettings> advanced_settings;
        result = resources->state.control->QueryInterface(
            IID_PPV_ARGS(&advanced_settings));
        if (FAILED(result) || advanced_settings == nullptr) {
            if (FAILED(result)) {
                return record_last_hresult(
                    owner,
                    NAVOP_RDP_RESULT_INTERNAL_ERROR,
                    static_cast<int32_t>(result));
            }
            return record_last_error(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR);
        }

        result =
            advanced_settings->put_ClearTextPassword(password_bstr.get());
        if (FAILED(result)) {
            return record_last_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                static_cast<int32_t>(result));
        }
    }

    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult get_active_x_connection_state(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    uint32_t* out_state) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }
    if (out_state == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
    }

    short connected = 0;
    const HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected < 0 || connected > 2) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
    *out_state = static_cast<uint32_t>(connected);
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult get_active_x_extended_disconnect_reason(
    NativeRdpActiveXResources* resources,
    int32_t* out_extended_code) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return resource_result;
    }
    if (out_extended_code == nullptr) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    ExtendedDisconnectReasonCode extended_reason{};
    const HRESULT result =
        resources->state.client->get_ExtendedDisconnectReason(
            &extended_reason);
    if (FAILED(result)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    *out_extended_code = static_cast<int32_t>(extended_reason);
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult request_close_active_x(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources,
    uint32_t* out_status) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }
    if (out_status == nullptr) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INVALID_ARGUMENT);
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected == 0) {
        *out_status = 0;
        return NAVOP_RDP_RESULT_OK;
    }
    if (connected != 1 && connected != 2) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    ControlCloseStatus status = controlCloseCanProceed;
    result = resources->state.client->RequestClose(&status);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (status != controlCloseCanProceed &&
        status != controlCloseWaitForEvents) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }
    *out_status = static_cast<uint32_t>(status);
    return NAVOP_RDP_RESULT_OK;
}

NavopRdpResult disconnect_active_x(
    NativeRdpHost* owner,
    NativeRdpActiveXResources* resources) noexcept {
    const NavopRdpResult resource_result = validate_resources(resources);
    if (resource_result != NAVOP_RDP_RESULT_OK) {
        return record_last_error(owner, resource_result);
    }

    short connected = 0;
    HRESULT result = resources->state.client->get_Connected(&connected);
    if (FAILED(result)) {
        return record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result));
    }
    if (connected == 0) {
        return NAVOP_RDP_RESULT_OK;
    }
    if (connected != 1 && connected != 2) {
        return record_last_error(owner, NAVOP_RDP_RESULT_INTERNAL_ERROR);
    }

    result = resources->state.client->Disconnect();
    return FAILED(result)
        ? record_last_hresult(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            static_cast<int32_t>(result))
        : NAVOP_RDP_RESULT_OK;
}
