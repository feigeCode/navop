#include "host_internal.h"

#include <windows.h>

#include <atlbase.h>
#include <atlhost.h>

#include "mstscax.tlh"

#include <limits>
#include <memory>
#include <new>

namespace {

constexpr wchar_t kNativeHostWindowClassName[] =
    L"Navop.WindowsRdpHost.Container";
constexpr wchar_t kRdpControlClassName[] =
    L"CLSID:{945EE98E-B376-4EC2-B2E5-64C9410F93B7}";

LRESULT CALLBACK native_host_window_procedure(
    HWND window,
    UINT message,
    WPARAM wparam,
    LPARAM lparam) noexcept {
    return DefWindowProcW(window, message, wparam, lparam);
}

DWORD ensure_native_host_window_class(HINSTANCE instance) noexcept {
    WNDCLASSEXW window_class{};
    window_class.cbSize = sizeof(window_class);
    window_class.lpfnWndProc = native_host_window_procedure;
    window_class.hInstance = instance;
    window_class.lpszClassName = kNativeHostWindowClassName;

    SetLastError(ERROR_SUCCESS);
    if (RegisterClassExW(&window_class) != 0) {
        return ERROR_SUCCESS;
    }

    const DWORD error = GetLastError();
    return error == ERROR_CLASS_ALREADY_EXISTS ? ERROR_SUCCESS : error;
}

struct ActiveXCleanup {
    bool ole_initialized = false;
    bool atl_initialized = false;
    HWND parent_window = nullptr;
    HWND host_window = nullptr;
    HWND control_window = nullptr;
    CComPtr<IUnknown> container;
    CComPtr<IUnknown> control;
    CComPtr<IMsRdpClient10> client;
    NativeRdpEventSubscription* event_subscription = nullptr;

    ~ActiveXCleanup() noexcept {
        destroy_event_subscription(event_subscription);
        event_subscription = nullptr;
        if (control_window != nullptr) {
            DestroyWindow(control_window);
        }
        if (host_window != nullptr) {
            DestroyWindow(host_window);
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
        resources->state.host_window == nullptr ||
        resources->state.control_window == nullptr) {
        return NAVOP_RDP_RESULT_UNAVAILABLE;
    }
    if (!IsWindow(resources->state.parent_window) ||
        !IsWindow(resources->state.host_window) ||
        !IsWindow(resources->state.control_window)) {
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

    const HINSTANCE instance = GetModuleHandleW(nullptr);
    trace_native_stage("create.host_class.before");
    const DWORD host_class_error =
        ensure_native_host_window_class(instance);
    trace_native_win32(
        "create.host_class.after",
        static_cast<uint32_t>(host_class_error));
    if (host_class_error != ERROR_SUCCESS) {
        return record_last_stage_win32(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW,
            static_cast<uint32_t>(host_class_error));
    }

    // GPUI's HWND owns Rust window state. Keep ATL and ActiveX window traffic
    // below an inert native child so synchronous parent notifications cannot
    // enter GPUI's Rust window procedure directly.
    SetLastError(ERROR_SUCCESS);
    trace_native_stage("create.host_window.before");
    resources->state.host_window = CreateWindowExW(
        WS_EX_NOPARENTNOTIFY,
        kNativeHostWindowClassName,
        L"",
        WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0,
        0,
        1,
        1,
        parent,
        nullptr,
        instance,
        nullptr);
    trace_native_pointer(
        "create.host_window.after",
        reinterpret_cast<uintptr_t>(resources->state.host_window));
    if (resources->state.host_window == nullptr) {
        const DWORD win32_code = GetLastError();
        trace_native_win32(
            "create.host_window.failed",
            static_cast<uint32_t>(win32_code));
        return record_last_stage_win32(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW,
            static_cast<uint32_t>(win32_code));
    }

    // ATL's hosting helpers require an AtlAxWin host. Passing the custom
    // container HWND directly to AtlAxCreateControlEx caused an access
    // violation on Windows. Create the real ATL host with the RDP CLSID as
    // its window name so the control is installed during WM_CREATE, avoiding
    // both the invalid custom-host call and the empty AtlAxWin intermediate
    // state that previously stalled on some systems.
    SetLastError(ERROR_SUCCESS);
    trace_native_stage("create.control_window.before");
    resources->state.control_window = CreateWindowExW(
        WS_EX_NOPARENTNOTIFY,
        TEXT(ATLAXWIN_CLASS),
        kRdpControlClassName,
        WS_CHILD | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
        0,
        0,
        1,
        1,
        resources->state.host_window,
        nullptr,
        instance,
        nullptr);
    trace_native_pointer(
        "create.control_window.after",
        reinterpret_cast<uintptr_t>(resources->state.control_window));
    if (resources->state.control_window == nullptr) {
        const DWORD win32_code = GetLastError();
        trace_native_win32(
            "create.control_window.failed",
            static_cast<uint32_t>(win32_code));
        return record_last_stage_win32(
            owner,
            NAVOP_RDP_RESULT_INTERNAL_ERROR,
            NAVOP_RDP_CREATE_STAGE_CREATE_WINDOW,
            static_cast<uint32_t>(win32_code));
    }

    trace_native_stage("create.get_control.before");
    const HRESULT control_result = AtlAxGetControl(
        resources->state.control_window,
        &resources->state.control);
    trace_native_hresult(
        "create.get_control.after",
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

    trace_native_stage("create.get_host.before");
    const HRESULT host_result = AtlAxGetHost(
        resources->state.control_window,
        &resources->state.container);
    trace_native_hresult(
        "create.get_host.after",
        static_cast<int32_t>(host_result));
    if (FAILED(host_result) || resources->state.container == nullptr) {
        if (FAILED(host_result)) {
            return record_last_stage_hresult(
                owner,
                NAVOP_RDP_RESULT_INTERNAL_ERROR,
                NAVOP_RDP_CREATE_STAGE_CREATE_CONTROL,
                static_cast<int32_t>(host_result));
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
            resources->state.host_window,
            nullptr,
            bounds.x,
            bounds.y,
            bounds.width,
            bounds.height,
            SWP_NOZORDER | SWP_NOACTIVATE)) {
        return NAVOP_RDP_RESULT_INTERNAL_ERROR;
    }
    if (!SetWindowPos(
            resources->state.control_window,
            nullptr,
            0,
            0,
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
        window_or_descendant_has_focus(resources->state.control_window)) {
        SetFocus(resources->state.parent_window);
    }
    ShowWindow(
        resources->state.host_window,
        visible ? SW_SHOWNA : SW_HIDE);
    ShowWindow(
        resources->state.control_window,
        visible ? SW_SHOWNA : SW_HIDE);

    const LONG_PTR host_style = GetWindowLongPtrW(
        resources->state.host_window,
        GWL_STYLE);
    const LONG_PTR control_style = GetWindowLongPtrW(
        resources->state.control_window,
        GWL_STYLE);
    const bool host_is_visible = (host_style & WS_VISIBLE) != 0;
    const bool control_is_visible = (control_style & WS_VISIBLE) != 0;
    if (host_is_visible != visible || control_is_visible != visible) {
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
             resources->state.host_window,
             GWL_STYLE) &
         WS_VISIBLE) == 0 ||
        (GetWindowLongPtrW(
             resources->state.control_window,
             GWL_STYLE) &
         WS_VISIBLE) == 0) {
        return NAVOP_RDP_RESULT_INVALID_ARGUMENT;
    }

    SetFocus(resources->state.control_window);
    if (!window_or_descendant_has_focus(resources->state.control_window)) {
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
